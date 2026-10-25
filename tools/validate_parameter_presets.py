"""Owned synthetic graph/preset/browser acceptance. Does not open any COM port."""
import copy, csv as csv_module, io, hashlib, json, subprocess, threading, time, urllib.request, zlib
from pathlib import Path
from datetime import datetime, timezone
from validate_core import rpc, mutate

ROOT=Path(__file__).resolve().parents[1]

def main():
    qa=ROOT/'target/qa'/('parameter-presets-'+datetime.now().strftime('%Y%m%d-%H%M%S'));profile=qa/'profile';profile.mkdir(parents=True)
    children=[];logs=[];errors=[];stopping=threading.Event();feeder=None;checks=[]
    def spawn(name,*args):
        log=(qa/(name+'-'+str(len(children))+'.log')).open('wb');logs.append(log)
        p=subprocess.Popen([str(ROOT/'target/release'/(name+'.exe')),*map(str,args)],stdin=subprocess.PIPE,stdout=log,stderr=log,creationflags=0x08000000);children.append(p);return p
    def stop(p):
        if p.poll() is None:p.stdin.write(b'stop\n');p.stdin.flush();p.wait(timeout=10)
        assert p.returncode==0,p.returncode
    def wait(fn,seconds=20):
        end=time.monotonic()+seconds
        while time.monotonic()<end:
            try:
                value=fn()
                if value:return value
            except (OSError,ValueError,KeyError,StopIteration):pass
            time.sleep(.04)
        raise AssertionError('Preset fixture did not progress')
    def history(after=0):
        out=[]
        while True:
            r=rpc(profile,'history',after=after);out+=r['events'];after=r['next']
            if after>=r['through']:return out
    def values():return {p['id']:p for p in rpc(profile,'snapshot')['parameters']}
    def save(m):
        revision=rpc(profile,'calibration')['revision'];r=mutate(profile,'save_calibration',expected_revision=revision,map=m);assert r['result']=='durable',r
        wait(lambda:rpc(profile,'calibration')['active_revision']==revision+1);return revision+1
    try:
        core=spawn('kern-core','serve','--storage','redb','--profile',profile);wait(lambda:rpc(profile,'status')['result']=='status')
        original=json.loads((ROOT/'plugins/calibration.example.json').read_text(encoding='utf8'))
        for p in original['parameters']:p.update(source='test',points=[dict(raw=0,value=0),dict(raw=100000,value=100000)])
        assert mutate(profile,'save_calibration',expected_revision=0,map=original)['result']=='durable'
        worker=spawn('kern-processor','--profile',profile);wait(lambda:rpc(profile,'calibration')['active_revision']==1)
        (profile/'backup-settings.json').write_text(json.dumps(dict(revision=0,profile_id='a'*32,directory='',compression='none',layout='legacy')))
        backup=spawn('kern-recorder','--profile',profile,'--mode','backup');csv=spawn('kern-recorder','--profile',profile,'--mode','csv')
        opened=mutate(profile,'open_source',id='test',resource='SYNTHETIC-PRESETS',channels=[dict(id=i,name='Synthetic '+str(i),value_type='float') for i in range(6)],expected_catalog_revision=rpc(profile,'status')['status']['catalog_revision']);session=opened['session']
        start=time.monotonic()
        def feed():
            try:
                seq=0
                while not stopping.is_set():
                    seq+=1;items=[80.,3.,1.,time.monotonic()-start,2.,60.]
                    r=mutate(profile,'ingest',batch=dict(source='test',session=session,sequence=seq,measured_unix_ms=int(time.time()*1000),samples=[dict(channel=i,value=dict(type='float',value=v),quality='good') for i,v in enumerate(items)]));assert r['result']=='durable',r;stopping.wait(.1)
            except Exception as e:errors.append(str(e))
        feeder=threading.Thread(target=feed);feeder.start();wait(lambda:len(values())==6)
        charts=spawn('kern-charts','--profile',profile);wait(lambda:(profile/'charts-endpoint.json').exists())
        with (qa/'browser.log').open('wb') as log:r=subprocess.run(['node','tools/validate_parameter_presets_browser.cjs',str(profile),str(qa)],cwd=ROOT,stdout=log,stderr=log,timeout=150)
        assert r.returncode==0,(qa/'browser.log').read_text(encoding='utf8');checks.append('browser preset/catalog/provider workflows on continuously ingesting native pipeline')
        catalog=json.loads((ROOT/'crates/processor/parameter-library.json').read_text(encoding='utf8'));definitions={e['parameter']['id']:e['parameter'] for e in catalog['entries']}
        ids=['weight','level','hole_depth','free_weight','load','pump_strokes_1','pump_flow_1','tank_volume_1','drilling_rate']
        ps=[copy.deepcopy(definitions[id]) for id in ids]
        for p in ps:
            p['enabled']=True
            if not p.get('provider'):p.update(source='test',channel={'weight':0,'level':2,'pump_strokes_1':5}[p['id']],points=[dict(raw=0,value=0),dict(raw=100000,value=100000)])
        def find(id):return next(p for p in ps if p['id']==id)
        find('hole_depth').pop('provider');find('hole_depth').update(source='test',channel=3,points=[dict(raw=0,value=0),dict(raw=100000,value=100000)])
        find('free_weight')['provider']['value']=100
        off=copy.deepcopy(definitions['hole_depth']);off.update(id='off_bottom',name='Synthetic off-bottom',provider=dict(kind='manual',value=0));ps.append(off)
        find('pump_flow_1')['provider']['operation'].update(liters_per_stroke=2,efficiency=.9)
        find('tank_volume_1')['provider']['operation']['points']=[dict(raw=0,value=0),dict(raw=2,value=20)]
        find('drilling_rate')['provider']['operation']['window_seconds']=1
        graph=dict(algorithm='graph-v1',parameters=ps);revision=save(graph)
        wait(lambda:values().get('load',{}).get('value')==20)
        assert abs(values()['pump_flow_1']['value']-1.8)<1e-9 and abs(values()['tank_volume_1']['value']-10)<1e-9
        wait(lambda:values().get('drilling_rate',{}).get('value') is not None)
        rate=values()['drilling_rate']['value'];assert 3400<rate<3800,rate
        checks.append('independent 100-80=20 t, 60*2*.9/60=1.8 l/s, tank interpolation=10 m3, moving depth approximately 3600 m/h')
        before=rpc(profile,'calibration')['position'];stop(worker);time.sleep(1.3);assert rpc(profile,'status')['status']['last_raw_event']>before
        worker=spawn('kern-processor','--profile',profile);wait(lambda:rpc(profile,'calibration')['position']>before+2)
        results=[r['data']['batch'] for r in history(before) if r['data']['event']=='parameters_stored' and r['data']['batch']['revision']==revision]
        first=next(v for v in results[0]['values'] if v['id']=='drilling_rate');assert first['value'] is not None,first
        checks.append('interpreter restart restores committed window and catches ongoing raw backlog without new warmup')
        old=rpc(profile,'calibration');bad=copy.deepcopy(graph);bad['parameters'].append(copy.deepcopy(ps[0]));assert mutate(profile,'save_calibration',expected_revision=old['revision'],map=bad)['result']=='error'
        assert rpc(profile,'calibration')['revision']==old['revision'];checks.append('invalid map rejected atomically without replacing active revision')
        ui=json.loads((profile/'charts-endpoint.json').read_text());base,key=ui['url'].split('/#access=')
        try:urllib.request.urlopen(base+'/api/parameter-library',timeout=5);raise AssertionError('Unauthenticated catalog allowed')
        except urllib.error.HTTPError as e:assert e.code==403
        checks.append('preset/catalog API requires presentation key')
        stop(charts);charts=spawn('kern-charts','--profile',profile);wait(lambda:(profile/'charts-endpoint.json').exists())
        ui=json.loads((profile/'charts-endpoint.json').read_text());base,key=ui['url'].split('/#access=')
        request=urllib.request.Request(base+'/api/parameter-library',headers={'X-Kern-Key':key})
        with urllib.request.urlopen(request,timeout=5) as response:assert len(json.load(response)['custom'])==3
        checks.append('custom preset versions survive presentation-host restart while raw intake continues')
        stopping.set();feeder.join(timeout=10);assert not errors,errors
        wait(lambda:rpc(profile,'status')['status']['processor_position']==rpc(profile,'status')['status']['last_raw_event'])
        stop(worker);boundary=rpc(profile,'status')['status']['event_sequence']
        backup_path=profile/'backup/events.jsonl'
        def backup_cursor():
            lines=backup_path.read_text(encoding='utf8').splitlines()
            return max((json.loads(line).get('_commit',0) for line in lines if line.startswith('{"_commit":')),default=0)
        # Windows exclusive output lock forbids reading while the recorder owns the file.
        for attempt in range(5):
            time.sleep(.6);stop(backup)
            if backup_cursor()>=boundary:break
            backup=spawn('kern-recorder','--profile',profile,'--mode','backup')
        else:raise AssertionError(('backup drain',boundary,backup_cursor()))
        stop(csv);stop(charts)
        rows=[];payload=b''
        for line in (profile/'exports/parameters.csv').read_bytes().splitlines(keepends=True)[1:]:
            if line.startswith(b'#commit,'):
                assert zlib.crc32(payload)==int(line.strip().split(b',')[2]);rows.extend(csv_module.reader(io.StringIO(payload.decode('utf8'))));payload=b''
            else:payload+=line
        for parameter,expected in [('load',20),('pump_flow_1',1.8),('tank_volume_1',10)]:
            assert any(r[6]==str(revision) and r[7]==parameter and r[8] and abs(float(r[8])-expected)<1e-9 for r in rows),parameter
        checks.append('independent CSV CRC reader confirms calculated load, flow and tank-volume rows')
        context=rpc(profile,'processing_context',position=rpc(profile,'calibration')['position'])['context']
        mutate(profile,'stop');core.wait(timeout=10)
        # Both source database and independent CRC-checked backup reconstruct the same window.
        def dump(p):return [json.loads(line) for line in subprocess.check_output([str(ROOT/'target/release/kern-core.exe'),'dump','--profile',str(p)],encoding='utf8').splitlines()]
        records=[];pending=None;payload=b''
        for line in backup_path.read_bytes().splitlines(keepends=True):
            item=json.loads(line)
            if '_commit' in item:assert pending and item['crc32']==zlib.crc32(payload);records.append(pending);pending=None
            else:pending=item;payload=line
        assert dump(profile)[:len(records)]==records
        restored=qa/'restored';subprocess.run([str(ROOT/'target/release/kern-core.exe'),'restore','--profile',str(restored),'--input',str(backup_path)],check=True,capture_output=True,timeout=30)
        assert dump(restored)==records
        restored_core=spawn('kern-core','serve','--storage','redb','--profile',restored);wait(lambda:(restored/'endpoint.json').exists())
        actual=rpc(restored,'processing_context',position=rpc(restored,'calibration')['position'])['context'];assert actual==context
        mutate(restored,'stop');restored_core.wait(timeout=10);checks.append('independent backup CRC/event equality and exact graph context/window restored without original database')
        report=dict(passed=True,date=datetime.now(timezone.utc).isoformat(),checks=checks,browser=json.loads((qa/'browser-report.json').read_text()),catalog_parameters=len(catalog['entries']),builtin_presets=len(catalog['presets']),rate_observation=rate,binaries={n:hashlib.sha256((ROOT/'target/release'/n).read_bytes()).hexdigest() for n in ['kern-core.exe','kern-processor.exe','kern-charts.exe','kern-recorder.exe']},not_checked=['Physical depth/load model','Low-end resources','Power loss','24-72h','macOS','Windows10'])
        (qa/'report.json').write_text(json.dumps(report,indent=2)+'\n',encoding='utf8');print(json.dumps(dict(passed=True,report=str(qa/'report.json'))))
    finally:
        stopping.set()
        if feeder:feeder.join(timeout=10)
        for p in reversed(children):
            if p.poll() is None:
                try:
                    if p==children[0]:mutate(profile,'stop')
                    else:p.stdin.write(b'stop\n');p.stdin.flush()
                    p.wait(timeout=8)
                except (OSError,subprocess.TimeoutExpired):p.kill();p.wait(timeout=5)
        for log in logs:log.close()

if __name__=='__main__':main()
