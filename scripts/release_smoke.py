"""Verify an archive, then exercise its binary outside the checkout with an empty environment."""
from __future__ import annotations
import argparse
import hashlib
import http.client
import json
from pathlib import Path, PurePosixPath
import platform
import re
import signal
import socket
import subprocess
import tarfile
import tempfile
import time

MAX_BYTES = 128 * 1024 * 1024

def digest(path):
    with path.open('rb') as file:
        return hashlib.file_digest(file, 'sha256').hexdigest()

def verify_extract(archive, destination):
    expected = (archive.parent / (archive.name + '.sha256')).read_text()
    if expected != f'{digest(archive)}  {archive.name}\n':
        raise ValueError('archive checksum mismatch')
    prefix = archive.name.removesuffix('.tar.gz')
    seen, total = set(), 0
    with tarfile.open(archive, 'r:gz') as tar:
        for index, member in enumerate(tar):
            path = PurePosixPath(member.name)
            if (index >= 64 or path.is_absolute() or '..' in path.parts or
                str(path) != member.name or not path.parts or path.parts[0] != prefix or
                member.name in seen or not (member.isdir() or member.isfile())):
                raise ValueError('unsafe archive member')
            seen.add(member.name)
            total += member.size
            if member.size < 0 or total > MAX_BYTES:
                raise ValueError('archive size limit exceeded')
            target = destination.joinpath(*path.parts)
            if member.isdir():
                target.mkdir(parents=True, exist_ok=True, mode=0o700)
            else:
                target.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
                with target.open('xb') as output, tar.extractfile(member) as source:
                    remaining = member.size
                    while remaining:
                        chunk = source.read(min(remaining, 1024 * 1024))
                        if not chunk:
                            raise ValueError('truncated archive member')
                        output.write(chunk)
                        remaining -= len(chunk)
                target.chmod(0o755 if path.parts == (prefix, 'kiarni') else 0o644)
    root = destination / prefix
    manifest = root / 'CHECKSUMS.sha256'
    entries = {}
    for line in manifest.read_text().splitlines():
        match = re.fullmatch(r'([a-f0-9]{64})  (.+)', line)
        if not match:
            raise ValueError('invalid checksum manifest')
        checksum, name = match.groups()
        path = PurePosixPath(name)
        if path.is_absolute() or '..' in path.parts or str(path) != name or name in entries:
            raise ValueError('unsafe checksum path')
        entries[name] = checksum
    actual = {p.relative_to(root).as_posix() for p in root.rglob('*') if p.is_file() and p != manifest}
    if actual != entries.keys() or any(digest(root / name) != value for name, value in entries.items()):
        raise ValueError('package checksum mismatch')
    required = {'kiarni', 'LICENSE', 'THIRD_PARTY_NOTICES.txt', 'DEPENDENCIES.json',
                'BUILD_INFO.json', 'INSTALL.md', 'toolchain/COPYRIGHT-library.html'}
    if not required <= actual or not any(name.startswith('toolchain/licenses/') for name in actual):
        raise ValueError('incomplete package')
    return root

class Server:
    def __init__(self, binary, work, data):
        self.binary, self.work, self.data = binary, work, data
        self.process = None
    def start(self, options=()):
        with socket.socket() as listener:
            listener.bind(('127.0.0.1', 0))
            self.port = listener.getsockname()[1]
        self.log = tempfile.TemporaryFile()
        self.process = subprocess.Popen([str(self.binary), 'serve', '--data-dir', str(self.data),
            '--bind', f'127.0.0.1:{self.port}', '--heartbeat-ms', '1000', *options], cwd=self.work,
            env={}, stdin=subprocess.DEVNULL, stdout=self.log, stderr=self.log)
        deadline = time.monotonic() + 20
        while time.monotonic() < deadline:
            if self.process.poll() is not None:
                raise RuntimeError('artifact server exited during startup')
            try:
                if self.request('/')[0] == 200:
                    return self
            except OSError:
                pass
            time.sleep(0.05)
        raise RuntimeError('artifact server startup deadline')
    def request(self, path, token=None, method='GET', body=None):
        connection = http.client.HTTPConnection('127.0.0.1', self.port, timeout=5)
        headers = {} if token is None else {'Authorization': 'Bearer ' + token}
        if body is not None:
            headers['Content-Type'] = 'application/merge-patch+json' if method == 'PATCH' else 'application/json'
        try:
            connection.request(method, path, body, headers)
            response = connection.getresponse()
            result = response.read(5 * 1024 * 1024 + 1)
            if len(result) > 5 * 1024 * 1024:
                raise RuntimeError('oversized artifact response')
            return response.status, result
        finally:
            connection.close()
    def stop(self):
        if self.process is not None:
            if self.process.poll() is None:
                self.process.send_signal(signal.SIGTERM)
            try:
                code = self.process.wait(timeout=15)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait(timeout=5)
                raise RuntimeError('artifact shutdown deadline') from None
            finally:
                self.log.close()
                self.process = None
            if code != 0:
                raise RuntimeError('artifact shutdown failed')

def credentials(data):
    return {p.read_text().strip() for p in data.with_name(data.name + '.credentials').glob('*.txt')}

