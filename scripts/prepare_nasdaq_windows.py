"""Copy deterministic, record-aligned windows from verified expanded DBN.
Reuse the original time-quartile census boundaries; preserve exact metadata.
"""
import hashlib
import json
from pathlib import Path

root = Path(__file__).resolve().parents[1] / 'benchmarks/nasdaq'
windows = json.loads((root / 'profiles/sampling.json').read_text())
verified = json.loads((root / 'uncompressed/verification.json').read_text())
out = root / 'representative-windows'
out.mkdir(exist_ok=True)
ledger = []
for index, w in enumerate(windows):
    source = next(s for s in verified if Path(s['source']).name == Path(w['source']).name)
    start = w['start_record']
    count = 131072
    assert start + count <= source['records']
    target = out / f'window-{index:02}.dbn'
    with Path(source['output']).open('rb') as f:
        metadata = f.read(source['metadata_bytes'])
        f.seek(source['metadata_bytes'] + start * 56)
        payload = f.read(count * 56)
    assert len(payload) == count * 56
    with target.open('xb') as f:
        f.write(metadata)
        f.write(payload)
    ledger.append(dict(source=source['output'], source_dbn_sha256=source['dbn_sha256'],
                       output=str(target), start_record=start, records=count,
                       boundary_ts_event=w['boundary_ts_event'], logical_bytes=len(payload),
                       canonical_sha256=hashlib.sha256(payload).hexdigest(),
                       dbn_sha256=hashlib.sha256(metadata+payload).hexdigest()))
(out / 'provenance.json').write_text(json.dumps(ledger, indent=2)+'\n')
print(f'Created {len(ledger)} windows, {len(ledger)*count:,} real records')
