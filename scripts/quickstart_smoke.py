"""Execute the quickstart shipped inside a verified native archive, outside the checkout."""
import argparse
from datetime import datetime, timezone
import http.client
import json
import os
from pathlib import Path
import platform
import re
import shutil
import subprocess
import tempfile
import time
from release_smoke import Server, credentials, digest, frame, verify_extract

def exercise(artifacts, target, report, allow_dirty=False):
    start=time.monotonic()
    archives=list(artifacts.glob('*.tar.gz'))
    if len(archives)!=1:
        raise ValueError('one native archive required')
    hosts={'x86_64-unknown-linux-gnu':('Linux','x86_64'),'aarch64-apple-darwin':('Darwin','arm64')}
    if hosts.get(target)!=(platform.system(),platform.machine()):
        raise ValueError('native host required')
    with tempfile.TemporaryDirectory(prefix='kiarni-quickstart-') as temporary:
        temporary=Path(temporary)
        package=verify_extract(archives[0],temporary)
        info=json.loads((package/'BUILD_INFO.json').read_text())
        if os.environ.get('GITHUB_ACTIONS')=='true' and info['commit']!=os.environ.get('GITHUB_SHA'):
            raise ValueError('CI archive must match workflow commit')
        if info['target']!=target or (info['dirty'] and (not allow_dirty or os.environ.get('GITHUB_ACTIONS')=='true')):
            raise ValueError('unexpected native artifact provenance')
        examples=re.findall(r'<!-- verify:quickstart -->\s*```bash\n(.*?)\n```',(package/'QUICKSTART.md').read_text(),re.S)
        if len(examples)!=4 or any('\n' in line or not line.startswith('kc ') for line in examples):
            raise ValueError('four packaged quickstart commands required')
        work=temporary/'installation'
        work.mkdir(mode=0o700)
        install=work/'bin'
        install.mkdir(mode=0o700)
        binary=install/'kiarni'
        shutil.copyfile(package/'kiarni',binary)
        binary.chmod(0o755)
        # Client tool PATH is explicit and contains no Rust/Cargo/Node/toolchain.
        shell=shutil.which('bash')
        for tool in ['curl','cat']:
            location=shutil.which(tool)
            if not location:
                raise ValueError('quickstart client tool missing: '+tool)
            (install/tool).symlink_to(location)
        if not shell:
            raise ValueError('Bash client required')
        server=Server(binary,work,work/'data')
        connection=None
        milestones={}
        try:
            version=subprocess.run([str(binary),'--version'],env={},cwd=work,capture_output=True,timeout=10,check=True).stdout.decode().strip()
            if version!='kiarni '+info['version']:
                raise ValueError('installed version mismatch')
            server.start()
            token=next(iter(credentials(server.data)))
            token_file=next(server.data.with_name('data.credentials').glob('token-*.txt'))
            environment={'PATH':str(install),'api_base':f'http://127.0.0.1:{server.port}',
                'token_file':str(token_file),'quickstart_package':str(package),
                'quickstart_headers':str(work/'headers'),'quickstart_output':str(work/'response')}
            for index,line in enumerate(examples):
                script='source "$quickstart_package/examples/client.sh"\n'+line+' --dump-header "$quickstart_headers" --output "$quickstart_output"'
                result=subprocess.run([shell,'-eu','-o','pipefail','-c',script],env=environment,cwd=package,capture_output=True,timeout=15)
                if result.returncode:
                    raise RuntimeError('packaged quickstart command failed')
                header=(work/'headers').read_text()
                body=(work/'response').read_bytes()
                if int(header.splitlines()[0].split()[1])!=[201,201,200,200][index]:
                    raise RuntimeError('packaged quickstart status mismatch')
                if index==0:
                    connection=http.client.HTTPConnection('127.0.0.1',server.port,timeout=5)
                    connection.request('GET','/v1/collections/orders/changes',headers={'Authorization':'Bearer '+token,'Accept':'text/event-stream'})
                    stream=connection.getresponse()
                    if stream.status!=200 or b'"snapshot":false,"replay":false' not in frame(stream):
                        raise RuntimeError('packaged quickstart did not subscribe')
                if index==1:
                    milestones['first_document_seconds']=round(time.monotonic()-start,3)
                    if b'9007199254740993' not in body:
                        raise RuntimeError('quickstart precision mismatch')
                if index in [1,3]:
                    deadline=time.monotonic()+8
                    while time.monotonic()<deadline:
                        event=frame(stream)
                        if b'event: change' in event:
                            if b'9007199254740993' not in event or (index==1 and b'"type":"insert"' not in event) or (index==3 and b'"type":"update"' not in event):
                                raise RuntimeError('unexpected quickstart event')
                            break
                    else:
                        raise RuntimeError('quickstart event deadline')
                    if index==1:
                        milestones['first_change_seconds']=round(time.monotonic()-start,3)
                if index==2:
                    value=json.loads(body)
                    if value['count']!=1 or value['documents'][0]['total']!=9007199254740993:
                        raise RuntimeError('quickstart query mismatch')
                    milestones['first_query_seconds']=round(time.monotonic()-start,3)
                if time.monotonic()-start>600:
                    raise RuntimeError('quickstart exceeded ten-minute budget')
            stream.close();connection.close();connection=None
            server.stop()
            server.start()
            status,document=server.request('/v1/collections/orders/documents/first',token)
            if status!=200 or json.loads(document)['status']!='shipped' or next(iter(credentials(server.data)))!=token:
                raise RuntimeError('quickstart restart persistence mismatch')
            value={'schema':1,'recorded_at_utc':datetime.now(timezone.utc).isoformat(),'status':'passed','source_commit':info['commit'],'dirty':info['dirty'],'target':target,
                'version':info['version'],'rust_version':info['rust_version'],
                'archive_sha256':digest(archives[0]),
                'binary_sha256':digest(binary),
                'environment':{'system':platform.system(),'release':platform.release(),'machine':platform.machine(),'libc':platform.libc_ver()},
                'timings':milestones,'total_seconds':round(time.monotonic()-start,3),'budget_seconds':600,
                'executed_packaged_commands':4,'runtime_environment':'empty; client PATH contains only kiarni/curl/cat',
                'restart_preserved_document_and_credential':True,
                'scope':'automated local-archive installation; excludes download, tool installation, quarantine and human reading time'}
            report.parent.mkdir(parents=True,exist_ok=True)
            with report.open('x') as output:
                json.dump(value,output,indent=2,sort_keys=True);output.write('\n')
            print('Packaged quickstart passed: '+json.dumps(milestones),flush=True)
            return value
        finally:
            if connection:
                connection.close()
            server.stop()

if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--artifact-dir',type=Path,required=True)
    parser.add_argument('--expected-target',required=True)
    parser.add_argument('--report',type=Path,required=True)
    parser.add_argument('--allow-dirty',action='store_true',help='local QA only')
    args=parser.parse_args()
    exercise(args.artifact_dir,args.expected_target,args.report,args.allow_dirty)
