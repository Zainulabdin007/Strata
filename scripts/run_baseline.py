"""Run the complete measured release matrix; any failed or missing run is fatal.

Synthetic market-v2 only. This does not produce claims about Nasdaq/Databento.
Each process writes a real dataset, decodes every row and checks Strata equality.
"""
import argparse, datetime, hashlib, json, pathlib, platform, re, subprocess, sys, time
ROOT=pathlib.Path(__file__).resolve().parents[1]
p=argparse.ArgumentParser()
p.add_argument('--sizes',nargs='+',type=int,default=[100_000,1_000_000,10_000_000,100_000_000])
p.add_argument('--out',default='benchmarks/verified-gates')
p.add_argument('--work',default='benchmarks/work-gates')
p.add_argument('--tag',default='baseline')
p.add_argument('--workers',type=int,default=2)
p.add_argument('--profiles',nargs='+',default=['plain','manual','balanced','max_compression','max_ingest'])
p.add_argument('--formats',nargs='*',default=['jsonl','fixed','parquet'])
p.add_argument('--prepare',action='store_true')
a=p.parse_args();out=ROOT/a.out;out.mkdir(parents=True,exist_ok=True)
binary=ROOT/'target/release/baseline'
def call(cmd):return subprocess.check_output(cmd,text=True,cwd=ROOT).strip()
source_hash=hashlib.sha256()
for f in sorted((ROOT/'crates').rglob('*.rs')):
 source_hash.update(str(f.relative_to(ROOT)).encode());source_hash.update(f.read_bytes())
meta={'started_utc':datetime.datetime.now(datetime.timezone.utc).isoformat(),'platform':platform.platform(),'machine':platform.machine(),'rustc':call(['rustc','--version']),'cargo':call(['cargo','--version']),'binary_sha256':hashlib.sha256(binary.read_bytes()).hexdigest(),'rust_sources_sha256':source_hash.hexdigest(),'cargo_lock_sha256':hashlib.sha256((ROOT/'Cargo.lock').read_bytes()).hexdigest(),'argv':sys.argv,'corpus':'synthetic market-v2; NOT Nasdaq data','git_commit':None,'limitations':['single run per case unless tagged repeat','warm filesystem cache; no direct-I/O','fast durability (no fsync guarantee)','peak RSS and CPU cover full process, including verification/query']}
if platform.system()=='Darwin':
 hw=call(['sysctl','-n','machdep.cpu.brand_string','hw.memsize','hw.logicalcpu']).splitlines()
 meta.update(cpu=hw[0],ram_bytes=int(hw[1]),logical_cpus=int(hw[2]))
meta_path=out/f'{a.tag}-environment.json';meta_path.write_text(json.dumps(meta,indent=2)+'\n')
base=[str(binary),'--out',a.out,'--work',a.work,'--workers',str(a.workers),'--tag',a.tag]
if a.prepare:
 subprocess.run([*base,'--prepare'],cwd=ROOT,check=True)
for size in a.sizes:
 for kind,label in [('strata',x) for x in a.profiles]+[(x,x) for x in a.formats]:
  name=f'{a.tag}-{label}-{size}-w{a.workers}';report=out/f'{name}.json'
  if report.exists():raise RuntimeError(f'Refusing to overwrite {report}; use another tag')
  cmd=[*base,'--records',str(size),'--format',kind,'--profile',label]
  print('RUN',name,flush=True);start=time.monotonic()
  with (out/f'{name}.stdout').open('w') as stdout,(out/f'{name}.time').open('w') as stderr:
   subprocess.run(['/usr/bin/time','-l',*cmd],cwd=ROOT,stdout=stdout,stderr=stderr,check=True)
  record=json.loads(report.read_text());timing=(out/f'{name}.time').read_text()
  assert record['release_build'] is True and record['records']==size and record['decoded_records']==size
  if kind=='strata':assert record['verified_records']==size and record['query']['rows']==min(1000,size-size//2)
  wall=re.search(r'([\d.]+) real\s+([\d.]+) user\s+([\d.]+) sys',timing)
  rss=re.search(r'(\d+)\s+maximum resident set size',timing)
  usage={'process_wall_seconds':time.monotonic()-start,'peak_rss_bytes':int(rss[1]) if rss else None}
  if wall:
   real,user,system=map(float,wall.groups());usage.update(time_wall_seconds=real,user_seconds=user,system_seconds=system,cpu_percent_of_one_core=(user+system)/real*100 if real else None)
  record['process_usage']=usage;record['environment']=meta_path.name
  record['timing_valid']=bool(wall) and abs(usage['time_wall_seconds']-usage['process_wall_seconds'])<=max(1,usage['process_wall_seconds']*.05)
  if not record['timing_valid']:raise RuntimeError(f'Clock discrepancy in {name}: possible system suspension; rerun this case')
  report.write_text(json.dumps(record,indent=2)+'\n')
  print(f"PASS {name}: {record['sizes']['physical_bytes']} bytes; {usage['process_wall_seconds']:.2f}s whole process; RSS={usage['peak_rss_bytes']}",flush=True)
print('ALL REQUESTED RUNS COMPLETED',flush=True)
