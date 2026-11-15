"""Bounded single-node observation using a distributed binary and a real external Prometheus."""
import argparse
from datetime import datetime, timezone
import hashlib
import http.client
import json
import os
from pathlib import Path
import platform
import re
import shutil
import socket
import statistics
import subprocess
import tempfile
import time
from urllib.parse import urlencode
import uuid
from release_smoke import Server, credentials, frame, verify_extract

PROMETHEUS = 'prom/prometheus:v3.16.0@sha256:e04987641b57704f408e31f6373ba4138ecf27cb4a165d24aabb6c38fddd0f06'

def metric(text, name, **labels):
    for line in text.splitlines():
        match = re.fullmatch(re.escape(name)+r'(?:\{([^}]*)\})? ([0-9.eE+\-]+)',line)
        if match and dict(re.findall(r'(\w+)="([^"]*)"',match[1] or '')) == labels:
            return float(match[2])
    raise RuntimeError('required metric not found: '+name)

def exercise(artifacts, duration, report):
    if platform.system() != 'Linux' or not 20 <= duration <= 3600:
        raise ValueError('Linux and duration 20..3600 seconds required')
    archives = list(artifacts.glob('*.tar.gz'))
    if len(archives) != 1:
        raise ValueError('one Linux native archive required')
    name = 'kiarni-observe-'+uuid.uuid4().hex[:12]
    collector_started = False
    slow = fast = None
    server = None
    def docker(*args, check=True, timeout=60):
        result = subprocess.run(['docker',*args],capture_output=True,text=True,timeout=timeout)
        if check and result.returncode:
            raise RuntimeError('external collector command failed: '+args[0])
        return result
    with tempfile.TemporaryDirectory(prefix='kiarni-observe-') as temporary:
        temporary = Path(temporary)
        package = verify_extract(archives[0],temporary)
        info = json.loads((package/'BUILD_INFO.json').read_text())
        if os.environ.get('GITHUB_ACTIONS')=='true' and info['commit']!=os.environ.get('GITHUB_SHA'):
            raise ValueError('CI archive must match workflow commit')
        if info['dirty'] or info['target'] != 'x86_64-unknown-linux-gnu':
            raise ValueError('clean distributed Linux archive required')
        work = temporary/'work'
        work.mkdir(mode=0o700)
        binary = work/'kiarni'
        shutil.copyfile(package/'kiarni',binary)
        binary.chmod(0o755)
        server = Server(binary,work,work/'data')
        started = time.monotonic()
        try:
            server.start(['--max-requests','8','--max-streams','2','--feed-queue','1','--write-timeout-ms','3000'])
            token = next(iter(credentials(server.data)))
            def request(path, method='GET', body=None, expected=200):
                connection = http.client.HTTPConnection('127.0.0.1',server.port,timeout=10)
                headers={'Authorization':'Bearer '+token}
                if body is not None:
                    headers['Content-Type']='application/json' if method!='PATCH' else 'application/merge-patch+json'
                try:
                    connection.request(method,path,body,headers)
                    response=connection.getresponse()
                    data=response.read(4*1024*1024+1)
                    if response.status!=expected or len(data)>4*1024*1024:
                        raise RuntimeError('unexpected observation response status/size')
                    return data,response.getheader('X-Request-ID')
                finally:
                    connection.close()
            request('/v1/collections','POST',b'{"name":"observation"}',201)
            pad='private_soak_marker_'+'x'*699980
            for index in range(8):
                request('/v1/collections/observation/documents','POST',json.dumps({'_id':f'large{index}','pad':pad,'n':index}).encode(),201)
            request('/v1/collections/observation/documents','POST',b'{"_id":"progress","n":0}',201)
            # The external collector receives only a separate private credential file.
            config = temporary/'collector'
            config.mkdir(mode=0o700)
            (config/'token').write_text(token)
            (config/'token').chmod(0o600)
            (config/'prometheus.yml').write_text(f'''global:
  scrape_interval: 1s
  scrape_timeout: 1s
scrape_configs:
  - job_name: kiarni
    metrics_path: /v1/metrics
    follow_redirects: false
    authorization:
      type: Bearer
      credentials_file: /config/token
    static_configs:
      - targets: ['127.0.0.1:{server.port}']
''')
            with socket.socket() as listener:
                listener.bind(('127.0.0.1',0))
                collector_port=listener.getsockname()[1]
            mount=f'type=bind,src={config},dst=/config,readonly'
            docker('run','--rm','--network','none','--user',f'{os.getuid()}:{os.getgid()}',
                '--mount',mount,'--entrypoint','/bin/promtool',PROMETHEUS,'check','config','/config/prometheus.yml',timeout=180)
            docker('run','-d','--name',name,'--network','host','--user',f'{os.getuid()}:{os.getgid()}',
                '--read-only','--cap-drop','ALL','--security-opt','no-new-privileges',
                '--memory','512m','--cpus','1','--pids-limit','128','--mount',mount,
                '--tmpfs',f'/prometheus:rw,size=256m,mode=0700,uid={os.getuid()},gid={os.getgid()}',
                PROMETHEUS,'--config.file=/config/prometheus.yml','--storage.tsdb.path=/prometheus',
                '--storage.tsdb.retention.time=1h','--storage.tsdb.retention.size=32MB',
                f'--web.listen-address=127.0.0.1:{collector_port}',timeout=180)
            collector_started=True
            def query(expression):
                connection=http.client.HTTPConnection('127.0.0.1',collector_port,timeout=5)
                try:
                    connection.request('GET','/api/v1/query?'+urlencode({'query':expression}))
                    response=connection.getresponse()
                    if response.status!=200:
                        return None
                    output=json.loads(response.read(1024*1024))
                    results=output['data']['result']
                    return float(results[0]['value'][1]) if results else None
                finally:
                    connection.close()
            def wait_for(predicate, description, timeout=20):
                deadline=time.monotonic()+timeout
                while time.monotonic()<deadline:
                    try:
                        if predicate(): return
                    except OSError:
                        pass
                    time.sleep(0.1)
                raise RuntimeError('observation deadline: '+description)
            wait_for(lambda: query('up{job="kiarni"}')==1,'authenticated external scrape')
            # Diagnose actual query cost without changing/injecting delays into product code.
            costs={}
            query_request_ids=[]
            for kind, query_body in [('id',b'{"from":"observation","id":"progress","limit":1,"project":[]}'),
                                     ('scan',b'{"from":"observation","limit":1,"project":[]}' )]:
                before=request('/v1/metrics')[0].decode()
                elapsed=[]
                for _ in range(5):
                    point=time.monotonic()
                    _,rid=request('/v1/collections/observation/query','POST',query_body)
                    elapsed.append(time.monotonic()-point)
                    query_request_ids.append(rid)
                after=request('/v1/metrics')[0].decode()
                costs[kind]={'requests':5,'client_median_seconds':statistics.median(elapsed),
                    'execution_sum_delta_seconds':metric(after,'kiarni_query_duration_seconds_sum',stage='execution')-metric(before,'kiarni_query_duration_seconds_sum',stage='execution'),
                    'wait_sum_delta_seconds':metric(after,'kiarni_query_duration_seconds_sum',stage='wait')-metric(before,'kiarni_query_duration_seconds_sum',stage='wait')}
            if costs['scan']['execution_sum_delta_seconds'] <= costs['id']['execution_sum_delta_seconds']:
                raise RuntimeError('fixture did not demonstrate scan cost above ID lookup')
            def subscribe(slow_reader=False):
                connection=http.client.HTTPConnection('127.0.0.1',server.port,timeout=8)
                if slow_reader:
                    connection.sock=socket.socket()
                    connection.sock.setsockopt(socket.SOL_SOCKET,socket.SO_RCVBUF,1024)
                    connection.sock.settimeout(8)
                    connection.sock.connect(('127.0.0.1',server.port))
                connection.request('GET','/v1/collections/observation/changes',headers={'Authorization':'Bearer '+token,'Accept':'text/event-stream'})
                response=connection.getresponse()
                if response.status!=200 or b'event: ready' not in frame(response):
                    raise RuntimeError('observation subscription failed')
                return connection,response
            slow,slow_stream=subscribe(True)
            slow_request_id=slow_stream.getheader('X-Request-ID')
            fast,fast_stream=subscribe()
            delivered=0
            for number in range(12):
                request('/v1/collections/observation/documents/large0','PATCH',json.dumps({'n':100+number}).encode())
                while True:
                    event=frame(fast_stream)
                    if b'event: change' in event:
                        if b'"type":"update"' not in event:
                            raise RuntimeError('unexpected fast observer event')
                        delivered+=1
                        break
            wait_for(lambda: metric(request('/v1/metrics')[0].decode(),'kiarni_changefeed_subscriptions_ended_total',reason='overflow')>=1,'slow observer overflow')
            fast_stream.close();fast.close();fast=None
            slow_stream.close();slow.close();slow=None
            wait_for(lambda: json.loads(request('/v1/status')[0])['connections']['streaming']==0,'stream slots released',timeout=10)
            wait_for(lambda: (query('kiarni_changefeed_subscriptions_ended_total{reason="overflow"}') or 0)>=1,'external overflow evidence')
            observation_started=time.monotonic()
            updates=0
            # Low-rate continuous activity, not a throughput or memory benchmark.
            while time.monotonic()-observation_started < duration:
                updates+=1
                current=json.dumps({'_id':'progress','n':updates,'exact':9007199254740993}).encode()
                request('/v1/collections/observation/documents/progress','PUT',current)
                read,_=request('/v1/collections/observation/documents/progress')
                if json.loads(read)!=json.loads(current) or b'9007199254740993' not in read:
                    raise RuntimeError('confirmed document mismatch')
                request('/v1/collections/observation/query','POST',b'{"from":"observation","id":"progress","limit":1}')
                if query('up{job="kiarni"}')!=1:
                    raise RuntimeError('external collector reports unavailable target')
                if server.process.poll() is not None:
                    raise RuntimeError('server exited during observation')
                if updates%30==0:
                    print(f'Observation progress: {int(time.monotonic()-observation_started)}s, {updates} confirmed updates',flush=True)
                time.sleep(min(2,max(0,duration-(time.monotonic()-observation_started))))
            observed=time.monotonic()-observation_started
            external={'up':query('up{job="kiarni"}'),
                'scrapes_observed':query('count_over_time(up{job="kiarni"}[1h])'),
                'minimum_up':query('min_over_time(up{job="kiarni"}[1h])'),
                'query_success':query('kiarni_query_completed_total{outcome="success"}'),
                'overflow':query('kiarni_changefeed_subscriptions_ended_total{reason="overflow"}')}
            if external['up']!=1 or external['minimum_up']!=1 or external['scrapes_observed']<duration*0.8:
                raise RuntimeError('external collector coverage insufficient')
            docker('stop','--time','10',name);collector_started=False
            server.log.seek(0)
            raw=server.log.read(16*1024*1024+1)
            if len(raw)>16*1024*1024 or token.encode() in raw or b'private_soak_marker_' in raw:
                raise RuntimeError('logs size/privacy failure')
            records=[]
            for line in raw.splitlines():
                if line.startswith(b'{'):
                    record=json.loads(line)
                    if record.get('request_id') in query_request_ids or record.get('request_id')==slow_request_id:
                        records.append({key:record.get(key) for key in ['request_id','operation','status','duration_us','transport','stream_end_reason']})
            if not any(row['request_id']==slow_request_id and row['stream_end_reason']=='overflow' for row in records):
                raise RuntimeError('slow consumer log not correlated with metric')
            if not any(row['operation']=='Query' and int(row['duration_us'])>0 for row in records):
                raise RuntimeError('query logs not correlated')
            server.stop()
            server.start()
            persisted,_=request('/v1/collections/observation/documents/progress')
            if json.loads(persisted)!=json.loads(current) or b'9007199254740993' not in persisted or next(iter(credentials(server.data)))!=token:
                raise RuntimeError('post-observation persistence/auth mismatch')
            result={'schema':1,'recorded_at_utc':datetime.now(timezone.utc).isoformat(),'status':'passed','source_commit':info['commit'],'target':info['target'],
                'binary_sha256':hashlib.sha256(binary.read_bytes()).hexdigest(),
                'host':{'system':platform.system(),'release':platform.release(),'machine':platform.machine(),'libc':platform.libc_ver()},
                'prometheus_image':PROMETHEUS,'duration_requested_seconds':duration,'duration_observed_seconds':round(observed,3),
                'elapsed_total_seconds':round(time.monotonic()-started,3),
                'dataset':{'large_documents':8,'padding_bytes_each':len(pad),'progress_documents':1},
                'confirmed_progress_updates':updates,'fast_observer_events':delivered,'external_collector':external,
                'query_diagnosis':costs,'correlated_logs':records,'restart_preserved_document_and_credential':True,
                'scope':'bounded functional observation; not production certification or performance/RSS benchmark'}
            report.parent.mkdir(parents=True,exist_ok=True)
            with report.open('x') as output:
                json.dump(result,output,indent=2,sort_keys=True);output.write('\n')
            print('Operational observation passed; sanitized evidence written.',flush=True)
            return result
        finally:
            if fast: fast.close()
            if slow: slow.close()
            if collector_started:
                docker('rm','-f',name,check=False)
            else:
                docker('rm',name,check=False)
            if server: server.stop()

if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--artifact-dir',type=Path,required=True)
    parser.add_argument('--duration-seconds',type=int,default=900)
    parser.add_argument('--report',type=Path,required=True)
    args=parser.parse_args()
    exercise(args.artifact_dir,args.duration_seconds,args.report)
