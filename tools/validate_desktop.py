"""Windows desktop acceptance. Uses only new synthetic profiles and owned processes.

Build release first. The UI thread suspension tests a real unresponsive message pump.
No hardware, global process termination or operator profiles are used.
"""
from __future__ import annotations
import ctypes as C
from ctypes import wintypes as W
from datetime import datetime, timezone, timedelta
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import time
from validate_core import rpc, mutate, journal

ROOT = Path(__file__).resolve().parents[1]
QA = ROOT/'target/qa'/('desktop-'+datetime.now(timezone(timedelta(hours=4))).strftime('%Y%m%d-%H%M%S')+'-'+str(os.getpid()))
CORE = ROOT/'target/release/kern-core.exe'
SHELL = ROOT/'target/release/kern-shell.exe'
U = C.WinDLL('user32', use_last_error=True)
K = C.WinDLL('kernel32', use_last_error=True)
U.GetWindowThreadProcessId.argtypes = [W.HWND, C.POINTER(W.DWORD)]
U.GetDlgItem.argtypes = [W.HWND, C.c_int]; U.GetDlgItem.restype = W.HWND
U.SendMessageW.argtypes = [W.HWND, W.UINT, W.WPARAM, W.LPARAM]; U.SendMessageW.restype = C.c_ssize_t
U.PostMessageW.argtypes = [W.HWND, W.UINT, W.WPARAM, W.LPARAM]
U.SetWindowTextW.argtypes = [W.HWND, W.LPCWSTR]
K.OpenProcess.argtypes = [W.DWORD, W.BOOL, W.DWORD]; K.OpenProcess.restype = W.HANDLE
K.TerminateProcess.argtypes = [W.HANDLE, W.UINT]
K.CloseHandle.argtypes = [W.HANDLE]
K.OpenThread.argtypes = [W.DWORD, W.BOOL, W.DWORD]; K.OpenThread.restype = W.HANDLE
K.SuspendThread.argtypes = [W.HANDLE]; K.SuspendThread.restype = W.DWORD
K.ResumeThread.argtypes = [W.HANDLE]; K.ResumeThread.restype = W.DWORD
CALLBACK = C.WINFUNCTYPE(W.BOOL, W.HWND, W.LPARAM)
U.EnumWindows.argtypes = [CALLBACK, W.LPARAM]
U.GetClassNameW.argtypes = [W.HWND, W.LPWSTR, C.c_int]

def until(check, seconds=12):
    deadline = time.monotonic()+seconds
    while time.monotonic()<deadline:
        value=check()
        if value: return value
        time.sleep(.05)
    raise AssertionError('Deadline expired')

def window(pid):
    result=[]
    @CALLBACK
    def visit(hwnd, _):
        found=W.DWORD(); U.GetWindowThreadProcessId(hwnd,C.byref(found))
        name=C.create_unicode_buffer(128); U.GetClassNameW(hwnd,name,128)
        if found.value==pid and name.value=='KernSystemShell': result.append(hwnd)
        return True
    U.EnumWindows(visit,0)
    return result[0] if result else None

def child(profile):
    result=rpc(profile,'plugins')
    assert result['result']=='plugins',result
    return result['plugins'][0]

def status(profile): return rpc(profile,'status')['status']

def kill_shell_owned(profile, core):
    assert core.poll() is None
    view=child(profile); assert view['id']=='kern.shell' and view['required']
    assert window(view['pid']), 'Only terminate the verified owned shell window process'
    handle=K.OpenProcess(1,False,view['pid']); assert handle
    try: assert K.TerminateProcess(handle,77)
    finally: K.CloseHandle(handle)
    return view['pid']

class TrayId(C.Structure):
    _fields_=[('cbSize',W.DWORD),('hWnd',W.HWND),('uID',W.UINT),('guid',C.c_ubyte*16)]
