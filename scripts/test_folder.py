"""Real CLI folder ingest and file-boundary resume regressions."""
import pathlib,subprocess,tempfile,json,shutil
repo=pathlib.Path(__file__).resolve().parents[1];root=pathlib.Path(tempfile.mkdtemp(prefix='strata-folder-'));cli=repo/'target/debug/strata'
def run(*args,ok=True):
 p=subprocess.run([str(cli),*map(str,args)],capture_output=True,text=True)
 assert (p.returncode==0)==ok,(p.args,p.stdout,p.stderr)
 return p
try:
 src=root/'input';src.mkdir();(src/'nested').mkdir()
 profile=root/'profile.toml';profile.write_text('''name="folder"
input_format="csv"
[schema]
id="events"
[[columns]]
name="id"
logical_type="unsigned_integer"
width="bits64"
encoding=["delta","zigzag","varint"]
[block]
row_count=2
[compression]
codec="zstd"
''')
 (src/'a.csv').write_text('id\n1\n2\n3\n');(src/'nested/b.csv').write_text('wrong\n4\n');(src/'skip.csv').write_text('bad data')
 (src/'ignore.txt').write_text('ignored');(src/'nested/cycle').symlink_to(src,target_is_directory=True)
 output=root/'dataset';args=['ingest',src,'--profile',profile,'--output',output,'--exclude','skip.csv','--workers','2']
 failed=run(*args,ok=False);assert 'b.csv' in failed.stderr,failed.stderr
 assert json.loads((output/'manifest.json').read_text())['record_count']==3
 assert json.loads((output/'ingest-errors.json').read_text())[0]['committed'] is False
 (src/'nested/b.csv').write_text('id\n4\n5\n')
 run(*args,'--resume')
 manifest=json.loads((output/'manifest.json').read_text());assert manifest['record_count']==5
 snap=json.loads((output/'ingest-metrics.json').read_text());assert snap['records_processed']==5 and snap['files_completed']==2 and snap['total_files']==2
 actual=(output/'manifest.json').stat().st_size+sum(p.stat().st_size for p in (output/'segments').iterdir());assert snap['physical_bytes_written']==actual
 assert snap['compression_ratio']==snap['logical_input_bytes']/actual
 run(*args,'--resume');assert json.loads((output/'manifest.json').read_text())['record_count']==5
 q=run('query','--dataset',output,'--query','select id','--format','json');values=json.loads(q.stdout)
 assert [r['values']['id']['value'] for r in values]==[1,2,3,4,5],values
 run('verify',output)
 (src/'a.csv').write_text('id\n9\n2\n3\n');p=run(*args,'--resume',ok=False);assert 'completed source changed' in p.stderr
 assert json.loads((output/'manifest.json').read_text())['record_count']==5
 (src/'a.csv').write_text('id\n1\n2\n3\n')
 out2=root/'filtered';run('ingest',src,'--profile',profile,'--output',out2,'--include','nested/*.csv')
 assert json.loads((out2/'manifest.json').read_text())['record_count']==2
 # Reject incompatible schema rather than silently dropping extra columns.
 bad=root/'extra.csv';bad.write_text('id,extra\n1,2\n');run('ingest',bad,'--profile',profile,'--output',root/'bad',ok=False)
 print('PASS gate 21: recursive real-file ingest, stable filtered ordering, symlink cycle exclusion, schema rejection, partial-file rollback, idempotent resume, changed-source rejection, canonical aggregate sizes')
finally:shutil.rmtree(root)
