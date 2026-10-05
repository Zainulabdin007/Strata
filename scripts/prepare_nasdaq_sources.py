"""Extract only original transport files and sidecars; verify vendor SHA-256.
Never edits the archive or the user's existing .dbn.zst files.
"""
from pathlib import Path
import zipfile, hashlib, json, datetime, os
root=Path(__file__).resolve().parents[1]
archive=Path('/Users/zain/Downloads/XNAS-20260915-AWFMJJN8F5.zip')
out=root/'benchmarks/nasdaq/sources';out.mkdir(parents=True,exist_ok=True)
with zipfile.ZipFile(archive) as z:
 manifest=json.loads(z.read('manifest.json'));expected={f['filename']:f for f in manifest['files']}
 for name in ['manifest.json',*expected]:
  p=out/name
  if p.exists():
   if name in expected:
    h=hashlib.file_digest(p.open('rb'),'sha256').hexdigest();assert h==expected[name]['hash'].split(':')[1]
   continue
  temporary=p.with_suffix(p.suffix+'.extracting');h=hashlib.sha256();count=0
  with z.open(name) as src,temporary.open('xb') as dest:
   while block:=src.read(8*1024*1024):dest.write(block);h.update(block);count+=len(block)
   dest.flush();os.fsync(dest.fileno())
  if name in expected:
   assert count==expected[name]['size'];assert h.hexdigest()==expected[name]['hash'].split(':')[1],name
  temporary.rename(p);print('VERIFIED',name,count,h.hexdigest(),flush=True)
 sources=[dict(path=str((out/f['filename']).resolve()),bytes=f['size'],sha256=f['hash'].split(':')[1]) for f in expected.values() if f['filename'].endswith('.dbn.zst')]
 inventory={'archive':str(archive),'source_files':sources,'vendor_transport_bytes':sum(s['bytes'] for s in sources),'verified_utc':datetime.datetime.now(datetime.timezone.utc).isoformat(),'status':'exact transport sizes and SHA-256 verified; decompressed byte/record totals not yet measured'}
 (out.parent/'source-inventory.json').write_text(json.dumps(inventory,indent=2)+'\n')
