"""Real Docker/Compose lifecycle test, isolated by a unique project and volume."""
import argparse
import json
import os
from pathlib import Path
import socket
import subprocess
import time
import uuid
from release_smoke import Server

ROOT = Path(__file__).resolve().parents[1]

def exercise(image):
    project = 'kiarni-test-' + uuid.uuid4().hex[:12]
    restored_name = project + '-restored'
    with socket.socket() as listener:
        listener.bind(('127.0.0.1', 0))
        port = listener.getsockname()[1]
    environment = dict(os.environ, KIARNI_IMAGE=image, KIARNI_BIND=f'127.0.0.1:{port}')
    base = ['docker', 'compose', '-f', str(ROOT / 'deploy/docker/compose.yaml'), '-p', project]
    def execute(args, check=True):
        result = subprocess.run(args, env=environment, capture_output=True, text=True, timeout=90)
        if check and result.returncode:
            raise RuntimeError('container fixture command failed: ' + args[0] + ' ' + args[1])
        return result
    def compose(*args, check=True):
        return execute(base + list(args), check)
    def inspect(name):
        return json.loads(execute(['docker', 'inspect', name]).stdout)[0]
    def until(predicate, description):
        deadline = time.monotonic() + 45
        while time.monotonic() < deadline:
            if predicate():
                return
            time.sleep(0.2)
        raise RuntimeError('container deadline: ' + description)
    def healthy(name):
        return inspect(name)['State'].get('Health', {}).get('Status') == 'healthy'
    def keys(name, directory='/var/lib/kiarni/data.credentials'):
        # Captured only in memory; never print credentials or pass their values as argv/env.
        text = execute(['docker','exec',name,'/bin/sh','-ec',f'cat {directory}/token-*.txt']).stdout
        if len(text) % 91:
            raise RuntimeError('unexpected credential delivery')
        return {text[n:n+91] for n in range(0,len(text),91)}
    api = Server(None, None, None)
    api.port = port
    try:
        config = json.loads(compose('config', '--format', 'json').stdout)
        service = config['services']['kiarni']
        assert service['network_mode'] == 'host' and not service.get('ports')
        assert service['read_only'] and service['user'] == '10001:10001'
        compose('up', '-d', '--no-build')
        identifier = compose('ps', '-q', 'kiarni').stdout.strip()
        until(lambda: healthy(identifier), 'initial health')
        details = inspect(identifier)
        assert details['Config']['User'] == '10001:10001'
        assert details['HostConfig']['ReadonlyRootfs']
        assert details['HostConfig']['NetworkMode'] == 'host'
        initial_keys = keys(identifier)
        assert len(initial_keys) == 1
        token = next(iter(initial_keys))
        assert api.request('/v1/health/ready')[0] == 401
        assert api.request('/v1/collections', token, 'POST', b'{"name":"orders"}')[0] == 201
        document = b'{"_id":"first","amount":9007199254740993}'
        assert api.request('/v1/collections/orders/documents', token, 'POST', document)[0] == 201
        execute(['docker','exec',identifier,'/bin/sh','-ec',
            'mkdir -m 0700 /var/lib/kiarni/backups /var/lib/kiarni/restores'])
        # Lock protection must also apply between the server and a one-off container.
        assert compose('run','--rm','--no-deps','kiarni','backup','--output','/var/lib/kiarni/backups/busy',check=False).returncode != 0
        compose('restart','kiarni')
        until(lambda: healthy(identifier), 'restart health')
        assert keys(identifier) == initial_keys
        assert api.request('/v1/collections/orders/documents/first', token) == (200,document)
        compose('down') # Deliberately preserve this test project's named volume.
        compose('up','-d','--no-build')
        identifier = compose('ps','-q','kiarni').stdout.strip()
        until(lambda: healthy(identifier), 'recreated health')
        assert keys(identifier) == initial_keys
        assert api.request('/v1/collections/orders/documents/first', token) == (200,document)
        compose('stop','kiarni')
        assert inspect(identifier)['State']['ExitCode'] == 0
        compose('run','--rm','--no-deps','kiarni','auth','rotate')
        compose('start','kiarni')
        until(lambda: healthy(identifier), 'rotated health')
        new_keys = keys(identifier) - initial_keys
        assert len(new_keys) == 1
        rotated = next(iter(new_keys))
        assert api.request('/v1/status', token)[0] == 401
        assert api.request('/v1/status', rotated)[0] == 200
        compose('stop','kiarni')
        compose('run','--rm','--no-deps','kiarni','auth','revoke-all')
        compose('start','kiarni')
        assert execute(['docker','exec',identifier,'kiarni','healthcheck'],check=False).returncode != 0
        until(lambda: api.request('/')[0] == 200, 'revoked server startup')
        assert api.request('/v1/status', rotated)[0] == 401
        compose('stop','kiarni')
        compose('run','--rm','--no-deps','kiarni','backup','--output','/var/lib/kiarni/backups/first')
        compose('run','--rm','--no-deps','kiarni','restore','--from','/var/lib/kiarni/backups/first','--data-dir','/var/lib/kiarni/restores/data')
        compose('run','-d','--no-deps','--name',restored_name,'-e','KIARNI_DATA_DIR=/var/lib/kiarni/restores/data','kiarni','serve')
        until(lambda: healthy(restored_name), 'restored health')
        restored_keys = keys(restored_name, '/var/lib/kiarni/restores/data.credentials')
        assert len(restored_keys) == 1 and not (restored_keys & (initial_keys | new_keys))
        assert api.request('/v1/status', rotated)[0] == 401
        assert api.request('/v1/collections/orders/documents/first', next(iter(restored_keys))) == (200,document)
        for name in [identifier,restored_name]:
            output = execute(['docker','logs',name])
            logs = output.stdout + output.stderr + json.dumps(inspect(name)['State'].get('Health',{}))
            assert all(value not in logs for value in initial_keys | new_keys | restored_keys)
        print('Container smoke passed: non-root, health/auth, restart/recreate, persistence, rotation/revocation, backup/restore.')
    finally:
        execute(['docker','rm','-f',restored_name],check=False)
        compose('down','--volumes','--remove-orphans',check=False)

if __name__ == '__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--image',required=True)
    exercise(parser.parse_args().image)
