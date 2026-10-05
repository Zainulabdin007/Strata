"""Derive bounded candidate profiles from frozen REAL-data optimizer output.
No synthetic field assumptions. Selection happens only after repeated tier runs.
"""
import json
import re
from pathlib import Path

root = Path(__file__).resolve().parents[1] / 'benchmarks/nasdaq'
profiles = root / 'profiles'
choices = [
    ('real_auto_z1_b64k', 'max_compression', 1, 65536),
    ('real_auto_z3_b64k', 'max_compression', 3, 65536),
    ('real_auto_z9_b131k', 'max_compression', 9, 131072),
    ('real_plain_z1_b64k', 'manual', 1, 65536),
    ('real_delta_z3_b64k', 'balanced', 3, 65536),
]
for name, parent, level, block_rows in choices:
    text = (profiles / f'{parent}.toml').read_text()
    text = re.sub(r'^name = .*$', f'name = "{name}"', text, count=1, flags=re.M)
    text = re.sub(r'^row_count = \d+$', f'row_count = {block_rows}', text, flags=re.M)
    text = re.sub(r'^level = \d+$', f'level = {level}', text, flags=re.M)
    path = profiles / f'{name}.toml'
    if path.exists() and path.read_text() != text:
        raise RuntimeError(f'Refusing to alter frozen candidate {path}')
    path.write_text(text)
(root / 'candidate-space.json').write_text(json.dumps({
    'basis': 'Original 32768-record time-quartile real Nasdaq optimizer; no synthetic profiles',
    'candidates': [dict(name=n, parent=p, zstd_level=z, block_rows=b) for n,p,z,b in choices],
    'selection': 'Repeat finalists at 1 GB; equal-weight geometric size/ingest/columnar-decode score with ratios relative to best observed per dimension; validate winner at 10 GB before full corpus',
    'scope': 'Bounded empirical candidate set, not a global optimum claim',
}, indent=2)+'\n')
