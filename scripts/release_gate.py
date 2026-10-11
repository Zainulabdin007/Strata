"""Run the release checks sequentially and retain exact commands and outcomes.

Do not run this concurrently with timed benchmarks. A failing or timed-out
check stops the gate; no check is silently skipped.
"""
import argparse
import datetime
import json
from pathlib import Path
import subprocess
import time

ROOT = Path(__file__).resolve().parents[1]
p = argparse.ArgumentParser(description=__doc__)
p.add_argument('--tag', required=True)
p.add_argument('--product-only', action='store_true')
p.add_argument('--rust-only', action='store_true')
a = p.parse_args()
assert not (a.product_only and a.rust_only)
out = ROOT / 'benchmarks/release-checks' / a.tag
out.mkdir(parents=True, exist_ok=False)
checks = []
if not a.product_only:
    checks += [(ROOT, cmd) for cmd in [
        ['cargo', 'fmt', '--all', '--', '--check'],
        ['cargo', 'clippy', '--workspace', '--all-targets', '--', '-D', 'warnings'],
        ['cargo', 'test', '--workspace'],
        ['cargo', 'test', '--workspace', '--release'],
        ['cargo', 'build', '--workspace'],
        ['cargo', 'build', '--workspace', '--release'],
    ]]
if not a.rust_only:
    checks += [
        (ROOT / 'strata-lab', ['npm', 'run', 'build']),
        (ROOT, ['python3', 'scripts/test_api.py']),
        (ROOT, ['python3', 'scripts/test_folder.py']),
        (ROOT / 'strata-lab', ['node', 'tests/e2e.mjs']),
    ]
receipt = {'started_utc': datetime.datetime.now(datetime.timezone.utc).isoformat(),
           'status': 'running', 'checks': []}
for i, (cwd, cmd) in enumerate(checks):
    print('RUN', ' '.join(cmd), flush=True)
    log = out / f'{i:02d}.log'
    start = time.monotonic()
    try:
        with log.open('w') as stream:
            code = subprocess.run(cmd, cwd=cwd, stdout=stream, stderr=subprocess.STDOUT,
                                  timeout=900, check=False).returncode
    except subprocess.TimeoutExpired:
        code = 'timeout'
    receipt['checks'].append(dict(command=cmd, cwd=str(cwd), returncode=code,
                                  seconds=time.monotonic()-start, log=log.name))
    receipt['status'] = 'failed' if code != 0 else 'running'
    (out / 'receipt.json').write_text(json.dumps(receipt, indent=2) + '\n')
    if code != 0:
        print(log.read_text()[-12000:], flush=True)
        raise SystemExit(f'FAILED {cmd}: {code}; evidence in {out}')
    print('PASS', ' '.join(cmd), flush=True)
receipt['status'] = 'passed'
receipt['finished_utc'] = datetime.datetime.now(datetime.timezone.utc).isoformat()
(out / 'receipt.json').write_text(json.dumps(receipt, indent=2) + '\n')
print('ALL REQUESTED RELEASE CHECKS PASSED', out, flush=True)
