"""Select a balanced profile only from repeated measured real-data runs."""
import argparse
import json
import math
import statistics
from pathlib import Path

p = argparse.ArgumentParser()
p.add_argument('--tags', nargs='+', required=True)
p.add_argument('--profiles', nargs='+', required=True)
p.add_argument('--out-name', default='selected_balanced')
a = p.parse_args()
root = Path(__file__).resolve().parents[1] / 'benchmarks/nasdaq'
results = []
for tag in a.tags:
    for path in sorted((root/'results').glob(tag+'-*.json')):
        d = json.loads(path.read_text())
        if d['target'] == 'strata' and d['profile'] in a.profiles and d['tier'] in ['1gb','windows']:
            if not d.get('eligible_for_profile_selection', True) or (d['profile'] == 'real_auto_z9_b131k' and 'effective_block_row_target' not in d):
                continue
            assert d['timing_valid'] and d['records'] == d['verified_records']
            assert d['columnar_scan']['timing_valid']
            results.append((path.name, d))
summary = {}
for profile in a.profiles:
    summary[profile] = {}
    for tier in ['1gb','windows']:
        runs = [(path,d) for path,d in results if d['profile']==profile and d['tier']==tier]
        assert len(runs) >= 2, (profile, tier, 'needs two independent runs')
        assert len({d['records'] for _,d in runs}) == 1
        summary[profile][tier] = dict(
            bytes=statistics.median(d['sizes']['physical_bytes'] for _,d in runs),
            ingest_rps=statistics.median(d['metrics']['records_per_sec'] for _,d in runs),
            scan_rps=statistics.median(d['columnar_scan']['records_per_sec'] for _,d in runs),
            reports=[path for path,_ in runs],
        )
for tier in ['1gb','windows']:
    best_size = min(v[tier]['bytes'] for v in summary.values())
    best_ingest = max(v[tier]['ingest_rps'] for v in summary.values())
    best_scan = max(v[tier]['scan_rps'] for v in summary.values())
    for v in summary.values():
        d=v[tier]
        d['cost'] = (d['bytes']/best_size * best_ingest/d['ingest_rps'] * best_scan/d['scan_rps'])**(1/3)
for v in summary.values():
    v['cost'] = math.sqrt(v['1gb']['cost'] * v['windows']['cost'])
winner = min(summary, key=lambda n: summary[n]['cost'])
report = dict(winner=winner, profiles=summary,
    method='Equal-weight geometric cost for physical bytes, ingest time and all-column scan time; median of repeats; equal weight to prefix and time-quartile windows',
    scope='Best among measured candidates, not a global optimum; 10 GB validation and full-corpus measurements follow separately')
(root/(a.out_name+'-selection.json')).write_text(json.dumps(report,indent=2)+'\n')
(root/'profiles'/(a.out_name+'.toml')).write_text((root/'profiles'/(winner+'.toml')).read_text())
print(json.dumps(report,indent=2))