S=C.WinDLL('shell32',use_last_error=True)
S.Shell_NotifyIconGetRect.argtypes=[C.POINTER(TrayId),C.POINTER(W.RECT)];S.Shell_NotifyIconGetRect.restype=C.c_long
U.GetGuiResources.argtypes=[W.HANDLE,W.DWORD];U.GetGuiResources.restype=W.DWORD
U.GetMenuStringW.argtypes=[W.HMENU,W.UINT,W.LPWSTR,C.c_int,W.UINT]
def tray_exists(hwnd):
    item=TrayId(C.sizeof(TrayId),hwnd,1);rect=W.RECT();return S.Shell_NotifyIconGetRect(C.byref(item),C.byref(rect))==0
def child_texts(hwnd):
    texts=[]
    @CALLBACK
    def visit(child,_):
        value=C.create_unicode_buffer(2048);U.GetWindowTextW(child,value,2048);texts.append(value.value);return True
    U.EnumChildWindows(hwnd,visit,0);return texts
def context_rows(pid):
    result=[]
    @CALLBACK
    def visit(hwnd,_):
        owner=W.DWORD();U.GetWindowThreadProcessId(hwnd,C.byref(owner));name=C.create_unicode_buffer(128);U.GetClassNameW(hwnd,name,128)
        if owner.value==pid and name.value=='#32768':
            menu=U.SendMessageW(hwnd,0x1e1,0,0)
            for n in range(20):
                value=C.create_unicode_buffer(1024)
                if U.GetMenuStringW(menu,n,value,1024,0x400):result.append(value.value)
        return True
    U.EnumWindows(visit,0);return result

