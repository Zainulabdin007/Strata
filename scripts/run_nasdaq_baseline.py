"""Sequential release measurements of the frozen real Nasdaq DBN corpus.

No extrapolation or implicit skipping. Failed validation aborts the requested matrix.
A full tier uses the manual baseline by default; optimizer goals are compared on
validation/1 GB/10 GB tiers. All formats reconstruct and hash every source field.
"""
import argparse
import datetime
import hashlib
import json
import pathlib
import platform
import re
import subprocess
import sys
import time
import tarfile
import shutil

ROOT = pathlib.Path(__file__).resolve().parents[1]
p = argparse.ArgumentParser(description=__doc__)
p.add_argument('--out', default='benchmarks/nasdaq')
p.add_argument('--input', default='benchmarks/nasdaq/sources')
p.add_argument('--prepare', action='store_true')
p.add_argument('--start-after-file', help='Wait for a prerequisite artifact before timed runs')
p.add_argument('--tiers', nargs='*', default=['validation', '1gb', '10gb', 'full'])
p.add_argument('--profiles', nargs='+', default=['manual', 'balanced', 'max_compression', 'max_ingest', 'max_decode'])
p.add_argument('--full-profiles', nargs='+', default=['manual'])
p.add_argument('--targets', nargs='*', default=['parquet', 'fixed'])
p.add_argument('--workers', type=int, default=2)
p.add_argument('--tag', default='baseline')
a = p.parse_args()
out = ROOT / a.out
out.mkdir(parents=True, exist_ok=True)
logs = out / 'logs'
logs.mkdir(exist_ok=True)
binary = ROOT / 'target/release/nasdaq-baseline'

def call(cmd):
    return subprocess.check_output(cmd, text=True, cwd=ROOT).strip()

source_hash = hashlib.sha256()
for f in sorted((ROOT / 'crates').rglob('*.rs')):
    source_hash.update(str(f.relative_to(ROOT)).encode())
    source_hash.update(f.read_bytes())
meta = {
    'started_utc': datetime.datetime.now(datetime.timezone.utc).isoformat(),
    'platform': platform.platform(), 'machine': platform.machine(),
    'rustc': call(['rustc', '--version']), 'cargo': call(['cargo', '--version']),
    'binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(),
    'rust_sources_sha256': source_hash.hexdigest(),
    'cargo_lock_sha256': hashlib.sha256((ROOT / 'Cargo.lock').read_bytes()).hexdigest(),
    'argv': sys.argv, 'git_commit': None,
    'corpus': 'Databento XNAS.ITCH MBO; actual source files',
    'limitations': [
        'single measurement per case unless repeated with a new tag',
        'OS-managed filesystem cache; no cache flush or guaranteed cold/warm state; buffered I/O; fast durability without fsync guarantee',
        'no CPU affinity; other host activity may affect timing; repeated sample results used for profile selection',
        'serial column scans and equal-worker parallel column scans reported separately; Strata arrays retain FieldValue tags, Parquet uses Arrow primitive arrays',
        'ingest includes DBN decode, source hashing, canonical rows and target writing; outer decompression only when source_representation is streaming vendor DBN.zst',
        'columnar_scan is separate from validation-inclusive decode; all columns materialized into bounded arrays for both Strata and Parquet',
        'decode includes all-field canonical materialization and SHA-256 validation for every target; Strata and Parquet use bounded column arrays, fixed uses bounded rows',
        'peak RSS and CPU cover entire process including input, verification and queries',
        'full tier compares only explicitly selected --full-profiles',
        'vendor full-file transport bytes are not a denominator for partial tiers',
    ],
}
if platform.system() == 'Darwin':
    hw = call(['sysctl', '-n', 'machdep.cpu.brand_string', 'hw.memsize', 'hw.logicalcpu']).splitlines()
    meta.update(cpu=hw[0], ram_bytes=int(hw[1]), logical_cpus=int(hw[2]))
meta_path = out / f'{a.tag}-environment.json'
if meta_path.exists():
    raise RuntimeError(f'Refusing to overwrite {meta_path}; select a fresh tag')
archive = out / f'{a.tag}-implementation.tar.gz'
with tarfile.open(archive, 'w:gz') as tar:
    for top in ['Cargo.toml', 'Cargo.lock', 'crates', 'scripts', 'tests', 'benchmarks/nasdaq/profiles']:
        tar.add(ROOT / top, arcname=top)
