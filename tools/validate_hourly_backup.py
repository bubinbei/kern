"""Owned redb/processor/hourly-archive acceptance. No hardware or operator data."""
from pathlib import Path
from datetime import datetime
import json,subprocess,time,urllib.request,shutil
from validate_core import rpc,mutate
ROOT=Path(__file__).resolve().parents[1];BIN=ROOT/'target/release'
def main():
 qa=ROOT/'target/qa'/('hourly-backup-'+datetime.now().strftime('%Y%m%d-%H%M%S'));profile=qa/'profile';profile.mkdir(parents=True);children=[];logs=[];checks=[]
 def spawn(name,*args):
  log=(qa/(name+str(len(children))+'.log')).open('wb');logs.append(log);p=subprocess.Popen([str(BIN/(name+'.exe')),*map(str,args)],stdin=subprocess.PIPE,stdout=log,stderr=log,creationflags=0x08000000);children.append(p);return p
 def wait(fn,seconds=20):
  end=time.monotonic()+seconds
  while time.monotonic()<end:
   try:
    v=fn()
    if v:return v
   except (OSError,ValueError,KeyError):pass
   time.sleep(.04)
  raise AssertionError('Hourly backup fixture timed out')
 def stop(p):
  if p.poll() is None:p.stdin.write(b'stop\n');p.stdin.flush();p.wait(timeout=10)
  assert p.returncode==0,p.returncode
 def state():return json.loads((profile/'backup-status.json').read_text())
 def cli(command,*args):
  r=subprocess.run([str(BIN/'kern-core.exe'),command,*map(str,args)],capture_output=True,text=True,encoding='utf8',timeout=35);assert r.returncode==0,r.stderr;return r.stdout
 def history(p=profile):
  out=[];cursor=0;through=rpc(p,'status')['status']['event_sequence']
  while cursor<through:
   v=rpc(p,'history',after=cursor);out+=v['events'];cursor=v['next']
   if not v['events']:break
  return out
 try:
  core=spawn('kern-core','serve','--storage','redb','--profile',profile);wait(lambda:rpc(profile,'status')['result']=='status')
  m=json.loads((ROOT/'plugins/calibration.example.json').read_text());m['parameters']=m['parameters'][:1];m['parameters'][0].update(source='test',name='Reference weight',points=[dict(raw=0,value=0),dict(raw=10000,value=100)],min=0,max=200)
  assert mutate(profile,'save_calibration',expected_revision=0,map=m)['result']=='durable'
  processor=spawn('kern-processor','--profile',profile);wait(lambda:rpc(profile,'calibration')['active_revision']==1)
  source=mutate(profile,'open_source',id='test',resource='SYNTHETIC',channels=[dict(id=0,name='Raw',value_type='unsigned')],expected_catalog_revision=0)
  backup=spawn('kern-recorder','--profile',profile,'--mode','backup');charts=spawn('kern-charts','--profile',profile)
  wait(lambda:(profile/'charts-endpoint.json').exists());ui=json.loads((profile/'charts-endpoint.json').read_text());base,key=ui['url'].split('/#access=')
  def api(body=None):
   req=urllib.request.Request(base+'/api/backup-settings',data=json.dumps(body).encode() if body is not None else None,headers={'X-Kern-Key':key})
   with urllib.request.build_opener(urllib.request.ProxyHandler({})).open(req,timeout=8) as response:return json.load(response)
  assert api()['settings']['layout']=='hourly_v2' and api()['settings']['compression']=='balanced'
  for i in range(1,131):
   if i==55:
    m['parameters'][0]['points'][1]['value']=200
    assert mutate(profile,'save_calibration',expected_revision=1,map=m)['result']=='durable';wait(lambda:rpc(profile,'calibration')['active_revision']==2)
   b=dict(source='test',session=source['session'],sequence=i,measured_unix_ms=int(time.time()*1000),samples=[dict(channel=0,value=dict(type='unsigned',value=i*10),quality='good')])
   result=mutate(profile,'ingest',batch=b);assert result['result']=='durable'
   if i==72:
    wait(lambda:state()['position']>=result['event_sequence']);backup.kill();backup.wait(timeout=5);backup=spawn('kern-recorder','--profile',profile,'--mode','backup');wait(lambda:state()['pid']==backup.pid)
   time.sleep(.09)
  wait(lambda:rpc(profile,'calibration')['position']==rpc(profile,'status')['status']['last_raw_event']);wait(lambda:state()['position']==rpc(profile,'status')['status']['event_sequence'])
  assert state()['error'] is None;root=profile/'backup/hourly-v2';assert list(root.glob('????-??-??/*.kbk'))
  checks.append('Fresh default hourly-v2/balanced; continuous raw, sparse snapshots, hot calibration rotation, forced writer restart and tail catch-up')
  browser=subprocess.run(['node','tools/validate_hourly_backup_browser.cjs',str(profile),str(qa)],cwd=ROOT,capture_output=True,text=True,timeout=35);assert browser.returncode==0,browser.stderr
  checks.append('Real browser: hourly/legacy selector, balanced default, Cancel keeps format, 1280x720 footer and no page errors')
  # Destination changes retain the old collection and catch up the complete history.
  s=api();v=api(dict(expected_revision=s['settings']['revision'],directory=str((qa/'external').resolve()),compression='fast',layout='hourly_v2'));assert v['result']=='backup_settings'
  wait(lambda:state()['active_revision']==v['settings']['revision'] and state()['position']==rpc(profile,'status')['status']['event_sequence'])
  external=Path(state()['archive_root']);assert external.is_relative_to((qa/'external').resolve());assert root.exists()
  s=api();v=api(dict(expected_revision=s['settings']['revision'],directory='',compression='balanced',layout='hourly_v2'));wait(lambda:state()['active_revision']==v['settings']['revision'])
  stop(processor);wait(lambda:state()['position']==rpc(profile,'status')['status']['event_sequence']);stop(backup);stop(charts);expected=history()
  info=json.loads(cli('backup-info','--input',root));files=info['files'];assert sum(f['recipes'] for f in files)>100;assert 2<=sum(f['snapshots'] for f in files)<15
  changed=[f for f in files if f['reason_code']=='calibration' and f['new_revision']==2];assert changed and 'Reference weight' in changed[0]['reason'];assert changed[0]['active_map']['parameters'][0]['points'][1]['value']==200
  checks.append('Header has active map/new revision and named reason; full snapshots limited to 10-second buckets; intermediate result recipes retained')
  restored=qa/'restored-all';report=json.loads(cli('restore','--input',root,'--profile',restored));assert report['format']=='hourly-v2' and not report['uncommitted_tail'];cli('check','--profile',restored)
  dumped=[json.loads(l) for l in cli('dump','--profile',restored).splitlines()];assert [r for r in dumped if 'data' in r]==expected
  checks.append('Whole collection restores every original raw/control/result event exactly; independent core dump equals canonical live history')
  one=Path(changed[0]['file']);single=qa/'restored-one';single_report=json.loads(cli('restore','--input',one,'--profile',single));assert single_report['baseline_position']>0;cli('check','--profile',single)
  one_dump=[json.loads(l) for l in cli('dump','--profile',single).splitlines()];after=[e for e in expected if changed[0]['first_sequence']<=e['sequence']<=changed[0]['last_sequence']];assert [r for r in one_dump if 'data' in r]==after
  checks.append('Calibration file alone restores from its validated state/map baseline and keeps exact original event numbers')
  # A restored standalone interval can boot and produce another hourly backup.
  second=spawn('kern-core','serve','--storage','redb','--profile',single);wait(lambda:rpc(single,'status')['result']=='status');new_backup=spawn('kern-recorder','--profile',single,'--mode','backup');wait(lambda:(single/'backup-status.json').exists() and json.loads((single/'backup-status.json').read_text())['position']==rpc(single,'status')['status']['event_sequence']);stop(new_backup);mutate(single,'stop');second.wait(timeout=10);assert second.returncode==0
  checks.append('Restored baseline replays on boot; paged baseline API lets backup resume on the restored interval')
  # Missing middle files and a missing active disk tail cannot masquerade as complete.
  broken=qa/'broken';shutil.copytree(root,broken)
  middle=Path(files[1]['file']);(broken/middle.relative_to(root)).unlink()
  bad=subprocess.run([str(BIN/'kern-core.exe'),'restore','--input',str(broken),'--profile',str(qa/'refused')],capture_output=True,text=True,timeout=30);assert bad.returncode!=0 and (qa/'refused/import-incomplete').exists()
  active=Path(files[-1]['file']);tail=active.with_suffix('.tail');assert tail.exists();saved=tail.read_bytes();tail.unlink()
  bad=subprocess.run([str(BIN/'kern-core.exe'),'restore','--input',str(active),'--profile',str(qa/'refused-tail')],capture_output=True,text=True,timeout=30);assert bad.returncode!=0;tail.write_bytes(saved)
  checks.append('A missing middle file or required active tail fails closed; incomplete import marker remains')
  total=sum(p.stat().st_size for p in root.rglob('*') if p.is_file());raw_time=expected[-1]['data'].get('batch',{}).get('received_unix_ms',int(time.time()*1000))
  mutate(profile,'stop');core.wait(timeout=10);assert core.returncode==0
  output=dict(passed=True,checks=checks,files=len(files),snapshots=sum(f['snapshots'] for f in files),recipes=sum(f['recipes'] for f in files),events=len(expected),archive_bytes=total,weak_machine_or_month_size_acceptance=False,hardware_ports_used=False)
  (qa/'report.json').write_text(json.dumps(output,indent=2)+'\n');print(json.dumps(dict(passed=True,report=str(qa/'report.json'))))
 finally:
  for p in reversed(children):
   if p.poll() is None:
    try:p.stdin.write(b'stop\n');p.stdin.flush();p.wait(timeout=4)
    except (OSError,subprocess.TimeoutExpired):p.kill();p.wait(timeout=5)
  for log in logs:log.close()
if __name__=='__main__':main()