def main():
    assert os.name=='nt'
    QA.mkdir(parents=True)
    profile=QA/'profile'; packages=QA/'plugins'; package=packages/'synthetic'; package.mkdir(parents=True)
    shutil.copyfile(ROOT/'plugins/synthetic/plugin.json',package/'plugin.json')
    shutil.copyfile(ROOT/'target/release/kern-demo.exe',package/'kern-demo.exe')
    log=open(QA/'core.log','wb',buffering=0)
    core=subprocess.Popen([str(CORE),'run','--profile',str(profile),'--plugins',str(packages)],cwd=ROOT,stdout=log,stderr=log)
    checks=[]; thread_handle=None
    try:
        until(lambda:(profile/'endpoint.json').exists())
        until(lambda:status(profile)['shell_connected'])
        pid=child(profile)['pid']; hwnd=until(lambda:window(pid)); checks.append('native_shell_connected')
        until(lambda:tray_exists(hwnd));until(lambda:any('Свободно:' in t and 'Файлы данных:' in t and 'н/д' not in t for t in child_texts(hwnd)));checks.append('native_tray_registered_and_storage_sizes_visible')
        # A native confirmation dialog has a nested message pump; it must not hide
        # the UI heartbeat even when the operator waits beyond the hang timeout.
        U.PostMessageW(hwnd,0x111,106,0)
        time.sleep(11)
        assert child(profile)['pid']==pid and status(profile)['shell_connected']
        @CALLBACK
        def cancel_dialog(dialog, _):
            found=W.DWORD(); U.GetWindowThreadProcessId(dialog,C.byref(found))
            name=C.create_unicode_buffer(128); U.GetClassNameW(dialog,name,128)
            if found.value==pid and name.value=='#32770': U.PostMessageW(dialog,0x111,7,0)
            return True
        U.EnumWindows(cancel_dialog,0)
        assert status(profile)['raw_batches']==0
        assert len(rpc(profile,'plugins')['plugins'])==2; checks.append('discovery_does_not_execute')
        edit=U.GetDlgItem(hwnd,121); assert edit
        value=C.create_unicode_buffer('1700')
        assert U.SendMessageW(edit,0x0c,0,C.addressof(value)) # WM_SETTEXT updates the cross-process EDIT control.
        U.PostMessageW(hwnd,0x111,104,0)
        until(lambda:status(profile)['settings']['stale_after_ms']==1700); checks.append('gui_settings_durable')
        conflict=mutate(profile,'set_settings',expected_revision=0,settings={'stale_after_ms':1800})
        assert conflict['code']=='revision_conflict'; checks.append('stale_settings_revision_rejected')
        assert rpc(profile,'plugin_control',id='kern.shell',action='stop')['code']=='required'; checks.append('required_shell_cannot_be_disabled')
        lst=U.GetDlgItem(hwnd,120)
        until(lambda:U.SendMessageW(lst,0x18b,0,0)==2) # LB_GETCOUNT: wait for actual UI state.
        U.SendMessageW(lst,0x186,1,0) # LB_SETCURSEL
        U.PostMessageW(hwnd,0x111,102,0)
        until(lambda:status(profile)['raw_batches']>=4); checks.append('gui_starts_synthetic_package')
        # The same cached icon is mirrored in WM_GETICON for titlebar/accessibility.
        icons=set()
        process=K.OpenProcess(0x400,False,pid);assert process
        baseline=(U.GetGuiResources(process,0),U.GetGuiResources(process,1));assert all(baseline),baseline
        try:
            for _ in range(30):icons.add(U.SendMessageW(hwnd,0x7f,0,0));time.sleep(.1)
            after=(U.GetGuiResources(process,0),U.GetGuiResources(process,1));assert all(after),after;assert all(b<=a+2 for a,b in zip(baseline,after)),(baseline,after)
        finally:K.CloseHandle(process)
        assert 0 not in icons and 2<=len(icons)<=10;checks.append('confirmed_raw_changes_cached_icon_frames_without_gdi_growth')
        U.PostMessageW(hwnd,0x8000+12,0,(1<<16)|0x7b)
        until(lambda:any('База:' in t and 'Резерв:' in t for t in context_rows(pid)))
        menu_before=status(profile)['raw_batches'];time.sleep(10)
        assert child(profile)['pid']==pid and status(profile)['shell_connected'] and status(profile)['raw_batches']>menu_before
        U.PostMessageW(hwnd,0x1f,0,0);checks.append('tray_context_menu_shows_components_and_keeps_heartbeat_over_watchdog_timeout')
        taskbar=U.RegisterWindowMessageW('TaskbarCreated');U.PostMessageW(hwnd,taskbar,0,0);until(lambda:tray_exists(hwnd));checks.append('taskbar_restart_notification_recreates_owned_icon')
        U.PostMessageW(hwnd,0x111,103,0)
        until(lambda:status(profile)['active_sources']==0)
        until(lambda:rpc(profile,'plugins')['plugins'][1]['state']=='stopped'); checks.append('cooperative_stop_closes_source')
        assert rpc(profile,'plugin_control',id='kern.synthetic',action='start')['result']=='accepted'
        until(lambda:status(profile)['active_sources']==1); checks.append('package_restarts_with_new_source_session')
        before=status(profile)['raw_batches']; old=kill_shell_owned(profile,core)
        until(lambda:child(profile)['pid'] not in (None,old) and status(profile)['shell_connected'])
        until(lambda:status(profile)['raw_batches']>before+5); checks.append('shell_crash_restarts_without_stopping_raw')
        new_pid=child(profile)['pid']; hwnd=until(lambda:window(new_pid))
        own_pid=W.DWORD(); tid=U.GetWindowThreadProcessId(hwnd,C.byref(own_pid)); assert own_pid.value==new_pid
        thread_handle=K.OpenThread(0x0002,False,tid); assert thread_handle
        assert K.SuspendThread(thread_handle)!=0xffffffff
        before=status(profile)['raw_batches']
        until(lambda:child(profile)['pid'] not in (None,new_pid) and status(profile)['shell_connected'],20)
        assert status(profile)['raw_batches']>before+20; checks.append('hung_ui_thread_restarts_while_recording_continues')
        K.CloseHandle(thread_handle);thread_handle=None
        final_pid=child(profile)['pid']; hwnd=until(lambda:window(final_pid))
        U.PostMessageW(hwnd,0x10,0,0)
        until(lambda:not U.IsWindowVisible(hwnd));before=status(profile)['raw_batches'];time.sleep(2)
        assert child(profile)['pid']==final_pid and status(profile)['shell_connected'] and status(profile)['raw_batches']>before and tray_exists(hwnd);checks.append('normal_window_close_hides_to_tray_and_keeps_shell_and_recording_alive')
        U.PostMessageW(hwnd,0x8000+12,0,(1<<16)|1024);until(lambda:U.IsWindowVisible(hwnd));checks.append('tray_selection_reopens_same_shell')
        assert rpc(profile,'plugin_control',id='kern.shell',action='start')['result']=='accepted'
        # Three fresh launches per minute after reset (the existing child is not a launch), then latched safe mode; raw keeps progressing.
        for i in range(4):
            old=kill_shell_owned(profile,core)
            if i<3: until(lambda:child(profile)['pid'] not in (None,old) and status(profile)['shell_connected'])
        until(lambda:child(profile)['state']=='closed' and child(profile)['pid'] is None)
        before=status(profile)['raw_batches']; time.sleep(2)
        assert status(profile)['raw_batches']>before and 'safe mode' in rpc(profile,'plugins')['diagnostic']
        checks.append('restart_budget_enters_safe_mode_without_losing_acquisition')
        assert rpc(profile,'plugin_control',id='kern.synthetic',action='stop')['result']=='accepted'
        until(lambda:status(profile)['active_sources']==0)
        final=status(profile)
        assert mutate(profile,'stop')['result']=='durable'; core.wait(timeout=10);assert core.returncode==0
        events,pending=journal(profile)
        assert pending is None
        batches=[e for e in events if e['event']=='batch_stored']
        assert len(batches)==final['raw_batches'];checks.append('independent_journal_count_matches_live_after_ui_failures')
        source_hash=hashlib.sha256()
        files=sorted([ROOT/'Cargo.toml',ROOT/'Cargo.lock',*(ROOT/'crates').rglob('*.toml'),*(ROOT/'crates').rglob('*.rs')],key=lambda p:p.relative_to(ROOT).as_posix())
        for path in files:source_hash.update(path.relative_to(ROOT).as_posix().encode()+b'\0'+path.read_bytes().replace(b'\r\n',b'\n'))
        report={'schema':1,'rust_source_sha256':source_hash.hexdigest(),'ui_confirmation_over_hang_timeout':True,'checks':checks,'tray_cached_frames_observed':len(icons),'gui_resources_before_after':[baseline,after],'raw_batches':final['raw_batches'],'raw_samples':final['raw_samples'],
                'binaries':{p.name:{'bytes':p.stat().st_size,'sha256':hashlib.sha256(p.read_bytes()).hexdigest()} for p in (CORE,SHELL)},
                'platform':'Windows 11; automated native UI, synthetic profile only',
                'limits':['No Windows 10/weak-hardware/long-duration acceptance','Native plugins are trusted local processes, not a sandbox','Synthetic source only; physical devices, consumer file-fsync telemetry and weak-machine resources not accepted']}
        (QA/'report.json').write_text(json.dumps(report,ensure_ascii=False,indent=2)+'\n',encoding='utf8')
        print(json.dumps({'checks':len(checks),'report':str((QA/'report.json').relative_to(ROOT))}))
    finally:
        if thread_handle: K.ResumeThread(thread_handle);K.CloseHandle(thread_handle)
        if core.poll() is None:
            try: mutate(profile,'stop');core.wait(timeout=10)
            except Exception:
                # Kill only children obtained from this owned core's registry, never by image name.
                try:
                    for p in rpc(profile,'plugins')['plugins']:
                        if p['pid']:
                            h=K.OpenProcess(1,False,p['pid'])
                            if h: K.TerminateProcess(h,78);K.CloseHandle(h)
                finally: core.kill();core.wait(timeout=5)
        log.close()

if __name__=='__main__':main()
