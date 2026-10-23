"""Local HTTP transport regression; owns only a synthetic profile and child host."""
from concurrent.futures import ThreadPoolExecutor
from datetime import datetime, timezone
import argparse
import hashlib
import json
from pathlib import Path
import socket
import subprocess
import time

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, default=ROOT / 'target/release/kern-charts.exe')
    args = parser.parse_args()
    binary = args.binary.resolve()
    qa = ROOT / 'target/qa' / ('charts-transport-' + datetime.now().strftime('%Y%m%d-%H%M%S'))
    qa.mkdir(parents=True)
    # Reserved, non-serving IPC address. No request in this test may reach a real core.
    with socket.socket() as reserved, (qa / 'console.log').open('wb') as log:
        reserved.bind(('127.0.0.1', 0))
        (qa / 'endpoint.json').write_text(json.dumps(dict(
            version=1, address='127.0.0.1:' + str(reserved.getsockname()[1]),
            token='0' * 64, pid=0)), encoding='utf8')
        child = subprocess.Popen([str(binary), '--profile', str(qa)], stdin=subprocess.PIPE,
                                 stdout=log, stderr=log)
        try:
            deadline = time.monotonic() + 5
            while not (qa / 'charts-endpoint.json').exists():
                assert child.poll() is None, 'Host exited before publishing endpoint'
                assert time.monotonic() < deadline, 'Host did not start'
                time.sleep(.02)
            ui = json.loads((qa / 'charts-endpoint.json').read_text())
            host = ui['url'].split('/')[2]
            address, port = host.split(':')

            def request(path='/', fragments=False, extra='', body=b''):
                method = 'POST' if body else 'GET'
                wire = (f'{method} {path} HTTP/1.1\r\nHost: {host}\r\n'
                        f'Content-Length: {len(body)}\r\n{extra}\r\n').encode() + body
                with socket.create_connection((address, int(port)), timeout=5) as client:
                    if fragments:
                        # Deliberately accept before any bytes, then split header and body.
                        time.sleep(.06)
                        for start in range(0, len(wire), 7):
                            client.sendall(wire[start:start + 7])
                            time.sleep(.003)
                    else:
                        client.sendall(wire)
                    response = bytearray()
                    while chunk := client.recv(65536):
                        response.extend(chunk)
                head, payload = bytes(response).split(b'\r\n\r\n', 1)
                headers = dict(line.split(b': ', 1) for line in head.split(b'\r\n')[1:])
                assert int(headers[b'Content-Length']) == len(payload)
                return head.split(b'\r\n')[0], headers, payload

            expected_html = (ROOT / 'crates/charts/web/index.html').read_bytes()
            status, _, payload = request(fragments=True)
            assert status == b'HTTP/1.1 200 OK' and payload == expected_html
            with ThreadPoolExecutor(max_workers=4) as pool:
                replies = list(pool.map(lambda _: request(fragments=True), range(32)))
            assert all(s == b'HTTP/1.1 200 OK' and p == expected_html for s, _, p in replies)
            key = ui['url'].split('#access=')[1]
            status, headers, _ = request('/api/control', fragments=True,
                                         extra=f'X-Kern-Key: {key}\r\n', body=b'{invalid')
            assert status == b'HTTP/1.1 400 Bad Request'
            assert headers[b'Content-Type'] == b'text/plain; charset=utf-8'
            assert request('/api/status')[0] == b'HTTP/1.1 403 Forbidden'
            # An incomplete request is bounded by the worker timeout; host stays usable.
            with socket.create_connection((address, int(port)), timeout=5) as idle:
                idle.sendall(b'GET / HTTP/1.1\r\n')
                started = time.monotonic()
                result = bytearray()
                while chunk := idle.recv(4096):
                    result.extend(chunk)
                assert result.startswith(b'HTTP/1.1 400 Bad Request')
                assert 1 < time.monotonic() - started < 4.5
            assert request()[0] == b'HTTP/1.1 200 OK'
            child.stdin.write(b'stop\n')
            child.stdin.flush()
            assert child.wait(timeout=7) == 0
            report = dict(passed=True, date=datetime.now(timezone.utc).isoformat(),
                          binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),
                          source_sha256=hashlib.sha256((ROOT / 'crates/charts/src/main.rs').read_bytes().replace(b'\r\n', b'\n')).hexdigest(),
                          tests=['delayed fragmented headers', '32 fragmented requests / four workers',
                                 'fragmented malformed body; UTF-8 error', 'unauthenticated API rejected',
                                 'incomplete request timeout; host remains usable', 'cooperative shutdown'])
            (qa / 'report.json').write_text(json.dumps(report, indent=2) + '\n', encoding='utf8')
            print(json.dumps(dict(passed=True, groups=len(report['tests']), report=str(qa / 'report.json'))))
        finally:
            if child.poll() is None:
                child.kill()
                child.wait(timeout=5)


if __name__ == '__main__':
    main()