def frame(response):
    result = b''
    while not result.endswith(b'\n\n'):
        line = response.readline(3 * 1024 * 1024 + 1)
        if not line or len(result) + len(line) > 3 * 1024 * 1024:
            raise RuntimeError('invalid artifact SSE frame')
        result += line
    return result

def exercise(root, work, expected_target, allow_dirty=False):
    info = json.loads((root / 'BUILD_INFO.json').read_text())
    if info['target'] != expected_target or (info['dirty'] and not allow_dirty):
        raise ValueError('unexpected artifact provenance')
    hosts = {'x86_64-unknown-linux-gnu': ('Linux', 'x86_64'), 'aarch64-apple-darwin': ('Darwin', 'arm64')}
    if hosts[expected_target] != (platform.system(), platform.machine()):
        raise ValueError('native artifact host required')
    binary = root / 'kiarni'
    def run(*args):
        result = subprocess.run([str(binary), *map(str, args)], cwd=work, env={},
            stdin=subprocess.DEVNULL, capture_output=True, timeout=30)
        if result.returncode != 0:
            raise RuntimeError('artifact CLI command failed: ' + args[0])
        return result.stdout
    if run('--version').decode().strip() != 'kiarni ' + info['version']:
        raise ValueError('artifact version mismatch')
    run()
    data, restored = work / 'data', work / 'restored'
    server = Server(binary, work, data)
    restored_server = Server(binary, work, restored)
    try:
        server.start()
        keys = credentials(data)
        if len(keys) != 1:
            raise RuntimeError('unexpected initial credential count')
        token = next(iter(keys))
        run('healthcheck', '--data-dir', data, '--bind', f'127.0.0.1:{server.port}')
        assert server.request('/v1/status')[0] == 401
        assert server.request('/v1/status', token)[0] == 200
        assert server.request('/v1/collections', token, 'POST', b'{"name":"orders"}')[0] == 201
        document = b'{"_id":"doc","amount":9007199254740993}'
        assert server.request('/v1/collections/orders/documents', token, 'POST', document)[0] == 201
        status, result = server.request('/v1/collections/orders/query', token, 'POST', b'{"from":"orders","limit":1}')
        assert status == 200 and b'9007199254740993' in result
        connection = http.client.HTTPConnection('127.0.0.1', server.port, timeout=5)
        try:
            connection.request('GET', '/v1/collections/orders/changes', headers={'Authorization': 'Bearer ' + token, 'Accept': 'text/event-stream'})
            stream = connection.getresponse()
            assert stream.status == 200 and b'"replay":false' in frame(stream)
            assert server.request('/v1/collections/orders/documents', token, 'POST', b'{"_id":"live"}')[0] == 201
            deadline = time.monotonic() + 10
            while time.monotonic() < deadline:
                event = frame(stream)
                if b'event: change' in event:
                    assert b'"id":"live"' in event
                    break
            else:
                raise RuntimeError('artifact changefeed deadline')
        finally:
            connection.close()
        assert server.request('/v1/collections/orders/documents/live', token, 'PATCH', b'{"updated":true}')[0] == 200
        status, updated = server.request('/v1/collections/orders/documents/live', token)
        assert status == 200 and json.loads(updated)['updated'] is True
        assert server.request('/v1/collections/orders/documents/live', token, 'DELETE')[0] == 204
        assert server.request('/v1/collections/orders/documents/live', token)[0] == 404
        server.stop()
        server.start()
        assert server.request('/v1/collections/orders/documents/doc', token) == (200, document)
        server.stop()
        run('auth', 'rotate', '--data-dir', data)
        rotated = credentials(data) - keys
        assert len(rotated) == 1
        new_token = rotated.pop()
        server.start()
        assert server.request('/v1/status', token)[0] == 401
        assert server.request('/v1/status', new_token)[0] == 200
        server.stop()
        run('auth', 'revoke-all', '--data-dir', data)
        server.start()
        assert server.request('/v1/status', new_token)[0] == 401
        server.stop()
        run('backup', '--data-dir', data, '--output', work / 'backup')
        run('restore', '--from', work / 'backup', '--data-dir', restored)
        restored_server.start()
        restored_keys = credentials(restored)
        assert len(restored_keys) == 1 and not (restored_keys & credentials(data))
        assert restored_server.request('/v1/status', new_token)[0] == 401
        assert restored_server.request('/v1/collections/orders/documents/doc', next(iter(restored_keys))) == (200, document)
    finally:
        try:
            server.stop()
        finally:
            restored_server.stop()
    print(f'Artifact smoke passed: {expected_target}, version {info["version"]}')

if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--artifact-dir', type=Path, required=True)
    parser.add_argument('--expected-target', required=True, choices=['x86_64-unknown-linux-gnu', 'aarch64-apple-darwin'])
    parser.add_argument('--allow-dirty', action='store_true', help='local QA only')
    args = parser.parse_args()
    archives = list(args.artifact_dir.glob('*.tar.gz'))
    if len(archives) != 1:
        raise ValueError('exactly one native archive required')
    with tempfile.TemporaryDirectory(prefix='kiarni-artifact-') as temporary:
        temporary = Path(temporary)
        root = verify_extract(archives[0], temporary)
        work = temporary / 'work'
        work.mkdir(mode=0o700)
        exercise(root, work, args.expected_target, args.allow_dirty)
