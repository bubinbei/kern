"""Owned module-management acceptance on a dedicated virtual pair; never opens working COM1."""
import argparse, copy, ctypes, hashlib, json, shutil, subprocess, time, urllib.request
from pathlib import Path
from datetime import datetime, timezone
from im2480_test_peer import Peer, Com
from validate_core import rpc, mutate
from validate_desktop import U, window

ROOT=Path(__file__).resolve().parents[1]

def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--ports',nargs=2,required=True);args=parser.parse_args()
    assert args.ports[0]!=args.ports[1] and not set(args.ports)&{'COM1','COM2'}
    qa=ROOT/'target/qa'/('im2480-modules-'+datetime.now().strftime('%Y%m%d-%H%M%S'));qa.mkdir(parents=True)
    profile=qa/'profile';packages=qa/'plugins';logs=[];core=None;peer=None;checks=[]
    for name,exe in [('im2480','im2480.exe'),('charts','kern-charts.exe'),('processor','kern-processor.exe'),('csv','kern-recorder.exe'),('backup','kern-recorder.exe')]:
        folder=packages/name;folder.mkdir(parents=True);shutil.copyfile(ROOT/'plugins'/name/'plugin.json',folder/'plugin.json');shutil.copyfile(ROOT/'target/release'/exe,folder/exe)
    config=dict(schema=1,source_id='im2480',port=args.ports[0],baud=9600,timeout_ms=150,poll_interval_ms=100,modules=[dict(address=27,slot=0),dict(address=29,slot=3),dict(address=200,slot=6)])
    config_path=packages/'im2480/config.json';config_path.write_text(json.dumps(config),encoding='utf8')
    def wait(fn,seconds=20):
        end=time.monotonic()+seconds
        while time.monotonic()<end:
            try:
                v=fn()
                if v:return v
            except (OSError,ValueError,KeyError,StopIteration):pass
            time.sleep(.05)
        raise AssertionError('Management acceptance timed out')
    def state():return rpc(profile,'status')['status']
    def plugin(id):return next(p for p in rpc(profile,'plugins')['plugins'] if p['id']==id)
    def start(id):assert mutate(profile,'plugin_control',id=id,action='start')['result']=='accepted'
    def management():return json.loads((profile/'im2480/status.json').read_text(encoding='utf8'))
    def api(path,body=None,auth=True):
        ui=json.loads((profile/'charts-endpoint.json').read_text());base,key=ui['url'].split('/#access=')
        headers={'X-Kern-Key':key} if auth else {};data=None if body is None else json.dumps(body).encode()
        with urllib.request.build_opener(urllib.request.ProxyHandler({})).open(urllib.request.Request(base+'/api/'+path,data=data,headers=headers),timeout=8) as response:return json.load(response)
    def command(action,ok=True):
        queued=api('im2480/command',action);assert queued['result']=='im2480_queued',queued
        result=wait(lambda:next((r for r in management()['operations'] if r['id']==queued['id']),None),30)
        assert result['ok']==ok,result
        return result
    try:
        probe=Com(args.ports[0]);probe.close();peer=Peer(args.ports[1])
        log=(qa/'core.log').open('wb');logs.append(log)
        core=subprocess.Popen([str(ROOT/'target/release/kern-core.exe'),'run','--storage','redb','--profile',str(profile),'--plugins',str(packages)],stdout=log,stderr=log,creationflags=0x08000000)
        wait(lambda:(profile/'endpoint.json').exists());shell=wait(lambda:plugin('kern.shell'));hwnd=wait(lambda:window(shell['pid']));U.ShowWindow(hwnd,0)
        calibration=json.loads((ROOT/'plugins/calibration.example.json').read_text(encoding='utf8'))
        assert mutate(profile,'save_calibration',expected_revision=0,map=calibration)['result']=='durable'
        for name in ['processor','csv','backup','im2480','charts']:start(name)
        wait(lambda:state()['raw_batches']>=6);wait(lambda:(profile/'charts-endpoint.json').exists());wait(lambda:management()['report']['phase']=='polling')
        pid=plugin('im2480')['pid'];checks.append('managed package acquisition with owned COM; control status and native port enumeration')
        with (qa/'browser-console.log').open('wb') as log:
            result=subprocess.run(['node','tools/validate_im2480_modules_browser.cjs',str(profile),str(qa)],stdout=log,stderr=log,cwd=ROOT,timeout=120)
        assert result.returncode==0,(qa/'browser-console.log').read_text(encoding='utf8')
        assert plugin('im2480')['pid']==pid
        checks.append('browser wiring/disable/re-enable/fanout and settings apply without process restart')
        before=state()['raw_batches'];command(dict(action='scan',**{'from':0,'to':3}));wait(lambda:not management()['scan']['active']);scan=management()['scan'];assert scan['checked']==3 and [v['address'] for v in scan['found']]==[1,2,3]
        assert state()['raw_batches']>before;assert not any(r[0]==0 for r in peer.requests);checks.append('scan skips broadcast zero; interleaved acquisition; no second COM owner')
        before=state()['raw_batches'];command(dict(action='scan',**{'from':1,'to':100}));wait(lambda:not management()['scan']['active'],60)
        assert len(management()['scan']['found'])==100 and management()['scan']['checked']==100
        assert (profile/'im2480/status.json').stat().st_size<131072 and api('im2480/status')['state'] is not None
        assert state()['raw_batches']>before;checks.append('full 1..100 scan remains within status bound while acquisition advances')
        peer.mode(29,'crc');wait(lambda:next(d for d in management()['report']['devices'] if d['address']==29)['errors']>0)
        peer.mode(29,'good');wait(lambda:next(d for d in management()['report']['devices'] if d['address']==29)['online'])
        assert next(d for d in management()['report']['devices'] if d['address']==29)['last_error']
        command(dict(action='reset_stats',address=29));d=next(d for d in management()['report']['devices'] if d['address']==29);assert d['errors']==0 and d['online'] and d['last_error'] is None
        checks.append('per-address errors persist after recovery; reset preserves availability')
        active=management()['config'];candidate=copy.deepcopy(active);candidate['port']=args.ports[1];original=config_path.read_bytes();before=state()['raw_batches']
        command(dict(action='apply',expected=active,config=candidate),ok=False);wait(lambda:state()['raw_batches']>before)
        assert config_path.read_bytes()==original and management()['config']==active and plugin('im2480')['pid']==pid
        checks.append('busy destination COM rolls back to working configuration with same source process')
        external=copy.deepcopy(active);external['poll_interval_ms']+=1;config_path.write_text(json.dumps(external),encoding='utf8')
        command(dict(action='apply',expected=active,config=active),ok=False);assert json.loads(config_path.read_text())==external
        config_path.write_bytes(original);checks.append('external configuration edit is never overwritten by stale UI')
        candidate=copy.deepcopy(active);candidate['poll_interval_ms']+=10
        command(dict(action='apply',expected=active,config=candidate));wait(lambda:management()['config']==candidate)
        assert plugin('im2480')['pid']==pid;assert list((profile/'im2480/config-history').glob('*.json'))
        checks.append('validated configuration change and preserved before/after history')
        try:api('im2480/command',dict(action='stop_scan'),auth=False);raise AssertionError('Unauthenticated source control accepted')
        except urllib.error.HTTPError as error:assert error.code==403
        assert all(function==3 for _,function,_,_ in peer.requests);checks.append('management requires presentation key; only FC03 observed')
        report=dict(passed=True,date=datetime.now(timezone.utc).isoformat(),checks=checks,browser=json.loads((qa/'browser-report.json').read_text()),requests=len(peer.requests),source_pid_unchanged=True,binaries={name:hashlib.sha256((ROOT/'target/release'/name).read_bytes()).hexdigest() for name in ['kern-core.exe','im2480.exe','kern-charts.exe','kern-processor.exe']})
        (qa/'report.json').write_text(json.dumps(report,indent=2)+'\n',encoding='utf8');print(json.dumps(dict(passed=True,groups=len(checks),report=str(qa/'report.json'))))
    finally:
        if core and core.poll() is None:
            try:mutate(profile,'stop');core.wait(timeout=12)
            except (OSError,subprocess.TimeoutExpired):core.kill();core.wait(timeout=5)
        if peer:peer.close()
        for log in logs:log.close()

if __name__=='__main__':main()
