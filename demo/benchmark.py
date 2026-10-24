"""Reproducible bounded-memory benchmark; the verifier does not load the output."""
import argparse
from contextlib import ExitStack
import json
import os
from pathlib import Path
import subprocess
import time

parser = argparse.ArgumentParser()
parser.add_argument('--exe', required=True)
parser.add_argument('--mib', type=int, default=512)
parser.add_argument('--memory', default='128MiB')
parser.add_argument('--threads', type=int, default=2)
parser.add_argument('--directory', required=True)
parser.add_argument('--outfil', action='store_true', help='Also split the sorted stream into two OUTFIL outputs')
parser.add_argument('--reformat', action='store_true', help='Exercise field comparison and INREC/OUTREC OVERLAY, verifying every changed byte')
args = parser.parse_args()
exe = Path(args.exe)
if os.name == 'nt' and not exe.suffix:
    exe = exe.with_suffix('.exe')
exe = exe.resolve(strict=True)
root = Path(args.directory).resolve()
root.mkdir(parents=True, exist_ok=True)
source, output = root / 'input.fixed', root / 'output.fixed'
if source.exists() or output.exists():
    raise SystemExit('Use an empty benchmark directory; existing data is never overwritten')
width = 256
count = args.mib * 1024 * 1024 // width
if count <= 0:
    raise SystemExit('mib must be positive')
started = time.monotonic()
with source.open('wb', buffering=1024*1024) as file:
    for i in range(count):
        key = (count-i) % 100003
        file.write(f'{key:08}{i:016}'.encode() + b'.' * (width-24))
generation = time.monotonic() - started
started = time.monotonic()
command = [str(exe), str(source), '-o', str(output), '--record-length', str(width),
    '--memory', args.memory, '--threads', str(args.threads), '--stats']
if args.outfil or args.reformat:
    control=root/'sort.ctl'
    if control.exists() or (root/'part1.fixed').exists() or (root/'part2.fixed').exists():
        raise SystemExit('OUTFIL benchmark paths already exist')
    cards = ('INCLUDE COND=(25,1,CH,EQ,26,1,CH)\n INREC OVERLAY=(256:C\'!\')\n' if args.reformat else '')
    cards += 'SORT FIELDS=(1,8,CH,A)\n'
    if args.reformat:
        cards += " OUTREC OVERLAY=(255:C'?')\n"
    if args.outfil:
        cards += ' OUTFIL FNAMES=(PART1,PART2),SPLIT\n'
    control.write_text(cards, encoding='utf-8')
    command += ['--control',str(control)]
    if args.outfil:
        command += ['--out',f'PART1={root/"part1.fixed"}','--out',f'PART2={root/"part2.fixed"}']
else:
    command += ['--key','1:8:CH:A']
process=subprocess.Popen(command,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True,
    start_new_session=os.name!='nt',creationflags=subprocess.CREATE_NEW_PROCESS_GROUP if os.name=='nt' else 0)
try:
    stdout,stderr=process.communicate(timeout=600)
except subprocess.TimeoutExpired:
    if os.name=='nt':
        subprocess.run(['taskkill','/PID',str(process.pid),'/T','/F'],capture_output=True,check=False)
    else:
        import signal
        os.killpg(process.pid,signal.SIGKILL)
    process.communicate()
    raise
result=subprocess.CompletedProcess(command,process.returncode,stdout,stderr)
elapsed = time.monotonic() - started
if result.returncode:
    raise SystemExit(result.stderr)
stats = json.loads(result.stderr)
if stats['initial_runs'] < 2:
    raise AssertionError('Benchmark did not exercise external spill')
if stats['resources']['affinity_cpus'] != args.threads:
    raise AssertionError('CPU affinity not applied')
seen = bytearray((count+7)//8)
previous = None
expected_payload = b'.' * (width-26) + b'?!' if args.reformat else b'.' * (width-24)
with ExitStack() as stack:
    file=stack.enter_context(output.open('rb',buffering=1024*1024))
    parts=[stack.enter_context((root/f'part{i}.fixed').open('rb',buffering=1024*1024)) for i in (1,2)] if args.outfil else []
    for ordinal in range(count):
        record = file.read(width)
        if parts and parts[ordinal%2].read(width)!=record:
            raise AssertionError('OUTFIL split record differs from sorted main output')
        if len(record) != width:
            raise AssertionError('truncated output')
        key, seq = int(record[:8]), int(record[8:24])
        if not 0 <= seq < count or key != (count-seq) % 100003 or record[24:] != expected_payload:
            raise AssertionError('record data changed')
        bit = 1 << (seq % 8)
        if seen[seq//8] & bit:
            raise AssertionError('duplicate record')
        seen[seq//8] |= bit
        if previous is not None and previous >= (key, seq):
            raise AssertionError('incorrect ordering or unstable equal keys')
        previous = (key, seq)
    if any(part.read(1) for part in parts):
        raise AssertionError('extra OUTFIL split records')
    if file.read(1):
        raise AssertionError('extra records')
report = dict(outfil_split=args.outfil, reformat=args.reformat, input_bytes=count*width, record_bytes=width, records=count,
    generation_seconds=generation, wall_seconds=elapsed,
    mib_per_second=args.mib/elapsed, requested_memory=args.memory, stats=stats,
    verified='all records, expected payloads, no duplicates/omissions, stable key order' + (', both OUTFIL split streams' if args.outfil else '') + (', field comparison and all overlay bytes' if args.reformat else ''))
(root/'result.json').write_text(json.dumps(report, ensure_ascii=False, indent=2)+'\n', encoding='utf-8')
print(json.dumps(report, ensure_ascii=False, indent=2))
