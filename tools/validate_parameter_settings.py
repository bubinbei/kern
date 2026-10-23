"""Parameter/voltage editor acceptance against an owned synthetic core; no COM ports."""
import hashlib
import json
from pathlib import Path
import subprocess
import threading
import time
from datetime import datetime, timezone
from validate_core import rpc, mutate

ROOT = Path(__file__).resolve().parents[1]


def main():
    qa = ROOT / 'target/qa' / ('parameter-settings-' + datetime.now().strftime('%Y%m%d-%H%M%S'))
    profile = qa / 'profile'
    profile.mkdir(parents=True)
    children, logs, errors = [], [], []
    stopping = threading.Event()
    feeder = None

    def spawn(name, *args):
        log = (qa / (name + '.log')).open('wb')
        logs.append(log)
        child = subprocess.Popen([str(ROOT / 'target/release' / (name + '.exe')), *map(str, args)],
                                 stdin=subprocess.PIPE, stdout=log, stderr=log, creationflags=0x08000000)
        children.append(child)
        return child

    def wait(check):
        end = time.monotonic() + 15
        while time.monotonic() < end:
            try:
                if check():
                    return
            except (OSError, ValueError, KeyError):
                pass
            time.sleep(.05)
        raise AssertionError('Synthetic fixture did not progress')

    try:
        spawn('kern-core', 'serve', '--storage', 'redb', '--profile', profile)
        wait(lambda: rpc(profile, 'status')['result'] == 'status')
        config = json.loads((ROOT / 'plugins/calibration.example.json').read_text(encoding='utf8'))
        for parameter in config['parameters']:
            parameter['source'] = 'test'
        assert mutate(profile, 'save_calibration', expected_revision=0, map=config)['result'] == 'durable'
        spawn('kern-processor', '--profile', profile)
        wait(lambda: rpc(profile, 'calibration')['active_revision'] == 1)
        channels = [dict(id=i, name='IM2480 27 ' + name, value_type=kind)
                    for i, (name, kind) in enumerate(zip(['A1', 'A2', 'A3', 'DOL', 'F1', 'F2'],
                                                         ['unsigned'] * 3 + ['signed', 'float', 'float']))]
        source = mutate(profile, 'open_source', id='test', resource='SYNTHETIC', channels=channels,
                        expected_catalog_revision=rpc(profile, 'status')['status']['catalog_revision'])
        session = source['session']

        def feed():
            try:
                sequence = 0
                while not stopping.is_set():
                    sequence += 1
                    batch = dict(source='test', session=session, sequence=sequence,
                                 measured_unix_ms=int(time.time() * 1000),
                                 samples=[dict(channel=i, value=dict(type=channels[i]['value_type'], value=value), quality='good')
                                          for i, value in enumerate([32768, 32000, 28000, 20000, 1.5, 3.1])])
                    assert mutate(profile, 'ingest', batch=batch)['result'] == 'durable'
                    stopping.wait(.1)
            except Exception as error:
                errors.append(str(error))

        feeder = threading.Thread(target=feed)
        feeder.start()
        wait(lambda: len(rpc(profile, 'snapshot')['parameters']) == 6)
        spawn('kern-charts', '--profile', profile)
        wait(lambda: (profile / 'charts-endpoint.json').exists())
        with (qa / 'browser-console.log').open('wb') as log:
            browser = subprocess.run(['node', 'tools/validate_parameter_settings_browser.cjs', str(profile), str(qa)],
                                     cwd=ROOT, stdout=log, stderr=log, timeout=100)
        assert browser.returncode == 0, (qa / 'browser-console.log').read_text(encoding='utf8')
        # Preserve the new editor report; run the previous chart interactions too.
        browser_report = json.loads((qa / 'browser-report.json').read_text())
        with (qa / 'chart-regression-console.log').open('wb') as log:
            regression = subprocess.run(['node', 'tools/validate_prototype_browser.cjs', str(profile), str(qa)],
                                        cwd=ROOT, stdout=log, stderr=log, timeout=60)
        assert regression.returncode == 0, (qa / 'chart-regression-console.log').read_text(encoding='utf8')
        cal = rpc(profile, 'calibration')
        points = cal['map']['parameters'][0]['points']
        assert points == [dict(raw=volts / 12 * 65535, value=value) for volts, value in [(1, 0), (5, 100), (7, 150)]]
        wait(lambda: rpc(profile, 'calibration')['active_revision'] == cal['revision'])
        expected = 100 + (32768 - 5 / 12 * 65535) / (2 / 12 * 65535) * 50
        wait(lambda: abs(next(p for p in rpc(profile, 'snapshot')['parameters'] if p['id'] == 'weight')['value'] - expected) < 1e-9)
        assert not errors, errors
        report = dict(passed=True, date=datetime.now(timezone.utc).isoformat(),
                      binary_sha256=hashlib.sha256((ROOT / 'target/release/kern-charts.exe').read_bytes()).hexdigest(),
                      browser=browser_report,
                      previous_chart_interactions=json.loads((qa / 'browser-report.json').read_text()),
                      independent_checks=['voltage points persisted as exact raw codes',
                                          'interpreter result checked against independent formula',
                                          'continuous synthetic ingestion; no COM access'])
        (qa / 'report.json').write_text(json.dumps(report, indent=2) + '\n', encoding='utf8')
        print(json.dumps(dict(passed=True, report=str(qa / 'report.json'))))
    finally:
        stopping.set()
        if feeder:
            feeder.join(timeout=12)
        for child in reversed(children):
            if child.poll() is None:
                try:
                    if child == children[0]:
                        rpc(profile, 'stop')
                    else:
                        child.stdin.write(b'stop\n')
                        child.stdin.flush()
                    child.wait(timeout=10)
                except (OSError, subprocess.TimeoutExpired):
                    child.kill()
                    child.wait(timeout=5)
        for log in logs:
            log.close()


if __name__ == '__main__':
    main()
