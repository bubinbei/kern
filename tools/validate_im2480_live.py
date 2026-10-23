"""Read an already-running emulator through an explicit config, never reconfigure it."""
from __future__ import annotations
import argparse
from datetime import datetime,timezone,timedelta
import hashlib
import json
from pathlib import Path
import subprocess
import time
from validate_core import rpc,mutate,journal

ROOT=Path(__file__).resolve().parents[1]

def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--config',type=Path,required=True);args=p.parse_args()
    config=json.loads(args.config.read_text(encoding='utf-8'))
    plugin=ROOT/'target/release/im2480.exe';core_exe=ROOT/'target/release/kern-core.exe'
    subprocess.run([str(plugin),'check','--config',str(args.config.resolve())],check=True,capture_output=True,timeout=5)
    qa=ROOT/'target/qa'/('im2480-live-'+datetime.now(timezone(timedelta(hours=4))).strftime('%Y%m%d-%H%M%S'))
    qa.mkdir();profile=qa/'profile';log=open(qa/'core.log','wb')
    core=subprocess.Popen([str(core_exe),'serve','--profile',str(profile)],stdout=log,stderr=log,creationflags=0x08000000)
    try:
        end=time.monotonic()+8
        while time.monotonic()<end:
            try:
                if rpc(profile,'status')['result']=='status':break
            except (OSError,ValueError,KeyError):pass
            time.sleep(.02)
        result=subprocess.run([str(plugin),'run','--profile',str(profile),'--config',str(args.config.resolve()),'--cycles','2'],capture_output=True,text=True,timeout=90,creationflags=0x08000000)
        sources=rpc(profile,'snapshot')['sources'];n=len(config['modules'])
        assert result.returncode==0,(result.returncode,result.stderr)
        source=next(s for s in sources if s['id']==config['source_id'])
        assert source['last_sequence']==n*2 and len(source['channels'])==n*6 and not source['online']
        assert all(c['value'] is not None for c in source['channels'])
        assert mutate(profile,'stop')['result']=='durable';assert core.wait(timeout=8)==0
        events,tail=journal(profile);assert tail is None
        batches=[e['batch'] for e in events if e['event']=='batch_stored']
        assert len(batches)==n*2 and all(s['quality']=='good' and s['value'] is not None for b in batches for s in b['samples'])
        report={'schema':1,'transport':'Existing operator emulator; explicit local COM configuration','module_count':n,'passed':True,
                'batches':len(batches),'samples':sum(len(b['samples']) for b in batches),'channel_count':n*6,'independent_journal_check':True,
                'plugin_binary':{'bytes':plugin.stat().st_size,'sha256':hashlib.sha256(plugin.read_bytes()).hexdigest()},
                'not_checked':['Physical hardware','Long-duration run','Other data orders']}
        (qa/'report.json').write_text(json.dumps(report,indent=2)+'\n',encoding='utf-8');print(json.dumps({'report':str((qa/'report.json').relative_to(ROOT)),**report}))
    finally:
        if core.poll() is None:
            try:mutate(profile,'stop');core.wait(timeout=8)
            except Exception:core.kill();core.wait(timeout=3)
        log.close()

if __name__=='__main__':main()
