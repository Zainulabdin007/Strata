"""Materialize original DBN streams without modifying vendor Zstd files.
Validate Zstd EOF, vendor SHA-256, metadata, and full MBO payload against census.
Only rename a .partial output after all checks pass. Uses bounded memory.
"""
import datetime
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import time

ROOT = Path(__file__).resolve().parents[1]
BASE = ROOT / 'benchmarks/nasdaq'
OUT = BASE / 'uncompressed'

def main():
    OUT.mkdir(exist_ok=True)
    census = json.loads((BASE / 'source-census.json').read_text())
    manifest = {f['filename']: f for f in json.loads((BASE / 'sources/manifest.json').read_text())['files']}
    required = sum(s['decompressed_bytes_observed'] for s in census if not (OUT / Path(s['source']).name.removesuffix('.zst')).exists())
    if shutil.disk_usage(OUT).free < required + 10_000_000_000:
        raise RuntimeError('Insufficient free disk for expanded DBN plus reserve')
    reports = []
    for s in census:
        src = Path(s['source'])
        dst = OUT / src.name.removesuffix('.zst')
        report_path = dst.with_suffix('.verification.json')
        if dst.exists():
            raise RuntimeError(f'Refusing to overwrite {dst}; verify existing outputs explicitly')
        started = time.monotonic()
        before = src.stat()
        expected = manifest[src.name]
        with src.open('rb') as f:
            vendor_hash = hashlib.file_digest(f, 'sha256').hexdigest()
        assert before.st_size == expected['size'] == s['source_file_bytes']
        assert 'sha256:' + vendor_hash == expected['hash']
        assert vendor_hash == s['source_sha256']
        print(f'VERIFIED original {src.name}: {before.st_size} bytes', flush=True)
        tmp = dst.with_suffix('.dbn.partial')
        full_hash = hashlib.sha256()
        payload_hash = hashlib.sha256()
        total = 0
        last = time.monotonic()
        proc = subprocess.Popen(['zstd', '-d', '-c', '--', str(src)], stdout=subprocess.PIPE)
        try:
            with tmp.open('xb') as f:
                while block := proc.stdout.read(8 * 1024 * 1024):
                    if total == 0:
                        assert block[:3] == b'DBN'
                        assert int.from_bytes(block[4:8], 'little') + 8 == s['metadata_bytes']
                        assert block[3] == s['metadata']['dbn_version']
                    f.write(block)
                    full_hash.update(block)
                    payload_hash.update(block[max(0, s['metadata_bytes'] - total):])
                    total += len(block)
                    if time.monotonic() - last >= 5:
                        print(f'{src.name}: {total:,} DBN bytes written', flush=True)
                        last = time.monotonic()
                f.flush()
                os.fsync(f.fileno())
            assert proc.wait() == 0, 'Zstandard decompression failed'
        finally:
            proc.stdout.close()
            if proc.poll() is None:
                proc.terminate()
                proc.wait()
        assert total == s['decompressed_bytes_observed'] == s['dbn_bytes_accepted']
        assert total - s['metadata_bytes'] == s['records'] * 56
        assert payload_hash.hexdigest() == s['canonical_sha256']
        after = src.stat()
        assert (before.st_size, before.st_mtime_ns) == (after.st_size, after.st_mtime_ns)
        tmp.rename(dst)
        report = dict(source=str(src), output=str(dst), compressed_bytes=before.st_size,
                      decompressed_bytes=total, source_sha256=vendor_hash, dbn_sha256=full_hash.hexdigest(),
                      canonical_sha256=payload_hash.hexdigest(), records=s['records'],
                      metadata_bytes=s['metadata_bytes'], logical_bytes=s['logical_bytes'],
                      elapsed_seconds=time.monotonic()-started, verified_utc=datetime.datetime.now(datetime.timezone.utc).isoformat(),
                      validation='Zstd clean EOF; vendor SHA-256; exact DBN metadata length/version; full ordered MBO payload SHA-256 equals reference decoder census')
        report_path.write_text(json.dumps(report, indent=2)+'\n')
        reports.append(report)
        print(f'PASS {dst.name}: {total:,} bytes, {s["records"]:,} records', flush=True)
    (OUT/'verification.json').write_text(json.dumps(reports, indent=2)+'\n')

if __name__ == '__main__':
    main()
