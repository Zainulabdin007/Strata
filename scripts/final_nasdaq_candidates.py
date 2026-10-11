"""Freeze additional real-MBO profile candidates for the final measured sweep.

All candidates retain the canonical schema. These are hypotheses from the real
profiler (wide/random order IDs, integer prices, narrow codes), not winners.
Run and repeat them on actual windows and prefixes before selecting defaults.
"""
import json
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
profiles = ROOT / 'benchmarks/nasdaq/profiles'
base = (profiles / 'selected_balanced.toml').read_text()
candidates = [
    ('final_auto_z3_b262k', {}, 'zstd', 3, 262144),
    ('final_auto_z9_b262k', {}, 'zstd', 9, 262144),
    ('final_auto_z15_b262k', {}, 'zstd', 15, 262144),
    ('final_auto_lz4_b64k', {}, 'lz4', 0, 65536),
    ('final_price_plain_z3', {'price': ['plain']}, 'zstd', 3, 65536),
    ('final_price_plain_z9', {'price': ['plain']}, 'zstd', 9, 131072),
    ('final_price_plain_z15', {'price': ['plain']}, 'zstd', 15, 262144),
    ('final_order_for_z3', {'order_id': ['frame_of_reference', 'bitpack']}, 'zstd', 3, 65536),
    ('final_codes_bitpack_z3', {'flags': ['bitpack'], 'action': ['bitpack'], 'side': ['bitpack']}, 'zstd', 3, 65536),
]
for name, encodings, codec, level, rows in candidates:
    text = re.sub(r'^name = .*$', f'name = "{name}"', base, count=1, flags=re.M)
    chunks = text.split('[[columns]]')
    for i in range(1, len(chunks)):
        column = re.search(r'^name = "([^"]+)"', chunks[i], re.M)[1]
        if column in encodings:
            chunks[i] = re.sub(r'^encoding = .*$', 'encoding = ' + json.dumps(encodings[column]), chunks[i], count=1, flags=re.M)
    text = '[[columns]]'.join(chunks)
    text = re.sub(r'^row_count = .*$', f'row_count = {rows}', text, flags=re.M)
    text = re.sub(r'^codec = .*$', f'codec = "{codec}"', text, flags=re.M)
    text = re.sub(r'^level = .*$', f'level = {level}', text, flags=re.M)
    target = profiles / f'{name}.toml'
    if target.exists() and target.read_text() != text:
        raise RuntimeError(f'Refusing to change frozen profile {target}')
    target.write_text(text)
print(' '.join(c[0] for c in candidates))
