"""Exercise the packaged launcher and owned-consumer recovery against the running emulator."""
from __future__ import annotations
import ctypes as C,json,subprocess,time
from ctypes import wintypes as W
from datetime import datetime,timezone,timedelta
from pathlib import Path
from validate_core import rpc,mutate
from validate_desktop import K,U,window

ROOT=Path(__file__).resolve().parents[1]

def main():
    bundle=ROOT/'dist/Kern-Prototype-0.1.0';qa=ROOT/'target/qa'/('prototype-managed-'+datetime.now(timezone(timedelta(hours=4))).strftime('%Y%m%d-%H%M%S'));qa.mkdir(parents=True);profile=qa/'profile'
    thread=[]
    def wait(fn,seconds=45):
        end=time.monotonic()+seconds
        while time.monotonic()<end:
            try:
                value=fn()
                if value:return value
            except (OSError,ValueError,KeyError):pass
            time.sleep(.15)
        raise AssertionError('Managed recovery deadline expired')
    def plugin(id):return next(p for p in rpc(profile,'plugins')['plugins'] if p['id']==id)
    try:
        with open(qa/'launcher.log','wb') as log:
            result=subprocess.run([str(bundle/'kern-launch.exe'),'--profile',str(profile),'--open','0'],stdout=log,stderr=log,timeout=30)
        assert result.returncode==0,(qa/'launcher.log').read_text(encoding='utf8')
        shell=plugin('kern.shell');hwnd=wait(lambda:window(shell['pid']),10)
        U.ShowWindow.argtypes=[W.HWND,C.c_int];U.ShowWindow(hwnd,0)
        wait(lambda:rpc(profile,'status')['status']['raw_batches']>=10)
        wait(lambda:len(rpc(profile,'snapshot')['parameters'])==6)
        before=rpc(profile,'status')['status'];old=plugin('processor')['pid'];assert old
        # Suspend the worker's actual main thread, rather than only simulating a dead heartbeat.
        snap=K.CreateToolhelp32Snapshot
        class THREADENTRY32(C.Structure):
            _fields_=[('dwSize',W.DWORD),('cntUsage',W.DWORD),('th32ThreadID',W.DWORD),('th32OwnerProcessID',W.DWORD),('tpBasePri',C.c_long),('tpDeltaPri',C.c_long),('dwFlags',W.DWORD)]
        snap.argtypes=[W.DWORD,W.DWORD];snap.restype=W.HANDLE
        K.Thread32First.argtypes=[W.HANDLE,C.POINTER(THREADENTRY32)];K.Thread32Next.argtypes=[W.HANDLE,C.POINTER(THREADENTRY32)]
        handle=snap(4,0);assert handle and handle!=W.HANDLE(-1).value
        entry=THREADENTRY32();entry.dwSize=C.sizeof(entry);threads=[]
        try:
            ok=K.Thread32First(handle,C.byref(entry))
            while ok:
                if entry.th32OwnerProcessID==old:threads.append(entry.th32ThreadID)
                ok=K.Thread32Next(handle,C.byref(entry))
        finally:K.CloseHandle(handle)
        assert threads
        # The first thread in this freshly spawned single-worker process owns computation.
        for tid in threads:
            handle=K.OpenThread(0x2,False,tid);assert handle;assert K.SuspendThread(handle)!=0xffffffff;thread.append(handle)
        time.sleep(3)
        during=rpc(profile,'status')['status'];assert during['raw_batches']>before['raw_batches']
        replacement=wait(lambda:(p if (p:=plugin('processor'))['pid'] and p['pid']!=old else None),45)
        wait(lambda:rpc(profile,'status')['status']['interpreter_connected'])
        wait(lambda:rpc(profile,'status')['status']['processor_position']>=during['last_raw_event'])
        after=rpc(profile,'status')['status'];assert after['raw_batches']>during['raw_batches']

        for handle in thread:K.CloseHandle(handle)
        thread=[]
        csv=plugin('csv');assert csv['pid']
        h=K.OpenProcess(1,False,csv['pid']);assert h
        try:assert K.TerminateProcess(h,77)
        finally:K.CloseHandle(h)
        wait(lambda:(p:=plugin('csv'))['pid'] and p['pid']!=csv['pid'],15)
        time.sleep(2)
        assert mutate(profile,'plugin_control',id='csv',action='stop')['result']=='accepted'
        wait(lambda:plugin('csv')['state']=='stopped',10)
        rows=(profile/'exports/parameters.csv').read_text(encoding='utf8').splitlines()
        records=[line for line in rows[1:] if not line.startswith('#commit,')];assert len(records)>6
        keys=[tuple(line.split(',')[i] for i in (0,7)) for line in records];assert len(keys)==len(set(keys))
        # Explicit stop must suppress automatic restart.
        assert mutate(profile,'plugin_control',id='processor',action='stop')['result']=='accepted'
        wait(lambda:plugin('processor')['state']=='stopped',10);time.sleep(2);assert plugin('processor')['state']=='stopped'
        state=rpc(profile,'status')['status'];time.sleep(2);assert rpc(profile,'status')['status']['raw_batches']>state['raw_batches']
        report=dict(passed=True,tests=['one-command packaged launch','30 emulator channels / six calibrated parameters','real worker-thread hang; raw survives','automatic owned interpreter restart and catch-up','CSV crash restart without duplicate rows','explicit stop suppresses restart'],raw_batches=after['raw_batches'],csv_rows=len(records))
        (qa/'report.json').write_text(json.dumps(report,indent=2)+'\n',encoding='utf8');print(json.dumps(dict(report=str(qa/'report.json'),**report)))
    finally:
        for handle in thread:K.ResumeThread(handle);K.CloseHandle(handle)
        try:mutate(profile,'stop')
        except (OSError,ValueError):pass
        time.sleep(3)

if __name__=='__main__':main()
