"""Explicit virtual-pair acceptance of im2480 -> core -> journal, stdlib only."""
from __future__ import annotations
import argparse
from datetime import datetime,timezone,timedelta
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import time
from im2480_test_peer import Peer,Com
from validate_core import rpc,mutate,journal

ROOT=Path(__file__).resolve().parents[1]
CORE=ROOT/'target/release/kern-core.exe';PLUGIN=ROOT/'target/release/im2480.exe'

def until(check,seconds=12):
    end=time.monotonic()+seconds
    while time.monotonic()<end:
        value=check()
        if value:return value
        time.sleep(.03)
    raise AssertionError('Deadline expired')

def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--ports',nargs=2,required=True,metavar=('CLIENT','PEER'))
    args=parser.parse_args();assert os.name=='nt' and args.ports[0]!=args.ports[1]
    qa=ROOT/'target/qa'/('im2480-'+datetime.now(timezone(timedelta(hours=4))).strftime('%Y%m%d-%H%M%S')+'-'+str(os.getpid()))
    package=qa/'plugins/im2480';package.mkdir(parents=True);profile=qa/'profile'
    shutil.copyfile(ROOT/'plugins/im2480/plugin.json',package/'plugin.json');shutil.copyfile(PLUGIN,package/'im2480.exe')
    config={'schema':1,'source_id':'im2480','port':args.ports[0],'baud':9600,'timeout_ms':200,'poll_interval_ms':100,
            'modules':[{'address':27,'slot':0},{'address':29,'slot':3},{'address':200,'slot':6}]}
    def save_config(): (package/'config.json').write_text(json.dumps(config),encoding='utf-8')
    save_config();peer=Peer(args.ports[1]);core=None;demo=None;log=open(qa/'core.log','wb');checks=[]
    def state():return rpc(profile,'status')['status']
    def source():return next((s for s in rpc(profile,'snapshot')['sources'] if s['id']=='im2480'),None)
    def values():return {s['id']:s for s in source()['channels']}
    def view():return next(p for p in rpc(profile,'plugins')['plugins'] if p['id']=='im2480')
    def start():assert rpc(profile,'plugin_control',id='im2480',action='start')['result']=='accepted'
    def stop():
        assert rpc(profile,'plugin_control',id='im2480',action='stop')['result']=='accepted'
        until(lambda:view()['state']=='stopped');until(lambda:not source()['online'])
    def good():
        s=source()
        return s and len(s['channels'])==len(config['modules'])*6 and all(c['quality']=='good' for c in s['channels'])
    try:
        # Verify both explicitly selected ports are virtual and available, without taking another owner's port.
        probe=Com(args.ports[0]);probe.close()
        core=subprocess.Popen([str(CORE),'run','--profile',str(profile),'--plugins',str(qa/'plugins')],cwd=ROOT,stdout=log,stderr=log,creationflags=0x08000000)
        until(lambda:(profile/'endpoint.json').exists());until(lambda:rpc(profile,'status')['status']['shell_connected'])
        demo=subprocess.Popen([str(ROOT/'target/release/kern-demo.exe'),'--profile',str(profile),'--id','other','--channels','1','--count','0','--period-ms','40'],stdin=subprocess.PIPE,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL,creationflags=0x08000000)
        start();until(good);checks.append('package_launch_and_real_virtual_com')
        s=source();initial_slot=s['slot'];v=values()
        for m in config['modules']:
            base=m['slot']*6;a=m['address']
            assert [v[base+i]['value']['value'] for i in range(3)]==[a*10+1,a*10+2,a*10+3]
            assert v[base+3]['value']=={'type':'signed','value':-100000-a*17}
            assert v[base+4]['value']=={'type':'float','value':a+.25};assert v[base+5]['value']['value']==a+.5
        checks.append('independent_values_types_endianness_and_address_200')
        # OS and core registry both reject competing ownership of the same port.
        try:conflict=Com(args.ports[0])
        except OSError:pass
        else:conflict.close();raise AssertionError('Serial port was not exclusive')
        reply=mutate(profile,'open_source',id='competing',resource=args.ports[0],expected_catalog_revision=state()['catalog_revision'],channels=[{'id':0,'name':'test','value_type':'unsigned'}])
        assert reply['code']=='resource_busy';checks.append('exclusive_os_and_core_port_ownership')
        for mode in ['silent','crc','address','exception','order','short']:
            healthy_before=values()[36]['sequence']
            peer.mode(29,mode)
            until(lambda:all(values()[i]['quality']=='missing' for i in range(18,24)))
            until(lambda:values()[36]['sequence']>healthy_before)
            assert all(values()[i]['quality']=='good' for i in [0,1,2,36,37,38])
            before=state()['raw_batches'];time.sleep(.25);assert state()['raw_batches']>before
            checks.append('isolated_'+mode)
            peer.mode(29,'good');until(good)
        peer.mode(29,'nan');until(lambda:values()[22]['quality']=='missing' and values()[23]['quality']=='good')
        assert all(values()[i]['quality']=='good' for i in range(18,22));checks.append('nonfinite_frequency_does_not_poison_other_channels')
        peer.mode(29,'fragment');until(good);checks.append('fragmented_serial_response')
        peer.mode(29,'good');stop();checks.append('cooperative_stop_closes_source')
        probe=Com(args.ports[0]);probe.close();checks.append('normal_stop_releases_com')
        config['modules'].append({'address':77,'slot':10});save_config();start();until(good)
        assert source()['slot']==initial_slot and [m['slot']*6 for m in config['modules']]==[0,18,36,60]
        assert set(values())=={m['slot']*6+i for m in config['modules'] for i in range(6)};checks.append('module_growth_preserves_reserved_channel_ids')
        stop()
        probe=Com(args.ports[0]);start()
        until(lambda:view()['state']=='stopped')
        status_file=profile/'plugin-status/im2480.json'
        until(lambda:json.loads(status_file.read_text())['phase']=='error')
        assert not source()['online'];probe.close();checks.append('busy_port_fails_without_fake_live_source')
        start();until(good)
        for m in config['modules']:peer.mode(m['address'],'silent')
        time.sleep(.05);t=time.monotonic();stop();assert time.monotonic()-t<2;checks.append('stop_interrupts_silent_reads_within_supervisor_budget')
        demo.stdin.write(b'stop\n');demo.stdin.flush();assert demo.wait(timeout=6)==0
        final=state();assert mutate(profile,'stop')['result']=='durable';assert core.wait(timeout=8)==0
        events,tail=journal(profile);assert tail is None
        batches=[e['batch'] for e in events if e['event']=='batch_stored' and e['batch']['source']=='im2480']
        assert batches and all(len(b['samples'])==6 for b in batches)
        for batch in batches:
            for sample in batch['samples']:
                assert sample['quality']=='missing' if sample['value'] is None else sample['quality']=='good'
        checks.append('independent_crc_reader_confirms_typed_plugin_journal')
        source_hash=hashlib.sha256()
        paths=sorted([ROOT/'Cargo.toml',ROOT/'Cargo.lock',*(ROOT/'crates').rglob('*.toml'),*(ROOT/'crates').rglob('*.rs')],key=lambda p:p.relative_to(ROOT).as_posix())
        for path in paths:source_hash.update(path.relative_to(ROOT).as_posix().encode()+b'\0'+path.read_bytes().replace(b'\r\n',b'\n'))
        report={'schema':1,'rust_source_sha256':source_hash.hexdigest(),'platform':'Windows11, dedicated com0com pair, independent Python peer','checks':checks,
                'plugin_batches':len(batches),'plugin_samples':sum(len(b['samples']) for b in batches),'total_batches':final['raw_batches'],
                'plugin_binary':{'bytes':PLUGIN.stat().st_size,'sha256':hashlib.sha256(PLUGIN.read_bytes()).hexdigest()},
                'not_checked':['Physical IM2480/USB-RS485','Windows10 and weak hardware','24-72 hour run','Data order other than 0009=0']}
        (qa/'report.json').write_text(json.dumps(report,indent=2)+'\n',encoding='utf-8');print(json.dumps({'checks':len(checks),'report':str((qa/'report.json').relative_to(ROOT))}))
    finally:
        if demo and demo.poll() is None:
            try:demo.stdin.write(b'stop\n');demo.stdin.flush();demo.wait(timeout=6)
            except Exception:demo.kill();demo.wait(timeout=3)
        if core and core.poll() is None:
            try:mutate(profile,'stop');core.wait(timeout=8)
            except Exception:core.kill();core.wait(timeout=3)
        peer.close();log.close()

if __name__=='__main__':main()
