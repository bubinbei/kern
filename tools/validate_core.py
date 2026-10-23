"""Independent process/journal acceptance. Own synthetic profiles under target/qa only.

Build first: cargo build --workspace --release
Run: python tools/validate_core.py
No application modules or Rust journal reader are used to verify file contents.
"""
from __future__ import annotations
import ctypes
from datetime import datetime, timezone, timedelta
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import socket
import struct
import subprocess
import time
import zlib

ROOT = Path(__file__).resolve().parents[1]
SUFFIX = '.exe' if os.name == 'nt' else ''
CORE = ROOT / 'target/release' / ('kern-core' + SUFFIX)
DEMO = ROOT / 'target/release' / ('kern-demo' + SUFFIX)
QA = ROOT / 'target/qa' / ('core-' + datetime.now(timezone(timedelta(hours=4))).strftime('%Y%m%d-%H%M%S') + '-' + str(os.getpid()))
OWNED = []


def exact(stream, n):
    result = bytearray()
    while len(result) < n:
        b = stream.recv(n - len(result))
        if not b:
            raise ConnectionError('EOF before reply was complete')
        result.extend(b)
    return bytes(result)


def rpc(profile, command, **fields):
    endpoint = json.loads((profile / 'endpoint.json').read_text())
    address, port = endpoint['address'].rsplit(':', 1)
    b = json.dumps({'version': 1, 'token': endpoint['token'], 'request': {'command': command, **fields}}, separators=(',', ':')).encode()
    with socket.create_connection((address, int(port)), timeout=5) as s:
        s.sendall(struct.pack('<I', len(b)) + b)
        n, = struct.unpack('<I', exact(s, 4))
        assert 0 < n <= 512 * 1024
        return json.loads(exact(s, n))


def mutate(profile, command, **fields):
    deadline = time.monotonic() + 10
    while True:
        reply = rpc(profile, command, **fields)
        if reply.get('code') != 'busy':
            return reply
        assert time.monotonic() < deadline, 'Writer did not progress'
        time.sleep(.01)


def cli(*args, ok=True):
    p = subprocess.run([str(CORE), *map(str, args)], cwd=ROOT, capture_output=True, text=True, timeout=20)
    assert (p.returncode == 0) == ok, (args, p.returncode, p.stdout, p.stderr)
    return p


def start(profile):
    profile.mkdir(parents=True, exist_ok=True)
    log = open(profile / 'process.log', 'ab', buffering=0)
    proc = subprocess.Popen([str(CORE), 'serve', '--profile', str(profile)], cwd=ROOT, stdout=log, stderr=log)
    OWNED.append((proc, log, profile))
    deadline = time.monotonic() + 10
    while time.monotonic() < deadline:
        if proc.poll() is not None:
            raise AssertionError((proc.returncode, (profile / 'process.log').read_text()))
        try:
            if rpc(profile, 'status').get('result') == 'status':
                return proc
        except (OSError, ValueError, KeyError):
            pass
        time.sleep(.02)
    raise AssertionError('Core did not become ready')


def stop(profile, proc, disk_error=False):
    reply = rpc(profile, 'stop')
    assert reply['result'] == ('error' if disk_error else 'durable'), reply
    proc.wait(timeout=8)
    assert proc.returncode == 0


def demo(profile, id, channels, count, seed, period=5):
    log = open(QA / (id + '-demo.log'), 'ab', buffering=0)
    proc = subprocess.Popen([str(DEMO), '--profile', str(profile), '--id', id, '--channels', str(channels),
                             '--count', str(count), '--seed', str(seed), '--period-ms', str(period)], cwd=ROOT, stdout=log, stderr=log)
    OWNED.append((proc, log, None))
    return proc


