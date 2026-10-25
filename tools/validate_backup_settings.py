"""Owned synthetic backup acceptance: destinations, codecs, replay, UI and native picker."""
from pathlib import Path
from datetime import datetime
import ctypes as C
from ctypes import wintypes as W
import json, shutil, subprocess, threading, time, urllib.request
from validate_core import rpc, mutate

ROOT=Path(__file__).resolve().parents[1]
BIN=ROOT/'target/release'

def main():
    qa=ROOT/'target/qa'/('backup-settings-'+datetime.now().strftime('%Y%m%d-%H%M%S'))
    profile=qa/'profile';profile.mkdir(parents=True);owned=[];checks=[]
    def spawn(name,*args):
        log=(qa/(name+str(len(owned))+'.log')).open('wb')
        p=subprocess.Popen([str(BIN/(name+'.exe')),*map(str,args)],stdin=subprocess.PIPE,stdout=log,stderr=log,creationflags=0x08000000)
        owned.append((p,log));return p
    def wait(fn,seconds=20):
        end=time.monotonic()+seconds
        while time.monotonic()<end:
            try:
                v=fn()
                if v:return v
            except (OSError,ValueError,KeyError):pass
            time.sleep(.05)
        raise AssertionError('Backup fixture timed out')
    def stop(p):
        if p.poll() is None:p.stdin.write(b'stop\n');p.stdin.flush();p.wait(timeout=10)
        assert p.returncode==0
    try:
        core=spawn('kern-core','serve','--storage','redb','--profile',profile)
        wait(lambda:rpc(profile,'status')['result']=='status')
        source=mutate(profile,'open_source',id='test',resource='SYNTHETIC',channels=[dict(id=0,name='Raw',value_type='unsigned')],expected_catalog_revision=0)
        sequence=0
        def ingest(n=1):
            nonlocal sequence
            for _ in range(n):
                sequence+=1
                r=mutate(profile,'ingest',batch=dict(source='test',session=source['session'],sequence=sequence,measured_unix_ms=int(time.time()*1000),samples=[dict(channel=0,value=dict(type='unsigned',value=sequence),quality='good')]))
                assert r['result']=='durable',r
        ingest(40)
        backup=spawn('kern-recorder','--profile',profile,'--mode','backup')
        charts=spawn('kern-charts','--profile',profile)
        wait(lambda:(profile/'charts-endpoint.json').exists())
        ui=json.loads((profile/'charts-endpoint.json').read_text());base,key=ui['url'].split('/#access=')
        def http(path,body=None,auth=True):
            request=urllib.request.Request(base+'/api/'+path,data=json.dumps(body).encode() if body is not None else None,headers={'X-Kern-Key':key} if auth else {})
            with urllib.request.build_opener(urllib.request.ProxyHandler({})).open(request,timeout=15) as response:return json.load(response)
        def state():return json.loads((profile/'backup-status.json').read_text())
        wait(lambda:state()['position']==rpc(profile,'status')['status']['event_sequence'])
        original=profile/'backup/events.jsonl';old_size=original.stat().st_size
        files=[original]
        for compression in ['fast','balanced','compact','none']:
            saved=http('backup-settings')
            selected=qa/('destination-'+compression)
            value=http('backup-settings',dict(expected_revision=saved['settings']['revision'],directory=str(selected.resolve()),compression=compression))
            assert value['result']=='backup_settings',value
            ingest(5)
            wait(lambda:state()['active_revision']==value['settings']['revision'] and state()['position']==rpc(profile,'status')['status']['event_sequence'])
            path=Path(state()['file']);assert path.is_relative_to(selected.resolve());assert path.suffix==('.jsonl' if compression=='none' else '.kbk');files.append(path)
        assert original.stat().st_size==old_size
        checks.append('Four profiles apply without stopping intake; destinations catch up; previous file unchanged')
        # Switching codecs within one directory must not contend on a shared lock.
        for mode in ['fast','none','compact']:
            s=http('backup-settings');v=http('backup-settings',dict(expected_revision=s['settings']['revision'],directory=s['settings']['directory'],compression=mode))
            ingest();wait(lambda:state()['active_revision']==v['settings']['revision'] and state()['position']==rpc(profile,'status')['status']['event_sequence'])
        checks.append('Uncompressed/compressed transitions in the same folder catch up without lock contention')
        # Same archive, different level: no rewrite and no lost committed prefix.
        s=http('backup-settings');v=http('backup-settings',dict(expected_revision=s['settings']['revision'],directory=str((qa/'destination-fast').resolve()),compression='fast'))
        wait(lambda:state()['active_revision']==v['settings']['revision']);p=Path(state()['file']);length=p.stat().st_size
        s=http('backup-settings');v=http('backup-settings',dict(expected_revision=s['settings']['revision'],directory=s['settings']['directory'],compression='compact'))
        ingest(2);wait(lambda:state()['active_revision']==v['settings']['revision'] and state()['position']==rpc(profile,'status')['status']['event_sequence']);assert Path(state()['file'])==p and p.stat().st_size>length
        stale=http('backup-settings',dict(expected_revision=0,directory='',compression='none'));assert stale['result']=='error'
        bad=qa/'not-a-folder';bad.write_text('owned fixture')
        s=http('backup-settings');assert http('backup-settings',dict(expected_revision=s['settings']['revision'],directory=str(bad.resolve()),compression='fast'))['result']=='error'
        # Saved but subsequently unavailable destination leaves the last good writer active.
        stop(backup)
        s=http('backup-settings');newdir=qa/'unavailable';v=http('backup-settings',dict(expected_revision=s['settings']['revision'],directory=str(newdir.resolve()),compression='fast'))
        # Start applies desired destination on boot, so restore previous desired config first.
        v=http('backup-settings',dict(expected_revision=v['settings']['revision'],directory=s['settings']['directory'],compression=s['settings']['compression']))
        backup=spawn('kern-recorder','--profile',profile,'--mode','backup');wait(lambda:state()['pid']==backup.pid)
        # Change settings atomically while writer sleeps; an existing file cannot become a folder.
        cfg=json.loads((profile/'backup-settings.json').read_text());cfg['revision']+=1;cfg['directory']=str(bad.resolve())
        temp=profile/'backup-test.tmp';temp.write_text(json.dumps(cfg));temp.replace(profile/'backup-settings.json')
        ingest();wait(lambda:state()['error'] and state()['position']==rpc(profile,'status')['status']['event_sequence']);assert Path(state()['file'])==p
        http('backup-settings',dict(expected_revision=cfg['revision'],directory=s['settings']['directory'],compression=s['settings']['compression']))
        wait(lambda:not state()['error'])
        checks.append('Level change appends same archive; restart no duplicate; stale/bad settings rejected; unavailable target preserves prior writer')
        # Browser form uses installed-component catalog; synthetic serve has no package registry.
        result=subprocess.run(['node','tools/validate_backup_settings_browser.cjs',str(profile),str(qa)],cwd=ROOT,capture_output=True,text=True,timeout=65)
        assert result.returncode==0,result.stdout+result.stderr
        browser=json.loads(result.stdout);checks+=browser['checks']
        # Real native helper: set a filesystem selection, then cancel a second dialog.
        U=C.WinDLL('user32');U.GetWindowThreadProcessId.argtypes=[W.HWND,C.POINTER(W.DWORD)]
        U.SendMessageW.argtypes=[W.HWND,W.UINT,W.WPARAM,W.LPARAM];U.SendMessageW.restype=C.c_ssize_t
        U.PostMessageW.argtypes=[W.HWND,W.UINT,W.WPARAM,W.LPARAM]
        callback=C.WINFUNCTYPE(W.BOOL,W.HWND,W.LPARAM)
        U.EnumWindows.argtypes=[callback,W.LPARAM]
        U.GetClassNameW.argtypes=[W.HWND,W.LPWSTR,C.c_int]
        def dialog(pid):
            found=[]
            @callback
            def visit(hwnd,_):
                owner=W.DWORD();U.GetWindowThreadProcessId(hwnd,C.byref(owner));name=C.create_unicode_buffer(100);U.GetClassNameW(hwnd,name,100)
                if owner.value==pid and name.value=='#32770':found.append(hwnd)
                return True
            U.EnumWindows(visit,0);return found[0] if found else None
        for choose in [True,False]:
            child=subprocess.Popen([str(BIN/'kern-shell.exe'),'--pick-folder','1'],stdout=subprocess.PIPE,stderr=subprocess.PIPE,creationflags=0x08000000);owned.append((child,None))
            hwnd=wait(lambda:dialog(child.pid))
            if choose:
                # BFFM_SETSELECTION is WM_USER: Windows does not marshal its string
                # across processes. Allocate the text in this owned helper only.
                K=C.WinDLL('kernel32',use_last_error=True)
                K.OpenProcess.argtypes=[W.DWORD,W.BOOL,W.DWORD];K.OpenProcess.restype=W.HANDLE
                K.VirtualAllocEx.argtypes=[W.HANDLE,C.c_void_p,C.c_size_t,W.DWORD,W.DWORD];K.VirtualAllocEx.restype=C.c_void_p
                K.WriteProcessMemory.argtypes=[W.HANDLE,C.c_void_p,C.c_void_p,C.c_size_t,C.c_void_p]
                K.VirtualFreeEx.argtypes=[W.HANDLE,C.c_void_p,C.c_size_t,W.DWORD];K.CloseHandle.argtypes=[W.HANDLE]
                text=(str(qa.resolve())+'\0').encode('utf-16-le');handle=K.OpenProcess(0x28,False,child.pid);assert handle
                remote=K.VirtualAllocEx(handle,None,len(text),0x3000,4);assert remote
                try:
                    assert K.WriteProcessMemory(handle,remote,text,len(text),None)
                    U.SendMessageW(hwnd,0x400+103,1,remote)
                finally:K.VirtualFreeEx(handle,remote,0,0x8000);K.CloseHandle(handle)
                time.sleep(.2);U.PostMessageW(hwnd,0x111,1,0)
            else:U.PostMessageW(hwnd,0x111,2,0)
            out,err=child.communicate(timeout=12);assert child.returncode==0,(child.returncode,out,err)
            value=json.loads(out);assert value.get('folder')==(str(qa.resolve()) if choose else None),value
        checks.append('Native filesystem folder picker choose/cancel; separate helper process')
        # The authenticated host owns/cancels a picker without blocking acquisition.
        K=C.WinDLL('kernel32',use_last_error=True)
        class Entry(C.Structure):
            _fields_=[('size',W.DWORD),('usage',W.DWORD),('pid',W.DWORD),('heap',C.c_size_t),('module',W.DWORD),('threads',W.DWORD),('parent',W.DWORD),('priority',W.LONG),('flags',W.DWORD),('exe',W.WCHAR*260)]
        K.CreateToolhelp32Snapshot.argtypes=[W.DWORD,W.DWORD];K.CreateToolhelp32Snapshot.restype=W.HANDLE
        K.Process32FirstW.argtypes=[W.HANDLE,C.POINTER(Entry)];K.Process32NextW.argtypes=[W.HANDLE,C.POINTER(Entry)];K.CloseHandle.argtypes=[W.HANDLE]
        def helper():
            snap=K.CreateToolhelp32Snapshot(2,0);entry=Entry();entry.size=C.sizeof(entry);pid=None
            try:
                more=K.Process32FirstW(snap,C.byref(entry))
                while more:
                    if entry.parent==charts.pid and entry.exe=='kern-shell.exe':pid=entry.pid;break
                    more=K.Process32NextW(snap,C.byref(entry))
            finally:K.CloseHandle(snap)
            return pid
        replies=[]
        request=threading.Thread(target=lambda:replies.append(http('backup-folder',{})))
        request.start();pid=wait(helper);hwnd=wait(lambda:dialog(pid))
        assert http('backup-folder',{})['result']=='error'
        before=rpc(profile,'status')['status']['raw_batches'];ingest();assert rpc(profile,'status')['status']['raw_batches']>before
        U.PostMessageW(hwnd,0x111,2,0);request.join(timeout=10);assert not request.is_alive() and replies[-1]['folder'] is None
        request=threading.Thread(target=lambda:replies.append(http('backup-folder',{})))
        request.start();wait(helper);started=time.monotonic();stop(charts);assert time.monotonic()-started<4;request.join(timeout=5);assert not request.is_alive() and replies[-1]['result']=='error'
        checks.append('Authenticated picker single-instance; intake advances while dialog open; host stop cancels its helper')
        stop(backup);stop(charts)
        for i,path in enumerate(files):
            restored=qa/f'restored-{i}'
            r=subprocess.run([str(BIN/'kern-core.exe'),'restore','--profile',str(restored),'--input',str(path)],capture_output=True,text=True,timeout=30)
            assert r.returncode==0,r.stderr
            count=json.loads(r.stdout)['restored_events'];assert count>40
            dump=subprocess.run([str(BIN/'kern-core.exe'),'dump','--profile',str(restored)],capture_output=True,text=True,timeout=30);assert dump.returncode==0,dump.stderr
            actual=[json.loads(line) for line in dump.stdout.splitlines()]
            # Compare every restored event against core's canonical history, not just counters.
            expected=[];cursor=0
            while cursor<count:
                h=rpc(profile,'history',after=cursor);expected+=h['events'];cursor=h['next']
            assert actual==expected[:count],(i,len(actual),count)
        checks.append('Legacy and three compressed destinations restore every event exactly to independent new redb profiles')
        # A torn final compressed block restores prefix, while complete damage fails closed.
        raw=files[1].read_bytes();torn=qa/'torn.kbk';torn.write_bytes(raw[:-1])
        r=subprocess.run([str(BIN/'kern-core.exe'),'restore','--profile',str(qa/'restored-torn'),'--input',str(torn)],capture_output=True,text=True,timeout=30)
        assert r.returncode==0 and json.loads(r.stdout)['uncommitted_tail'],r.stderr
        damaged=bytearray(raw);damaged[36]^=1;badpath=qa/'damaged.kbk';badpath.write_bytes(damaged)
        r=subprocess.run([str(BIN/'kern-core.exe'),'restore','--profile',str(qa/'restored-damaged'),'--input',str(badpath)],capture_output=True,text=True,timeout=30)
        assert r.returncode!=0 and (qa/'restored-damaged/import-incomplete').exists()
        checks.append('Torn final block reports excluded tail; complete CRC damage refuses successful restore')
        mutate(profile,'stop');core.wait(timeout=10);assert core.returncode==0
        report=dict(passed=True,checks=checks,browser=browser,hardware_ports_used=False,weak_hardware_benchmark=False)
        (qa/'report.json').write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(dict(passed=True,report=str(qa/'report.json'))))
    finally:
        for child,log in reversed(owned):
            if child.poll() is None:
                try:
                    if child.stdin:child.stdin.write(b'stop\n');child.stdin.flush()
                    child.wait(timeout=4)
                except (OSError,subprocess.TimeoutExpired):child.kill();child.wait(timeout=5)
            if log:log.close()

if __name__=='__main__':main()
