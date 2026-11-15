"""Exercise the example in an isolated kind cluster; never uses the user's kubeconfig."""
import argparse
import http.client
import json
from pathlib import Path
import socket
import subprocess
import tempfile
import time
import uuid

ROOT=Path(__file__).resolve().parents[1]
NODE='kindest/node:v1.37.0@sha256:a1ed56cfb0e7b93589bdf97c8cd566405a265939e3620fc4f5de89adff580ae5'

def exercise(image, tools):
    name='kiarni-test-'+uuid.uuid4().hex[:12]
    kind=str((tools/'kind').resolve())
    kubectl=str((tools/'kubectl').resolve())
    forward=None
    with tempfile.TemporaryDirectory(prefix='kiarni-kubernetes-') as temporary:
        config=str(Path(temporary)/'config')
        def run(args, timeout=90, check=True):
            result=subprocess.run(args,capture_output=True,text=True,timeout=timeout)
            if check and result.returncode:
                # Tool errors never include credential values: fixture passes paths only.
                raise RuntimeError('Kubernetes fixture failed: '+args[0]+'\n'+result.stderr[-3000:])
            return result
        def kube(*args, timeout=90):
            return run([kubectl,'--kubeconfig',config,*args],timeout)
        def pod():
            items=json.loads(kube('get','pods','-l','app=kiarni','-o','json').stdout)['items']
            available=[item for item in items if not item['metadata'].get('deletionTimestamp')]
            if len(available)!=1:
                raise RuntimeError('one Kiarni pod required')
            return available[0]
        def keys(identifier):
            text=kube('exec',identifier,'-c','kiarni','--','/bin/sh','-ec','cat /var/lib/kiarni/data.credentials/token-*.txt').stdout
            assert len(text)==91
            return text
        def port_forward(identifier):
            nonlocal forward
            if forward:
                forward.terminate()
                forward.wait(timeout=5)
            with socket.socket() as listener:
                listener.bind(('127.0.0.1',0))
                port=listener.getsockname()[1]
            forward=subprocess.Popen([kubectl,'--kubeconfig',config,'port-forward','--address','127.0.0.1','pod/'+identifier,f'{port}:8711'],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
            deadline=time.monotonic()+15
            while time.monotonic()<deadline:
                if forward.poll() is not None:
                    raise RuntimeError('port-forward exited')
                try:
                    with socket.create_connection(('127.0.0.1',port),timeout=0.2):
                        return port
                except OSError:
                    time.sleep(0.1)
            raise RuntimeError('port-forward startup deadline')
        def request(port,path,token=None,method='GET',body=None):
            connection=http.client.HTTPConnection('127.0.0.1',port,timeout=5)
            headers={'Host':'127.0.0.1:8711'}
            if token:
                headers['Authorization']='Bearer '+token
            if body:
                headers['Content-Type']='application/json'
            try:
                connection.request(method,path,body,headers)
                response=connection.getresponse()
                data=response.read(1024*1024+1)
                assert len(data)<=1024*1024
                return response.status,data
            finally:
                connection.close()
        try:
            run([kind,'create','cluster','--name',name,'--kubeconfig',config,'--image',NODE,'--wait','120s'],timeout=300)
            Path(config).chmod(0o600)
            run([kind,'load','docker-image',image,'--name',name],timeout=120)
            # Override only the local test image; the manifest itself is submitted to API validation.
            kube('apply','--dry-run=server','-f',str(ROOT/'deploy/kubernetes/kiarni.yaml'))
            kube('apply','-f',str(ROOT/'deploy/kubernetes/kiarni.yaml'))
            if image!='kiarni:development':
                kube('set','image','deployment/kiarni',f'kiarni={image}',f'prepare-volume={image}')
            kube('rollout','status','deployment/kiarni','--timeout=180s',timeout=190)
            first=pod()
            identifier=first['metadata']['name']
            assert first['spec']['containers'][0]['securityContext']['runAsNonRoot']
            assert first['spec']['containers'][0]['readinessProbe']['exec']['command']==['/usr/local/bin/kiarni','healthcheck']
            assert first['spec']['volumes'][0]['persistentVolumeClaim']['claimName']=='kiarni-state'
            token=keys(identifier)
            port=port_forward(identifier)
            assert request(port,'/v1/status')[0]==401
            assert request(port,'/v1/collections',token,'POST',b'{"name":"orders"}')[0]==201
            document=b'{"_id":"persistent","amount":9007199254740993}'
            assert request(port,'/v1/collections/orders/documents',token,'POST',document)[0]==201
            kube('rollout','restart','deployment/kiarni')
            kube('rollout','status','deployment/kiarni','--timeout=180s',timeout=190)
            second=pod()
            assert second['metadata']['uid']!=first['metadata']['uid']
            identifier=second['metadata']['name']
            assert keys(identifier)==token
            port=port_forward(identifier)
            assert request(port,'/v1/collections/orders/documents/persistent',token)==(200,document)
            assert json.loads(kube('get','pvc','kiarni-state','-o','json').stdout)['status']['phase']=='Bound'
            assert not json.loads(kube('get','services','-l','app=kiarni','-o','json').stdout)['items']
            logs=kube('logs',identifier,'-c','kiarni').stdout
            assert token not in logs
            print('Kubernetes smoke passed: API validation, non-root, authenticated probes, PVC, port-forward, pod replacement and persisted document/credential.')
        finally:
            if forward:
                forward.terminate()
                forward.wait(timeout=5)
            run([kind,'delete','cluster','--name',name,'--kubeconfig',config],timeout=90,check=False)

if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--image',required=True)
    parser.add_argument('--tools',required=True,type=Path)
    args=parser.parse_args()
    exercise(args.image,args.tools)
