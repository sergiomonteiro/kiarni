"""Execute marked API reference commands and CLI config examples against a real binary."""
import argparse
import http.client
import json
import os
from pathlib import Path
import re
import socket
import subprocess
import tempfile
import time
from release_smoke import Server, credentials, frame

ROOT = Path(__file__).resolve().parents[1]

def exercise(binary):
    binary = binary.resolve(strict=True)
    examples = re.findall(r'<!-- verify:api -->\s*```bash\n(.*?)\n```', (ROOT/'docs/API.md').read_text(), re.S)
    if len(examples) != 5:
        raise ValueError('API walkthrough must contain five verified blocks')
    with tempfile.TemporaryDirectory(prefix='kiarni-docs-') as temporary:
        work = Path(temporary)
        data = work/'data'
        server = Server(binary, work, data)
        def start(environment, override=None):
            with socket.socket() as listener:
                listener.bind(('127.0.0.1', 0))
                server.port = listener.getsockname()[1]
            server.log = tempfile.TemporaryFile()
            args = [str(binary),'serve','--config',str(ROOT/'docs/examples/config.json'),
                    '--data-dir',str(data),'--bind',f'127.0.0.1:{server.port}']
            if override:
                args += ['--max-requests', override]
            server.process = subprocess.Popen(args, cwd=work, env=environment, stdin=subprocess.DEVNULL, stdout=server.log, stderr=server.log)
            deadline = time.monotonic()+15
            while time.monotonic()<deadline:
                if server.process.poll() is not None:
                    raise RuntimeError('documented config failed to start')
                try:
                    if server.request('/')[0] == 200:
                        return
                except OSError:
                    pass
                time.sleep(0.05)
            raise RuntimeError('docs server startup deadline')
        def cli(*args, environment=None):
            result = subprocess.run([str(binary), *map(str,args)], env=environment or {}, cwd=work,
                                    capture_output=True, timeout=20)
            if result.returncode:
                raise RuntimeError('documented CLI example failed')
            return result
        connection = None
        try:
            assert cli('--version').stdout.startswith(b'kiarni ')
            assert b'healthcheck' in cli().stdout
            # Exercise all three ranks with the same documented JSON, unchanged.
            token = None
            for environment, override, expected in [({},None,16),({'KIARNI_MAX_REQUESTS':'12'},None,12),({'KIARNI_MAX_REQUESTS':'12'},'8',8)]:
                start(environment, override)
                token = next(iter(credentials(data)))
                status, response = server.request('/v1/status', token)
                assert status == 200 and json.loads(response)['connections']['limit'] == expected
                assert json.loads(response)['connections']['stream_limit'] == 4
                server.stop()
            start({'KIARNI_MAX_REQUESTS':'12'}, '8')
            token_file = next(data.with_name('data.credentials').glob('token-*.txt'))
            cli('healthcheck','--data-dir',data,'--bind',f'127.0.0.1:{server.port}')
            cli('healthcheck','--data-dir',data,'--bind',f'127.0.0.1:{server.port}','--token-file',token_file)
            invalid_root = work/'invalid-config'
            bad = subprocess.run([str(binary),'serve','--config',str(ROOT/'docs/examples/config.json'),
                '--data-dir',str(invalid_root),'--bind',f'127.0.0.1:{server.port}','--max-requests','8'],
                env={'KIARNI_MAX_REQUESTS':'invalid'},cwd=work,capture_output=True,timeout=10)
            assert bad.returncode == 1 and bad.stderr == b'kiarni: invalid_configuration\n'
            assert not invalid_root.exists()
            assert server.request('/v1/status')[0] == 401
            environment = dict(os.environ, token_file=str(token_file), api_base=f'http://127.0.0.1:{server.port}')
            counter = 0
            expected_statuses = [201,201,200,200,200,200,200,200,200,200,200,200,200,204,204]
            for block_index, block in enumerate(examples):
                if block_index == 4:
                    connection = http.client.HTTPConnection('127.0.0.1',server.port,timeout=5)
                    connection.request('GET','/v1/collections/orders/changes',headers={'Authorization':'Bearer '+token,'Accept':'text/event-stream'})
                    stream = connection.getresponse()
                    assert stream.status == 200 and b'"replay":false' in frame(stream)
                for line in block.splitlines():
                    if not line.startswith('kc '):
                        raise ValueError('verified API block must contain one kc invocation per line')
                    headers = work/'headers'
                    output = work/'response'
                    # Only response capture options are added; execute the actual documented command.
                    script = 'source docs/examples/client.sh\n'+line+' --dump-header "$docs_headers" --output "$docs_output"'
                    result = subprocess.run(['bash','-eu','-o','pipefail','-c',script],cwd=ROOT,
                        env=dict(environment,docs_headers=str(headers),docs_output=str(output)),capture_output=True,timeout=15)
                    if result.returncode:
                        raise RuntimeError(f'documented curl command {counter+1} failed')
                    raw_headers = headers.read_text()
                    assert int(raw_headers.splitlines()[0].split()[1]) == expected_statuses[counter]
                    assert 'cache-control: no-store' in raw_headers.lower()
                    match = re.search(r'(?im)^x-request-id: (req_[a-f0-9]{32})\s*$',raw_headers)
                    assert match is not None
                    if counter in [0,1]:
                        assert 'location: /v1/collections/orders' in raw_headers.lower()
                    body = output.read_bytes()
                    if counter in [1,3,4,5,6]:
                        assert b'9007199254740993' in body
                    if counter == 4:
                        query = json.loads(body)
                        assert query['count'] == 1 and set(query['documents'][0]) == {'_id','status','total'}
                    if counter == 6:
                        assert json.loads(body)['status'] == 'shipped' and 'optional' not in json.loads(body)
                    if counter == 7:
                        assert json.loads(body)['count'] == '1' and json.loads(body)['exact']
                    if counter == 11:
                        assert b'kiarni_http_requests_total' in body
                    if block_index == 4 and counter == 13:
                        deadline = time.monotonic()+5
                        while time.monotonic()<deadline:
                            event = frame(stream)
                            if b'event: change' in event:
                                assert b'"type":"delete"' in event and b'9007199254740993' in event and b'"after":null' in event
                                break
                        else:
                            raise RuntimeError('documented delete did not reach SSE')
                        connection.close()
                        connection = None
                    counter += 1
                if block_index == 0:
                    paths = ['/v1/collections?limit=10','/v1/collections/orders/documents/first',
                             '/v1/collections/orders/count','/v1/health/live','/v1/health/ready','/v1/status','/v1/metrics','/v1/errors/recent']
                    for path in paths:
                        assert server.request(path, token, 'HEAD') == (200,b'')
                    status, error = server.request('/v1/collections/orders/documents',token,'POST',(ROOT/'docs/examples/document.json').read_bytes())
                    assert status == 409 and json.loads(error)['error']['code'] == 'duplicate_id'
                    assert re.fullmatch('req_[a-f0-9]{32}',json.loads(error)['request_id'])
            assert counter == len(expected_statuses)
            assert token.encode() not in result.stdout + result.stderr
            print(f'Documentation smoke passed: {counter} published curl commands, 8 HEADs, error envelope, exact JSON, live SSE, CLI/probe and JSON/env/CLI precedence.')
        finally:
            if connection:
                connection.close()
            server.stop()

if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary',required=True,type=Path)
    exercise(parser.parse_args().binary)
