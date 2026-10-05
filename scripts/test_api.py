"""Gate 18: actual HTTP, compressed ingest, canonical metrics, optimizer, SSE."""
import json, os, pathlib, shutil, subprocess, tempfile, time, urllib.request, urllib.error
repo=pathlib.Path(__file__).resolve().parents[1]
root=pathlib.Path(tempfile.mkdtemp(prefix='strata-api-test-'))
shutil.copytree(repo/'examples',root/'examples')
port=19160
proc=subprocess.Popen([str(repo/'target/debug/strata-api'),'--root',str(root),'--port',str(port)],stdout=subprocess.DEVNULL)
base=f'http://127.0.0.1:{port}'
def call(path,data=None,headers=None):
 h={'X-Strata-Local':'1','Content-Type':'application/json'};h.update(headers or {})
 r=urllib.request.Request(base+'/api/'+path,data=json.dumps(data).encode() if data is not None else None,headers=h)
 with urllib.request.urlopen(r,timeout=20) as f:return json.load(f)
def job(data):
 j=call('jobs',data)
 for _ in range(600):
  j=call(f'jobs/{j["id"]}')
  if j['status']!='running':
   assert j['status']=='complete',j
   return j
  time.sleep(.05)
 raise AssertionError('job timeout')
try:
 for _ in range(100):
  try:call('health');break
  except (OSError,urllib.error.URLError):time.sleep(.05)
 else:raise AssertionError('API did not start')
 assert call('metrics')['metrics'] is None
 job({'operation':'ingest','dataset':'market','input':'examples/data/market.csv','profile':'examples/profiles/stock-market.toml'})
 d=call('datasets/market');m=call('metrics')['metrics']
 assert d['metrics']['records']==m['records_processed']>0
 stored=json.loads((root/'datasets/market/ingest-metrics.json').read_text())
 assert stored==m,(stored,m)
 assert m['physical_bytes_written']==d['metrics']['physical_bytes'],(m,d)
 assert m['compression_ratio']==m['logical_input_bytes']/m['physical_bytes_written']
 assert call('datasets/market/segments')
 assert call('datasets/market/blocks')
 q=call('datasets/market/query',{'query':'select * limit 10'})
 assert len(q['rows'])==m['records_processed']
 assert q['stats']['blocks_scanned']>0
 job({'operation':'analysis','dataset':'market'})
 assert call('datasets/market/columns')['report']['record_count']==m['records_processed']
 j=job({'operation':'optimization','dataset':'market','goal':'balanced'})
 assert j['result']['report']['sample_rows']==m['records_processed']
 assert all(c['trials'] for c in j['result']['report']['columns'])
 with urllib.request.urlopen(base+'/api/events',timeout=5) as stream:
  assert stream.readline().strip()==b'event: metrics'
  event=json.loads(stream.readline().decode().removeprefix('data: '))
  assert event['metrics']==m
 for headers in [{'Origin':'https://evil.example'},{'Host':'evil.example'}]:
  try:call('health',headers=headers);raise AssertionError('origin accepted')
  except urllib.error.HTTPError as e:assert e.code==403
 assert call('benchmarks')==[]
 subprocess.run([str(repo/'target/debug/strata'),'ingest',str(root/'examples/data/market.csv'),'--profile',str(root/'examples/profiles/stock-market.toml'),'--output',str(root/'datasets/cli')],check=True,stdout=subprocess.DEVNULL)
 cli=json.loads((root/'datasets/cli/ingest-metrics.json').read_text())
 for key in ['records_processed','logical_input_bytes','physical_bytes_written','compression_ratio']:
  assert cli[key]==m[key],(key,cli[key],m[key])
 print('PASS gate 18: HTTP ingest/query/analyze/optimize, SSE, canonical CLI/API metrics, local origin restrictions')
finally:
 proc.terminate();proc.wait(timeout=5);shutil.rmtree(root)
