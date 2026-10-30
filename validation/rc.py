"""v26rc offline build, parity, reproductions and paired timings. No other checkout writes."""
import argparse
import ctypes
import hashlib
import json
import os
from pathlib import Path
import queue
import shutil
import subprocess
import sys
import threading
import time
import interleave
import run

ROOT = run.ROOT
OUT = ROOT / 'validation'
CONTROL = run.LAB / 'health/control.txt'
FIXES = ('FixMate50', 'FixTbPly', 'FixR50Tt')
FLAGS = subprocess.BELOW_NORMAL_PRIORITY_CLASS | subprocess.CREATE_NO_WINDOW

def paused():
    return 'pause' in CONTROL.read_text().lower()

def monitored(cmd, log, cwd=ROOT, env=None):
    """Only stop this command's own process tree on health pause, then retry."""
    first = True
    while True:
        run.health()
        with (OUT / log).open('w' if first else 'a') as stream:
            first = False
            p = subprocess.Popen(cmd, cwd=cwd, env=env, stdout=stream,
                                 stderr=subprocess.STDOUT, creationflags=FLAGS)
            retry = False
            while p.poll() is None:
                if paused():
                    subprocess.run(['taskkill', '/PID', str(p.pid), '/T', '/F'],
                                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                                   creationflags=FLAGS)
                    p.wait()
                    retry = True
                    break
                time.sleep(.2)
        if retry:
            run.health()
            continue
        if p.returncode:
            raise RuntimeError(f'Command failed ({p.returncode}); see validation/{log}')
        break

def build():
    reference = OUT / 'rc-reference'
    base = subprocess.check_output(['git', 'rev-parse', 'c73a3fe'], cwd=ROOT, text=True).strip()
    if not reference.exists():
        reference.mkdir()
        monitored(['git', 'archive', '--format=tar', '-o', str(OUT/'rc-reference.tar'), base], 'rc-archive.log')
        monitored(['tar', '-xf', str(OUT/'rc-reference.tar'), '-C', str(reference)], 'rc-archive.log')
    env = dict(os.environ, RUSTFLAGS='-C target-cpu=native',
               RUST_TEST_THREADS='1', RUST_MIN_STACK='67108864')
    for kind, source in [('reference', reference), ('candidate', ROOT)]:
        for feature in ['normal', 'frozen']:
            common = ['--release', '--offline', '-j', '4', '--manifest-path', str(source/'Cargo.toml'),
                      '--target-dir', str(OUT/('target-rc-'+kind))]
            if feature == 'frozen':
                common += ['--features', 'frozen']
            for action in ['build', 'test']:
                monitored(['cargo', action] + common, f'rc-{kind}-{feature}-{action}.log', env=env)
                print(action, kind, feature, 'passed', flush=True)
            shutil.copy2(OUT/('target-rc-'+kind)/'release/pawnstorm.exe', OUT/f'rc-{kind}-{feature}.exe')
    (OUT/'rc-build.json').write_text(json.dumps(dict(reference=base, flags=env['RUSTFLAGS'],
        binaries={p.name: hashlib.sha256(p.read_bytes()).hexdigest() for p in OUT.glob('rc-*.exe')},
        net_sha256=hashlib.sha256((run.LAB/'engines/x25-big7.nnue').read_bytes()).hexdigest(),
        frozen_options_sha256=hashlib.sha256((ROOT/'frozen.options').read_bytes()).hexdigest()), indent=2))

