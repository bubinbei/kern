"""Short, per-process observations on owned synthetic profiles; not hardware acceptance."""
import json
import platform
import subprocess
import time
import validate_core as v


def main():
    v.QA = v.QA.with_name(v.QA.name.replace('core-', 'core-resources-', 1))
    v.QA.mkdir(parents=True)
    profile = v.QA/'profile'
    proc = v.start(profile)
    idle_before = v.windows_metrics(proc)
    start = time.monotonic(); time.sleep(5); idle_after = v.windows_metrics(proc); idle_seconds=time.monotonic()-start
    a=v.demo(profile,'a',6,100,1000,100)
    b=v.demo(profile,'b',6,100,2000,100)
    start=time.monotonic();before=v.windows_metrics(proc);time.sleep(5);after=v.windows_metrics(proc);seconds=time.monotonic()-start
    assert a.wait(timeout=15)==b.wait(timeout=15)==0
    status=v.rpc(profile,'status')['status'];assert status['raw_batches']==200 and status['raw_samples']==1200
    v.stop(profile,proc)
    def metrics(before,after,seconds):
        return None if before is None else {'seconds':seconds,'one_cpu_percent':100*(after['cpu_seconds']-before['cpu_seconds'])/seconds,
                                           'working_set_bytes':after['working_set_bytes'],'private_bytes':after['private_bytes']}
    report={'platform':platform.platform(),'scenario':'two sources, six channels each; 100 ms sleep after each acknowledged batch; 200 batches/1200 samples total',
            'idle':metrics(idle_before,idle_after,idle_seconds),'live':metrics(before,after,seconds),
            'not_measured':['GPU','weak_hardware','24_to_72_hours','peak_load']}
    (v.QA/'report.json').write_text(json.dumps(report,indent=2)+'\n',encoding='utf-8')
    print(json.dumps(report,indent=2));print('Evidence:',v.QA.relative_to(v.ROOT))


if __name__=='__main__':
    try:main()
    finally:
        for proc,log,profile in reversed(v.OWNED):
            if proc.poll() is None:
                try:v.rpc(profile,'stop');proc.wait(timeout=2)
                except (OSError,ValueError,subprocess.TimeoutExpired):pass
                if proc.poll() is None:proc.kill();proc.wait(timeout=5)
            log.close()