def journal(profile):
    """Own decoder: 16-byte segment header; length/CRC + sequence/JSON frames."""
    events = []
    pending = None
    for number, path in enumerate(sorted((profile / 'journal').glob('segment-*.kjn')), 1):
        b = path.read_bytes()
        assert len(b) >= 16 and b[:8] == b'KERNJ001'
        assert struct.unpack_from('<Q', b, 8)[0] == number
        offset = 16
        first = True
        while offset < len(b):
            begin = offset
            if len(b) - offset < 8:
                pending = {'segment': number, 'valid_bytes': begin, 'file_bytes': len(b)}
                break
            n, crc = struct.unpack_from('<II', b, offset)
            assert 9 <= n <= 65536
            offset += 8
            if len(b) - offset < n:
                pending = {'segment': number, 'valid_bytes': begin, 'file_bytes': len(b)}
                break
            payload = b[offset:offset+n]
            assert zlib.crc32(payload) == crc
            seq, = struct.unpack_from('<Q', payload)
            assert seq == len(events) + 1
            event = json.loads(payload[8:])
            if first and pending:
                assert event == {'event': 'tail_recovered', 'boundary': pending}
                pending = None
            first = False
            events.append(event)
            offset += n
    return events, pending


def directory_hash(profile):
    return {p.name: hashlib.sha256(p.read_bytes()).hexdigest() for p in (profile / 'journal').glob('*.kjn')}


def open_source(profile, id, n):
    revision = rpc(profile, 'status')['status']['catalog_revision']
    reply = mutate(profile, 'open_source', id=id, resource='demo:' + id, expected_catalog_revision=revision,
                   channels=[{'id': i, 'name': f'Raw {i}', 'value_type': 'unsigned'} for i in range(n)])
    assert reply['result'] == 'opened', reply
    return reply


def raw_batch(id, session, sequence, values):
    return {'source': id, 'session': session, 'sequence': sequence, 'measured_unix_ms': 1_700_000_000_000 + sequence,
            'samples': [{'channel': k, 'quality': 'good', 'value': {'type': 'unsigned', 'value': v}} for k, v in values]}


def windows_metrics(proc):
    if os.name != 'nt':
        return None
    from ctypes import wintypes as w
    class Memory(ctypes.Structure):
        _fields_ = [('cb', w.DWORD), ('PageFaultCount', w.DWORD)] + [(n, ctypes.c_size_t) for n in
                    ['PeakWorkingSetSize','WorkingSetSize','QuotaPeakPagedPoolUsage','QuotaPagedPoolUsage',
                     'QuotaPeakNonPagedPoolUsage','QuotaNonPagedPoolUsage','PagefileUsage','PeakPagefileUsage','PrivateUsage']]
    kernel = ctypes.WinDLL('kernel32', use_last_error=True)
    psapi = ctypes.WinDLL('psapi', use_last_error=True)
    kernel.GetProcessTimes.argtypes = [w.HANDLE] + [ctypes.POINTER(w.FILETIME)] * 4
    psapi.GetProcessMemoryInfo.argtypes = [w.HANDLE, ctypes.c_void_p, w.DWORD]
    times = [w.FILETIME() for _ in range(4)]
    assert kernel.GetProcessTimes(int(proc._handle), *[ctypes.byref(t) for t in times])
    mem = Memory(); mem.cb = ctypes.sizeof(mem)
    assert psapi.GetProcessMemoryInfo(int(proc._handle), ctypes.byref(mem), mem.cb)
    cpu = sum((t.dwHighDateTime << 32) + t.dwLowDateTime for t in times[2:]) / 10_000_000
    return {'cpu_seconds': cpu, 'working_set_bytes': mem.WorkingSetSize, 'private_bytes': mem.PrivateUsage}