class Engine(interleave.Engine):
    def __init__(self, exe, variant=('defaults', 0)):
        super().__init__(exe, variant)
        self._queue = queue.Queue()
        self.log = []
        def reader():
            for line in self.p.stdout:
                line = line.strip()
                if not line.startswith('id author '):
                    self.log.append(line)
                    self._queue.put(line)
        threading.Thread(target=reader, daemon=True).start()
        if Path(exe).name == 'rc-on.exe':
            self.fixes(1)

    def until(self, prefix):
        if not hasattr(self, '_queue'):
            return super().until(prefix)
        lines = []
        deadline = time.monotonic() + 600
        while time.monotonic() < deadline:
            if paused():
                # Suspend only our own engine; all its search/helper threads stop.
                handle = ctypes.c_void_p(int(self.p._handle))
                assert ctypes.windll.ntdll.NtSuspendProcess(handle) == 0
                try:
                    run.health()
                finally:
                    assert ctypes.windll.ntdll.NtResumeProcess(handle) == 0
                deadline = time.monotonic() + 600
            try:
                line = self._queue.get(timeout=.1)
            except queue.Empty:
                if self.p.poll() is not None:
                    raise RuntimeError('Engine exited')
                continue
            lines.append(line)
            if line.startswith(prefix):
                return lines
        raise TimeoutError(prefix)

    def fixes(self, value):
        for name in FIXES:
            self.send(f'setoption name {name} value {value}')
        self.ready()

    def threads(self, count):
        assert count in (1, 4)
        assert ctypes.windll.kernel32.SetProcessAffinityMask(ctypes.c_void_p(int(self.p._handle)), (1 << count)-1)
        self.send(f'setoption name Threads value {count}')
        self.ready()

def parity():
    interleave.Engine = Engine
    for feature in ['normal', 'frozen']:
        for depth, count in [(15, 12), (17, 8)]:
            sys.argv = ['interleave.py', str(OUT/f'rc-parity-{feature}-d{depth}.json'),
                        str(OUT/f'rc-reference-{feature}.exe'), str(OUT/f'rc-candidate-{feature}.exe'),
                        '--depth', str(depth), '--count', str(count), '--pairs', '1']
            interleave.main()
        e = Engine(OUT/f'rc-candidate-{feature}.exe')
        try:
            e.send('uci')
            lines = e.until('uciok')
            for name in FIXES:
                assert f'option name {name} type spin default 0 min 0 max 1' in lines
            if feature == 'frozen':
                for name in FIXES:
                    e.send(f'setoption name {name} value 1')
                    e.send('isready')
                    assert e.until('readyok') == [f'info string {name} is frozen at 0 in this build', 'readyok']
        finally:
            e.close()
    print('40 per-position comparisons passed; default-off options advertised in both builds', flush=True)

def repro():
    env = dict(os.environ, RUST_TEST_THREADS='1', RUST_MIN_STACK='67108864')
    # Normal-feature test executable already built by build(). Do not toggle frozen.options.
    monitored(['cargo', 'test', '--release', '--offline', '-j', '4', '--target-dir', str(OUT/'target-rc-candidate'),
               '--lib', 'rc_bughunt_tb_reproduction', '--', '--ignored', '--nocapture', '--test-threads=1'],
              'rc-tb-reproduction.log', env=env)
    print((OUT/'rc-tb-reproduction.log').read_text(), flush=True)
    rows = []
    e = Engine(OUT/'rc-candidate-normal.exe')
    try:
        for threads in [1, 4]:
            e.threads(threads)
            for fix in [0, 1]:
                e.fixes(fix)
                for clock in [98, 99]:
                    r = e.search(f'7k/8/5KQ1/8/8/8/8/8 w - - {clock} 1', 3)
                    if fix or clock == 98:
                        assert r['best'].split()[1] == 'g6g7' and 'mate 1' in r['score']
                    else:
                        assert r['score'] == ' score cp 0'
                    rows.append(dict(threads=threads, all_on=fix, clock=clock, syzygy=False, **r))
                    print('MATE50', threads, fix, clock, r['score'], r['best'], flush=True)
            e.fixes(1)
            e.send(f'setoption name SyzygyPath value {run.LAB / "syzygy"}')
            e.ready()
            for clock in [0, 98, 99]:
                # Root DTZ bypasses TT score decoding; check its legal boundary move.
                r = e.search(f'7k/8/5KQ1/8/8/8/8/8 w - - {clock} 1', 1)
                assert r['best'].split()[1] == 'g6g7'
                rows.append(dict(threads=threads, all_on=1, clock=clock, syzygy=True, **r))
                print('ROOT_DTZ', threads, clock, r['score'], r['best'], flush=True)
            # A six-man root (outside local five-man coverage), capturing into
            # a probed five-man result exercises the actual SMP search/TB path.
            r = e.search('7k/8/5K2/8/8/8/pr6/RR6 w - - 0 1', 6)
            assert abs(int(r['score'].split()[-1])) >= 30615, r
            rows.append(dict(threads=threads, all_on=1, syzygy=True, **r))
            e.send('setoption name SyzygyPath value <empty>')
            e.ready()
    finally:
        (OUT/'rc-reproduction-uci.json').write_text(json.dumps(rows, indent=2))
        (OUT/'rc-reproduction-uci.log').write_text('\n'.join(e.log)+'\n')
        e.close()

