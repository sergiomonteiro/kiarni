"""Record a bounded paired RF-04 experiment with hardware/toolchain/source metadata."""
import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import platform
import statistics
import subprocess
import tempfile
import time

ROOT=Path(__file__).resolve().parents[1]

def output(*args):
    return subprocess.check_output(args,cwd=ROOT,text=True).strip()

def fingerprint():
    paths=[ROOT/'Cargo.toml',ROOT/'Cargo.lock',Path(__file__).resolve()]
    for folder in ['crates/core','crates/engine','spikes/storage']:
        paths.extend((ROOT/folder).rglob('*.rs'))
        paths.append(ROOT/folder/'Cargo.toml')
    digest=hashlib.sha256()
    names=[]
    for path in sorted(set(paths)):
        name=path.relative_to(ROOT).as_posix()
        digest.update(name.encode()+b'\0'+path.read_bytes()+b'\0')
        names.append(name)
    return {'sha256':digest.hexdigest(),'recipe':'sorted relative UTF-8 path + NUL + bytes + NUL','files':names}

def hardware():
    result={'system':platform.system(),'kernel':platform.release(),'machine':platform.machine(),'logical_cpus':os.cpu_count(),'libc':platform.libc_ver()}
    cpu=Path('/proc/cpuinfo')
    if cpu.exists():
        result['cpu_model']=next((line.split(':',1)[1].strip() for line in cpu.read_text().splitlines() if line.startswith('model name')),'unavailable')
        mem=Path('/proc/meminfo').read_text().splitlines()
        result['total_memory_bytes']=int(next(line for line in mem if line.startswith('MemTotal:')).split()[1])*1024
        governor=Path('/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor')
        result['cpu_governor']=governor.read_text().strip() if governor.exists() else 'unavailable'
        try:
            filesystems=json.loads(output('findmnt','--target',tempfile.gettempdir(),'--json','--output','FSTYPE,SOURCE'))['filesystems']
            result['temporary_filesystem']=filesystems[0]
        except (OSError,subprocess.CalledProcessError,KeyError):
            result['temporary_filesystem']='unavailable'
    result['cpu_affinity']='not pinned; shared host; caches not dropped'
    result['device_power_loss_guarantees']='not verified'
    return result

def summarize(runs):
    groups={}
    for row in runs:
        key=(row['documents'],row['status_cardinality'])
        groups.setdefault(key,{})[(row['repetition'],row['measurements']['indexed'])]=row['measurements']
    result=[]
    for (documents,cardinality),pairs in sorted(groups.items()):
        repeats=sorted({repetition for repetition,_ in pairs})
        ratios={phase:[] for phase in ['insert','indexed_field_update','other_field_update','delete']}
        speedups=[];sizes=[]
        for repetition in repeats:
            scan,index=pairs[(repetition,False)],pairs[(repetition,True)]
            speedups.append(scan['query']['p50_ns']/index['query']['p50_ns'])
            for phase in ratios:
                ratios[phase].append(index[phase]['p50_ns']/scan[phase]['p50_ns'])
            sizes.append(index['file_logical_bytes']['inserted']/scan['file_logical_bytes']['inserted'])
        row={'documents':documents,'status_cardinality':cardinality,'query_speedup_scan_over_index_median':statistics.median(speedups),
             'write_p50_ratio_index_over_scan_median':{phase:statistics.median(values) for phase,values in ratios.items()},
             'inserted_logical_file_ratio_median':statistics.median(sizes),
             'scan_examined_documents':pairs[(0,False)]['query_examined_documents'],
             'index_examined_documents':pairs[(0,True)]['query_examined_documents'],
             'repetitions':len(repeats),'query_speedup_each_pair':speedups}
        result.append(row)
    return result

def main(destination,repetitions=3,queries=30):
    if destination.exists():
        raise ValueError('new evidence file required')
    command=['cargo','run','--release','--locked','--offline','-p','kiarni-storage-spike','--example','secondary_index','--','--repetitions',str(repetitions),'--queries',str(queries)]
    source=fingerprint()
    start=time.monotonic()
    process=subprocess.run(command,cwd=ROOT,text=True,capture_output=True,timeout=600)
    if process.returncode:
        raise RuntimeError('Rust secondary-index experiment failed: '+process.stderr[-2000:])
    measurements=json.loads(process.stdout)
    if fingerprint()!=source:
        raise RuntimeError('source changed during measurement')
    result={'schema':1,'status':'passed','recorded_at_utc':datetime.now(timezone.utc).isoformat(),
            'base_commit':output('git','rev-parse','HEAD'),'source_dirty':bool(output('git','status','--porcelain')),
            'source_fingerprint':source,'hardware':hardware(),'rustc':output('rustc','-Vv'),'cargo':output('cargo','-V'),
            'command':command,'elapsed_including_build_seconds':round(time.monotonic()-start,3),
            'measurement':measurements,'summary':summarize(measurements['runs']),
            'limits':'single shared Linux host, warm cache, one writer, Immediate per-document commits; no HTTP/group commit, cold-cache, CPU/RSS or production throughput claim'}
    destination.parent.mkdir(parents=True,exist_ok=True)
    with destination.open('x') as file:
        json.dump(result,file,indent=2,sort_keys=True);file.write('\n')
    print(json.dumps(result['summary'],indent=2))
    return result

if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output',required=True,type=Path)
    parser.add_argument('--repetitions',type=int,choices=range(1,6),default=3)
    parser.add_argument('--queries',type=int,choices=range(10,201),default=30)
    args=parser.parse_args()
    main(args.output,args.repetitions,args.queries)