def main():
    assert CORE.is_file() and DEMO.is_file(), 'Build release binaries first'
    QA.mkdir(parents=True)
    profile = QA / 'main-profile'
    proc = start(profile)
    cli('serve', '--profile', profile, ok=False)  # second owner cannot start
    a = demo(profile, 'a', 18, 12, 1000)
    b = demo(profile, 'b', 20, 12, 2000)
    assert a.wait(timeout=15) == b.wait(timeout=15) == 0
    status = rpc(profile, 'status')['status']
    assert status['raw_batches'] == 24 and status['raw_samples'] == 456, status
    before = {s['id']: s for s in rpc(profile, 'snapshot')['sources']}
    slot_b = before['b']['slot']
    opened = open_source(profile, 'a', 24)
    session = opened['session']
    assert opened['slot'] == before['a']['slot']
    assert next(s for s in rpc(profile, 'snapshot')['sources'] if s['id']=='b')['slot'] == slot_b
    assert mutate(profile, 'set_settings', expected_revision=0, settings={'stale_after_ms': 100})['result']=='settings_saved'
    assert mutate(profile, 'set_settings', expected_revision=0, settings={'stale_after_ms': 1000})['code']=='revision_conflict'
    packet = raw_batch('a', session, 1, [(0, 7)])
    assert mutate(profile, 'ingest', batch=packet)['result']=='durable'
    assert mutate(profile, 'ingest', batch=packet)['duplicate']
    packet['samples'][0]['value']['value'] = 8
    assert mutate(profile, 'ingest', batch=packet)['code']=='sequence_conflict'
    time.sleep(.15)
    view = next(s for s in rpc(profile, 'snapshot')['sources'] if s['id']=='a')
    assert view['channels'][0]['quality']=='stale'
    assert mutate(profile, 'ingest', batch=raw_batch('a',session,2,[(0,7)]))['result']=='durable'
    assert mutate(profile, 'ingest', batch=raw_batch('a',session,3,[(1,2**64-1)]))['result']=='durable'
    assert mutate(profile, 'ingest', batch=raw_batch('a',session,4,[(99,4)]))['code']=='invalid'
    saved = rpc(profile, 'status')['status']; assert saved['raw_samples']==459 and saved['raw_batches']==27
    endpoint = json.loads((profile / 'endpoint.json').read_text())
    host, port = endpoint['address'].rsplit(':',1)
    with socket.create_connection((host,int(port)), timeout=2) as s:
        s.sendall(struct.pack('<I', 2**32-1));n,=struct.unpack('<I',exact(s,4));assert json.loads(exact(s,n))['code']=='invalid_frame'
    # Core is completely idle here; sample its own counters, not whole-machine load.
    initial = windows_metrics(proc); t = time.monotonic(); time.sleep(5); final = windows_metrics(proc)
    idle = None if initial is None else {'seconds':time.monotonic()-t,'one_cpu_percent':100*(final['cpu_seconds']-initial['cpu_seconds'])/(time.monotonic()-t),
                                         'working_set_bytes':final['working_set_bytes'],'private_bytes':final['private_bytes']}
    stop(profile,proc)
    cli('check','--profile',profile)
    events, tail = journal(profile);assert tail is None
    batches = [e['batch'] for e in events if e['event']=='batch_stored']
    assert len(batches)==27 and sum(len(b['samples']) for b in batches)==459
    for batch in batches:
        if batch['session']==1:
            seed=1000 if batch['source']=='a' else 2000
            for sample in batch['samples']:assert sample['value']['value']==seed+100*batch['sequence']+sample['channel']
    assert batches[-1]['samples'][0]['value']['value']==2**64-1
    proc=start(profile)
    restored=rpc(profile,'status')['status'];assert restored['raw_samples']==459 and restored['boot']==2
    restored_source=next(s for s in rpc(profile,'snapshot')['sources'] if s['id']=='a')
    assert restored_source['channels'][1]['quality']=='recovered' and not restored_source['online']
    # A forced process death never touches other user processes.
    crash_source=demo(profile,'crash-source',6,0,3000,2)
    time.sleep(.3);last_observed=rpc(profile,'status')['status']['event_sequence'];proc.kill();proc.wait(timeout=5)
    crash_source.wait(timeout=10)
    proc=start(profile);assert rpc(profile,'status')['status']['event_sequence']>=last_observed
    stop(profile,proc)
    events,_=journal(profile)
    # Work on copies of our own generated profile only.
    corrupt=QA/'corrupt';(corrupt/'journal').mkdir(parents=True)
    for p in (profile/'journal').glob('*.kjn'):shutil.copyfile(p,corrupt/'journal'/p.name)
    last=sorted((corrupt/'journal').glob('*.kjn'))[-1];data=bytearray(last.read_bytes());data[-1]^=1;last.write_bytes(data)
    original=directory_hash(corrupt)
    cli('check','--profile',corrupt,ok=False);cli('serve','--profile',corrupt,ok=False)
    recovered=QA/'recovered';cli('recover','--profile',corrupt,'--output',recovered)
    cli('check','--profile',recovered);assert directory_hash(corrupt)==original
    assert len(journal(recovered)[0])==len(events)-1
    torn=QA/'torn';(torn/'journal').mkdir(parents=True)
    for p in (profile/'journal').glob('*.kjn'):shutil.copyfile(p,torn/'journal'/p.name)
    last=sorted((torn/'journal').glob('*.kjn'))[-1];last.write_bytes(last.read_bytes()[:-1]);original=directory_hash(torn)
    proc=start(torn);stop(torn,proc);cli('check','--profile',torn)
    assert all(hashlib.sha256((torn/'journal'/p).read_bytes()).hexdigest()==h for p,h in original.items())
    proc=start(torn);stop(torn,proc);journal(torn)  # recovery boundary survives another restart
    # Fail creation of the next segment; status must remain responsive while writes stop.
    fault=QA/'disk-fault';proc=start(fault);opened=open_source(fault,'fault',100)
    blocker=fault/'journal/segment-00000002.kjn';blocker.mkdir()
    failed=False
    for seq in range(1,1500):
        reply=mutate(fault,'ingest',batch=raw_batch('fault',opened['session'],seq,[(i,seq*100+i) for i in range(100)]))
        if reply.get('code')=='disk_error':failed=True;break
        assert reply['result']=='durable',reply
    assert failed
    faulty_status=rpc(fault,'status')['status'];assert faulty_status['disk_error'] and not faulty_status['pending_write']
    stop(fault,proc,disk_error=True);blocker.rmdir();cli('check','--profile',fault)
    source_hash=hashlib.sha256()
    for p in sorted([ROOT/'Cargo.toml',ROOT/'Cargo.lock',*ROOT.glob('crates/**/*.toml'),*ROOT.glob('crates/**/*.rs')]):
        source_hash.update(p.relative_to(ROOT).as_posix().encode());source_hash.update(b'\0');source_hash.update(p.read_bytes().replace(b'\r\n',b'\n'))
    report={'platform':platform.platform(),'core_version':'0.1.0','rust_source_sha256':source_hash.hexdigest(),'checks':['two_source_processes','stable_slots','typed_uint64',
            'deduplication','sequence_conflict','invalid_batch','staleness_without_inputs','settings_revision','frame_bound',
            'exclusive_profile','independent_crc_reader','restart_recovered_quality','forced_core_death','crc_corruption_refusal',
            'recovery_copy_preserves_original','truncated_tail_preserves_original','recovery_boundary_restart','disk_failure_status_responsive'],
            'deterministic_batches':27,'deterministic_samples':459,'idle':idle,
            'release_bytes':{'kern-core':CORE.stat().st_size,'kern-demo':DEMO.stat().st_size},
            'not_checked':['Windows10_hardware','24_to_72_hours','GPU_counters','sync_failure_after_partial_write','power_loss_storage_firmware','interpreter','shell']}
    (QA/'report.json').write_text(json.dumps(report,ensure_ascii=False,indent=2),encoding='utf-8')
    print(json.dumps(report,ensure_ascii=True,indent=2));print('Evidence:',QA.relative_to(ROOT))


if __name__=='__main__':
    try:
        main()
    finally:
        for proc, log, profile in reversed(OWNED):
            if proc.poll() is None:
                if profile:
                    try:rpc(profile,'stop');proc.wait(timeout=2)
                    except (OSError,ValueError,subprocess.TimeoutExpired):pass
                if proc.poll() is None:proc.kill();proc.wait(timeout=5)
            log.close()
