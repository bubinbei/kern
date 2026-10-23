"""Independent numeric/file/process oracle for the first redb/calibration/uPlot slice."""
from __future__ import annotations
import argparse,csv,hashlib,io,json,os,subprocess,time,urllib.request,zlib
from pathlib import Path
from datetime import datetime,timezone,timedelta
from validate_core import rpc,mutate

ROOT=Path(__file__).resolve().parents[1]
BIN=ROOT/'target/release'

def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--config',type=Path);args=parser.parse_args()
    qa=ROOT/'target/qa'/('prototype-'+datetime.now(timezone(timedelta(hours=4))).strftime('%Y%m%d-%H%M%S'))
    qa.mkdir(parents=True);profile=qa/'profile';profile.mkdir();owned=[];tests=[]
    def spawn(name,*arguments):
        log=open(qa/(name+'-'+str(len(owned))+'.log'),'wb')
        p=subprocess.Popen([str(BIN/(name+'.exe')),*map(str,arguments)],cwd=ROOT,stdin=subprocess.PIPE,stdout=log,stderr=log,creationflags=0x08000000)
        owned.append((p,log));return p
    def wait(fn,seconds=15):
        end=time.monotonic()+seconds
        while time.monotonic()<end:
            try:
                value=fn()
                if value:return value
            except (OSError,ValueError,KeyError):pass
            time.sleep(.05)
        raise AssertionError('Timed out waiting for '+repr(fn))
    def stop(p):
        if p.poll() is None:p.stdin.write(b'stop\n');p.stdin.flush();p.wait(timeout=8)
        assert p.returncode==0,p.returncode
    def core():
        p=spawn('kern-core','serve','--storage','redb','--profile',profile)
        wait(lambda:rpc(profile,'status').get('result')=='status');return p
    def history():
        events=[];cursor=0
        while True:
            r=rpc(profile,'history',after=cursor);events+=r['events'];cursor=r['next']
            if cursor>=r['through']:return events
    def input_batch(session,sequence,value):
        return dict(source='test',session=session,sequence=sequence,measured_unix_ms=int(time.time()*1000),samples=[dict(channel=0,value=dict(type='unsigned',value=value),quality='good')])
    def ingest(batch):
        r=mutate(profile,'ingest',batch=batch);assert r['result']=='durable',r;return r['event_sequence']
    def output_at(seq):return wait(lambda:rpc(profile,'calibration')['position']>=seq)
    def check_backup(path):
        events=[];pending=None;payload=b'';last=0
        for line in path.read_bytes().splitlines(keepends=True):
            if not line.endswith(b'\n'):break
            v=json.loads(line)
            if '_commit' in v:
                assert pending and v['_commit']==pending['sequence'] and v['crc32']==zlib.crc32(payload)
                assert v['_commit']==last+1
                last=v['_commit'];events.append(pending);pending=None;payload=b''
            else:assert pending is None;pending=v;payload=line
        return events
    def check_csv(path):
        lines=path.read_bytes().splitlines(keepends=True);payload=b'';cursor=0;rows=[]
        for line in lines[1:]:
            if line.startswith(b'#commit,'):
                _,seq,crc=line.strip().split(b',');assert int(seq)>cursor and zlib.crc32(payload)==int(crc)
                rows.extend(list(csv.reader(io.StringIO(payload.decode('utf8')))));cursor=int(seq);payload=b''
            else:payload+=line
        return cursor,rows
    try:
        c=core()
        m=json.loads((ROOT/'plugins/calibration.example.json').read_text(encoding='utf8'))
        m['parameters']=m['parameters'][:1];p=m['parameters'][0];p.update(source='test',name='Reference weight',points=[dict(raw=1000,value=0),dict(raw=8800,value=1)],min=0,max=2)
        assert mutate(profile,'save_calibration',expected_revision=0,map=m)['result']=='durable'
        processor=spawn('kern-processor','--profile',profile)
        wait(lambda:rpc(profile,'calibration')['active_revision']==1)
        csv_process=spawn('kern-recorder','--profile',profile,'--mode','csv')
        backup=spawn('kern-recorder','--profile',profile,'--mode','backup')
        s=rpc(profile,'status')['status']
        source=mutate(profile,'open_source',id='test',resource='SYNTHETIC',channels=[dict(id=0,name='Raw',value_type='unsigned')],expected_catalog_revision=s['catalog_revision']);session=source['session']
        batches=[]
        for i,value in enumerate([1000,4900,8800],1):
            b=input_batch(session,i,value);seq=ingest(b);batches.append((seq,b));time.sleep(.03)
        output_at(seq)
        events=history();results=[e['data']['batch'] for e in events if e['data']['event']=='parameters_stored']
        assert [v['values'][0]['value'] for v in results]==[0,.5,1],results
        before=rpc(profile,'status')['status']['raw_batches'];assert mutate(profile,'ingest',batch=batches[-1][1])['duplicate'];assert rpc(profile,'status')['status']['raw_batches']==before
        tests.append('raw -> historical map -> independently verified 0/0.5/1; duplicate input')
        stop(processor)
        changed=json.loads(json.dumps(m));changed['parameters'][0]['points'][1]['value']=2
        assert mutate(profile,'save_calibration',expected_revision=1,map=changed)['result']=='durable'
        old_seq=ingest(input_batch(session,4,4900));assert rpc(profile,'calibration')['position']<old_seq
        processor=spawn('kern-processor','--profile',profile);wait(lambda:rpc(profile,'calibration')['active_revision']==2);output_at(old_seq)
        new_seq=ingest(input_batch(session,5,4900));output_at(new_seq)
        results=[e['data']['batch'] for e in history() if e['data']['event']=='parameters_stored']
        assert next(b for b in results if b['raw_event']==old_seq)['values'][0]['value']==.5
        assert next(b for b in results if b['raw_event']==new_seq)['values'][0]['value']==1
        cal=rpc(profile,'calibration')
        bad=mutate(profile,'publish_parameters',session=cal['generation'],raw_event=new_seq,revision=2,values=[dict(id='weight',value=999,quality='good')]);assert bad['result']=='error'
        assert mutate(profile,'save_calibration',expected_revision=1,map=m)['code']=='revision_conflict'
        tests.append('backlog uses historical revision; activation boundary; stale map/tampered result rejected')
        # Renderer must not block ingestion; security checks apply to the local web bridge.
        charts=spawn('kern-charts','--profile',profile);wait(lambda:(profile/'charts-endpoint.json').exists())
        ui=json.loads((profile/'charts-endpoint.json').read_text());base,key=ui['url'].split('/#access=');base+='/'
        def http(path,body=None,auth=True,origin=None):
            headers={'X-Kern-Key':key} if auth else {}
            if origin:headers['Origin']=origin
            data=json.dumps(body).encode() if body is not None else None
            for attempt in range(5):
                try:return urllib.request.build_opener(urllib.request.ProxyHandler({})).open(urllib.request.Request(base+path,data=data,headers=headers),timeout=8)
                except urllib.error.HTTPError:raise
                except OSError:
                    if attempt==4:raise
                    time.sleep(.1)
        for auth,origin in [(False,None),(True,'https://untrusted.invalid')]:
            try:http('api/status',auth=auth,origin=origin);raise AssertionError('Unauthorized bridge access')
            except urllib.error.HTTPError as error:assert error.code==403
        plot=json.load(http('api/plot',dict(from_ms=int(time.time()*1000)-300000,to_ms=int(time.time()*1000)+1000,pixels=200,parameters=['weight'])))
        assert any(p[1]==1 for p in plot['series'][0]['points'])
        stop(charts);last=ingest(input_batch(session,6,8800));output_at(last)
        tests.append('uPlot bridge history; no key/cross-origin rejection; screen stop leaves ingestion')
        stop(processor);stop(csv_process);stop(backup)
        # Both consumers resume from verified output positions, not volatile core state.
        csv_path=profile/'exports/parameters.csv';backup_path=profile/'backup/events.jsonl'
        csv_process=spawn('kern-recorder','--profile',profile,'--mode','csv');backup=spawn('kern-recorder','--profile',profile,'--mode','backup')
        time.sleep(1.2);stop(csv_process);stop(backup)
        csv_cursor,rows=check_csv(csv_path);records=check_backup(backup_path)
        assert len({(r[0],r[7]) for r in rows})==len(rows)
        assert len(rows)==6,rows
        tests.append('independent CSV/backup CRC readers; consumer restart without duplicate rows')
        # Confirmed prefix survives forced termination and is read from redb on next boot.
        before=rpc(profile,'status')['status']['raw_batches'];c.kill();c.wait(timeout=5)
        c=core();assert rpc(profile,'status')['status']['raw_batches']==before
        recovered=rpc(profile,'snapshot')['parameters'];assert recovered[0]['quality']=='recovered',recovered
        tests.append('forced core termination preserves confirmed prefix; recovered values are not fresh')
        live_report=None
        if args.config:
            defaults=json.loads((ROOT/'plugins/calibration.example.json').read_text(encoding='utf8'))
            config=json.loads(args.config.read_text(encoding='utf8'))
            for definition in defaults['parameters']:definition['source']=config['source_id']
            cal=rpc(profile,'calibration');assert mutate(profile,'save_calibration',expected_revision=cal['revision'],map=defaults)['result']=='durable'
            processor=spawn('kern-processor','--profile',profile);wait(lambda:rpc(profile,'calibration')['active_revision']==3)
            csv_process=spawn('kern-recorder','--profile',profile,'--mode','csv');backup=spawn('kern-recorder','--profile',profile,'--mode','backup')
            existing=[r['data'] for r in history() if r['data']['event']=='batch_stored'];start_count=len(existing)
            device=spawn('im2480','run','--profile',profile,'--config',args.config.resolve(),'--cycles','2');assert device.wait(timeout=90)==0
            raw=[r for r in history() if r['data']['event']=='batch_stored' and r['data']['batch']['source']==config['source_id']]
            output_at(raw[-1]['sequence']);assert len(raw)==len(config['modules'])*2
            all_events=history();all_results={r['data']['batch']['raw_event']:r['data']['batch'] for r in all_events if r['data']['event']=='parameters_stored'}
            for record in raw:
                batch=record['data']['batch'];result=all_results[record['sequence']]
                for value in result['values']:
                    definition=next(p for p in defaults['parameters'] if p['id']==value['id']);sample=next(p for p in batch['samples'] if p['channel']==definition['channel'])
                    x=sample['value']['value'];a,b=definition['points'];expected=a['value']+(x-a['raw'])/(b['raw']-a['raw'])*(b['value']-a['value'])
                    if a['raw']<=x<=b['raw']:assert abs(expected-value['value'])<1e-9,(expected,value)
                    else:assert value['value'] is None
            live_report=dict(modules=len(config['modules']),batches=len(raw),samples=sum(len(r['data']['batch']['samples']) for r in raw),parameters=len(rpc(profile,'snapshot')['parameters']))
            charts=spawn('kern-charts','--profile',profile);wait(lambda:(profile/'charts-endpoint.json').exists())
            browser=subprocess.run(['node','tools/validate_prototype_browser.cjs',str(profile),str(qa)],cwd=ROOT,capture_output=True,text=True,encoding="utf8",timeout=90)
            (qa/'browser-console.txt').write_text(browser.stdout+browser.stderr,encoding='utf8');assert browser.returncode==0,(browser.stdout,browser.stderr)
            tests.append('existing emulator COM acquisition, 30 channels and independent calibration oracle; browser interactions')
            stop(charts);stop(processor);time.sleep(1);stop(csv_process);stop(backup)
        records=check_backup(backup_path);csv_cursor,rows=check_csv(csv_path)
        assert len({(r[0],r[7]) for r in rows})==len(rows)
        mutate(profile,'stop');assert c.wait(timeout=8)==0
        dump=subprocess.run([str(BIN/'kern-core.exe'),'dump','--profile',str(profile)],capture_output=True,text=True,encoding="utf8",check=True,timeout=30)
        dumped=[json.loads(line) for line in dump.stdout.splitlines()]
        assert len(dumped)>=len(records)
        assert dumped[:len(records)]==records
        (qa/'ledger.jsonl').write_text(dump.stdout,encoding='utf8')
        restored=qa/'restored'
        restored_output=subprocess.run([str(BIN/'kern-core.exe'),'restore','--profile',str(restored),'--input',str(backup_path)],capture_output=True,text=True,encoding="utf8",check=True,timeout=30)
        restored_dump=subprocess.run([str(BIN/'kern-core.exe'),'dump','--profile',str(restored)],capture_output=True,text=True,encoding="utf8",check=True,timeout=30)
        assert [json.loads(line) for line in restored_dump.stdout.splitlines()]==records
        tests.append('restore into empty profile solely from backup; exact independent event comparison')
        report=dict(schema=1,passed=True,tests=tests,live=live_report,events=len(dumped),csv_rows=len(rows),backup_events=len(records),database_bytes=(profile/'measurements.redb').stat().st_size,
                    not_checked=['Physical modules','Windows10','Low-end hardware','GPU','Power loss','24-72h endurance'])
        (qa/'report.json').write_text(json.dumps(report,ensure_ascii=False,indent=2)+'\n',encoding='utf8');print(json.dumps(dict(report=str(qa/'report.json'),**report),ensure_ascii=False))
    finally:
        for p,log in reversed(owned):
            if p.poll() is None:
                try:p.stdin.write(b'stop\n');p.stdin.flush();p.wait(timeout=3)
                except (OSError,subprocess.TimeoutExpired):p.kill();p.wait(timeout=3)
            log.close()

if __name__=='__main__':main()