meta['implementation_archive'] = archive.name
frozen = out / 'binaries' / meta['binary_sha256']
frozen.parent.mkdir(exist_ok=True)
if not frozen.exists():
    shutil.copy2(binary, frozen)
meta['frozen_binary'] = str(frozen.relative_to(out))
meta_path.write_text(json.dumps(meta, indent=2) + '\n')
base = [str(frozen), '--out', a.out, '--input', a.input, '--workers', str(a.workers), '--tag', a.tag]

# Hold an idle-sleep assertion throughout the matrix. Explicit user sleep can still
# suspend the machine; independent wall/monotonic checks reject affected timings.
awake = subprocess.Popen(['caffeinate', '-i', '-w', str(__import__('os').getpid())]) if platform.system() == 'Darwin' else None
try:
    if a.start_after_file:
        print('WAIT prerequisite', a.start_after_file, flush=True)
        while not (ROOT / a.start_after_file).exists():
            time.sleep(1)
    if a.prepare:
        with (logs / f'{a.tag}-prepare.stdout').open('w') as stdout, (logs / f'{a.tag}-prepare.time').open('w') as stderr:
            print('RUN full source census and deterministic time-window analysis', flush=True)
            subprocess.run(['/usr/bin/time', '-l', *base, '--prepare'], cwd=ROOT, stdout=stdout, stderr=stderr, check=True)
        with (out / 'profiles/cli-analysis.json').open('w') as stdout:
            subprocess.run([str(ROOT / 'target/release/strata'), 'analyze', str(out / 'samples'), '--profile', str(out / 'profiles/manual.toml'), '--format', 'json'], cwd=ROOT, stdout=stdout, check=True)
        print('PASS source census, real optimizer candidates and strata analyze', flush=True)
    for tier in a.tiers:
        profiles = a.full_profiles if tier == 'full' else a.profiles
        for target, label in [('strata', x) for x in profiles] + [(x, x) for x in a.targets]:
            name = f'{a.tag}-{tier}-{label}-w{a.workers}'
            report_path = out / 'results' / f'{name}.json'
            if report_path.exists():
                raise RuntimeError(f'Refusing to overwrite {report_path}')
            cmd = [*base, '--tier', tier, '--target', target, '--profile', label if target == 'strata' else 'manual']
            print('RUN', name, flush=True)
            start = time.monotonic()
            with (logs / f'{name}.stdout').open('w') as stdout, (logs / f'{name}.time').open('w') as stderr:
                subprocess.run(['/usr/bin/time', '-l', *cmd], cwd=ROOT, stdout=stdout, stderr=stderr, check=True)
            report = json.loads(report_path.read_text())
            assert report['release_build'] and report['records'] == report['verified_records'] and report['timing_valid']
            timing = (logs / f'{name}.time').read_text()
            wall = re.search(r'([\d.]+) real\s+([\d.]+) user\s+([\d.]+) sys', timing)
            rss = re.search(r'(\d+)\s+maximum resident set size', timing)
            usage = {'process_monotonic_seconds': time.monotonic() - start, 'peak_rss_bytes': int(rss[1]) if rss else None}
            if wall:
                real, user, system = map(float, wall.groups())
                usage.update(wall_seconds=real, user_seconds=user, system_seconds=system, cpu_percent_of_one_core=(user + system) / real * 100 if real else None)
            report['process_usage'] = usage
            report['environment'] = meta_path.name
            report['timing_valid'] = bool(wall) and abs(usage['wall_seconds'] - usage['process_monotonic_seconds']) < max(1, usage['process_monotonic_seconds'] * .05)
            report_path.write_text(json.dumps(report, indent=2) + '\n')
            if not report['timing_valid']:
                raise RuntimeError(f'System suspension/clock discrepancy: {name}; sizes and hashes retained; rates invalid')
            print(f"PASS {name}: {report['records']} verified rows; {report['sizes']['physical_bytes']} bytes; RSS={usage['peak_rss_bytes']}", flush=True)
    print('ALL REQUESTED REAL-DATA RUNS COMPLETED', flush=True)
finally:
    if awake is not None:
        awake.terminate()
        awake.wait()