def speed():
    # Same executable, separate names let the existing paired harness identify
    # the all-off and all-on processes. At most two processes, one searching.
    for name in ['rc-off.exe', 'rc-on.exe']:
        shutil.copy2(OUT/'rc-candidate-normal.exe', OUT/name)
    interleave.Engine = Engine
    sys.argv = ['interleave.py', str(OUT/'rc-speed.json'), str(OUT/'rc-off.exe'), str(OUT/'rc-on.exe'),
                '--depth', '20', '--count', '12', '--pairs', '6']
    interleave.main()
    summarize_speed('rc-speed')

def summarize_speed(stem):
    rows = json.loads((OUT/f'{stem}.json').read_text())
    import statistics
    summary = {}
    for metric in ['seconds', 'cpu_seconds']:
        rates = [[sum(r['nodes'] for r in rs)/sum(r[metric] for r in rs) for rs in pair.values()] for pair in rows]
        ratios = [100*(b/a-1) for a,b in rates]
        summary[metric] = dict(off_median=statistics.median(a for a,b in rates),
            on_median=statistics.median(b for a,b in rates), paired_percent=ratios,
            paired_median=statistics.median(ratios), minimum=min(ratios), maximum=max(ratios))
    (OUT/f'{stem}-summary.json').write_text(json.dumps(summary, indent=2))
    print(json.dumps(summary, indent=2), flush=True)

def speed4():
    # SMP visits different trees even with identical settings. Measure aggregate
    # reported nodes/time, retaining every sample; do not demand node parity.
    engines = [Engine(OUT/'rc-candidate-normal.exe') for _ in range(2)]
    result = []
    try:
        for i, engine in enumerate(engines):
            engine.threads(4)
            engine.fixes(i)
            engine.search(run.FENS[0], 15)
        for pair in range(6):
            rows = {'off': [], 'on': []}
            for i, fen in enumerate(run.FENS[:12]):
                for which in ([0, 1] if (pair+i)%2 == 0 else [1, 0]):
                    rows[['off', 'on'][which]].append(engines[which].search(fen, 20))
            result.append(rows)
            (OUT/'rc-speed4.json').write_text(json.dumps(result, indent=2))
            print('SMP pair', pair, {k: interleave.round_rate(v) for k,v in rows.items()}, flush=True)
    finally:
        for engine in engines:
            engine.close()
    (OUT/'rc-speed4.meta.json').write_text(json.dumps(dict(threads=4, affinity_mask=15,
        depth=20, pairs=6, fens=run.FENS[:12], options=run.OPTIONS,
        fixes=dict(off=0, on=1), binary_sha256=hashlib.sha256((OUT/'rc-candidate-normal.exe').read_bytes()).hexdigest(),
        net_sha256=hashlib.sha256((run.LAB/'engines/x25-big7.nnue').read_bytes()).hexdigest()), indent=2))
    summarize_speed('rc-speed4')

if __name__ == '__main__':
    args = argparse.ArgumentParser()
    args.add_argument('action', choices=['build', 'parity', 'repro', 'speed', 'speed4'])
    globals()[args.parse_args().action]()
