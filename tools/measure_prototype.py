"""Read-only short Windows observation of the core and its managed plugin processes."""
from __future__ import annotations
import argparse,json,time,types
from pathlib import Path
from datetime import datetime,timezone,timedelta
from validate_core import rpc,windows_metrics
from validate_desktop import K

def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--profile',type=Path,required=True);p.add_argument('--seconds',type=float,default=5);p.add_argument('--without-viewer',action='store_true');args=p.parse_args()
    assert 1<=args.seconds<=30
    root=Path(__file__).resolve().parents[1]
    qa=root/'target/qa'/('prototype-observation-'+datetime.now(timezone(timedelta(hours=4))).strftime('%Y%m%d-%H%M%S'));qa.mkdir(parents=True)
    endpoint=json.loads((args.profile/'endpoint.json').read_text());ids={'core':endpoint['pid']}
    ids.update({p['id']:p['pid'] for p in rpc(args.profile,'plugins')['plugins'] if p['pid']})
    handles={name:K.OpenProcess(0x410,False,pid) for name,pid in ids.items()}
    try:
        assert all(handles.values())
        before={name:windows_metrics(types.SimpleNamespace(_handle=h)) for name,h in handles.items()}
        start=time.monotonic();time.sleep(args.seconds);seconds=time.monotonic()-start
        after={name:windows_metrics(types.SimpleNamespace(_handle=h)) for name,h in handles.items()}
    finally:
        for h in handles.values():
            if h:K.CloseHandle(h)
    report=dict(scenario='Live core and owned plugins only; browser runtime is not included',viewer_declared_absent=args.without_viewer,seconds=seconds,
        processes={name:dict(one_cpu_percent=100*(after[name]['cpu_seconds']-a['cpu_seconds'])/seconds,working_set_bytes=after[name]['working_set_bytes'],private_bytes=after[name]['private_bytes']) for name,a in before.items()},
        not_measured=['Browser runtime memory/CPU','GPU','Low-end hardware','24-72h endurance'])
    (qa/'report.json').write_bytes((json.dumps(report,indent=2)+'\n').encode('utf8'));print(json.dumps(dict(report=str(qa/'report.json'),**report)))

if __name__=='__main__':main()
