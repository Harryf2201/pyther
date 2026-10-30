"""Local measurement book, stdlib only. Never calls other engines or the network.

Run from the lab parent; see --help. Board adapter is compiled separately using rustc.
The journal permits restart with identical options. Use a different prefix for pilots.
"""
import argparse
import concurrent.futures as cf
import ctypes
import hashlib
import json
import pathlib
import queue
import random
import subprocess
import threading
import time
from collections import Counter

ROOT = pathlib.Path(__file__).resolve().parents[2]
LAB = ROOT / 'testing'
CONTROL = LAB / 'health/control.txt'
FLAGS = subprocess.CREATE_NO_WINDOW | subprocess.BELOW_NORMAL_PRIORITY_CLASS
STOP = threading.Event()
PRINT_LOCK = threading.Lock()

def say(s):
    with PRINT_LOCK:
        print(s, flush=True)

def paused():
    # Missing/unreadable control fails closed. No writes to the health controller.
    try:
        return 'pause' in CONTROL.read_text().lower()
    except OSError:
        return True

def wait_health():
    while paused():
        if STOP.wait(0.2):
            raise InterruptedError('stopped')
    if STOP.is_set():
        raise InterruptedError('stopped')

def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()

class Board:
    def __init__(self, line):
        self.fen, check, material, pieces, moves = line.strip().split('\t')
        self.check = check == 'true'
        self.material = int(material)
        self.pieces = int(pieces)
        self.moves = moves.split()
    def safe(self):
        return not self.check and abs(self.material) <= 300 and self.pieces > 12 and len(self.moves) >= 2

class Worker:
    def __init__(self, args, options):
        self.args = args
        wait_health()
        self.board = subprocess.Popen([str(LAB/'books/book-board.exe')], stdin=subprocess.PIPE,
            stdout=subprocess.PIPE, text=True, creationflags=FLAGS)
        self.engine = subprocess.Popen([str(LAB/'engines/pawnstorm-xsearch5.exe')],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
            text=True, bufsize=1, creationflags=FLAGS)
        self.lines = queue.Queue()
        def reader():
            for line in self.engine.stdout:
                self.lines.put(line.strip())
            self.lines.put(None)
        threading.Thread(target=reader, daemon=True).start()
        self.send('uci')
        advertised = set()
        while True:
            line = self.read()
            if line == 'uciok': break
            if line.startswith('option name '): advertised.add(line[12:].split(' type ')[0].lower())
        for key, value in options.items():
            if key.lower() not in advertised:
                raise RuntimeError('engine does not advertise option '+key)
            self.send(f'setoption name {key} value {value}')
        self.send('setoption name EvalFile value '+str(LAB/'engines/x26d-p25.nnue'))
        self.send('setoption name Threads value 1')
        self.send('setoption name Hash value 16')
        self.send('isready')
        loaded = False
        while True:
            line = self.read()
            loaded |= line.startswith('info string loaded network ')
            if line == 'readyok': break
        if not loaded: raise RuntimeError('network load not confirmed')
    def send(self, line):
        self.engine.stdin.write(line+'\n'); self.engine.stdin.flush()
    def read(self):
        line = self.lines.get(timeout=120)
        if line is None: raise RuntimeError('engine exited')
        return line
    def apply(self, fen, moves=''):
        self.board.stdin.write(fen+'\t'+moves+'\n'); self.board.stdin.flush()
        return Board(self.board.stdout.readline())
    def search(self, fen, nodes):
        while True:
            wait_health()
            self.send('ucinewgame')
            self.send('position fen '+fen)
            self.send(f'go nodes {nodes}')
            last, roots, interrupted = None, [], False
            started = time.monotonic()
            while True:
                if (paused() or STOP.is_set()) and not interrupted:
                    self.send('stop'); interrupted = True
                try: line = self.lines.get(timeout=0.1)
                except queue.Empty:
                    if time.monotonic()-started > 120: raise TimeoutError('search timeout')
                    continue
                if line is None: raise RuntimeError('engine exited during search')
                if line.startswith('info depth ') and ' score ' in line and ' pv ' in line:
                    t = line.split()
                    if 'lowerbound' in t or 'upperbound' in t: continue
                    score = t[t.index('score')+1:t.index('score')+3]
                    last = dict(kind=score[0], cp=int(score[1]), depth=int(t[t.index('depth')+1]),
                        nodes=int(t[t.index('nodes')+1]), pv=t[t.index('pv')+1:])
                    roots.append(last['pv'][0])
                if line.startswith('bestmove '): break
            if interrupted:
                wait_health(); continue
            if last is None: raise RuntimeError('no completed iteration')
            last['root_candidates'] = list(dict.fromkeys(reversed(roots)))
            return last
    def process(self, source):
        index, ucis, b = source
        rng = random.Random(self.args.seed+index)
        extension, trace = [], []
        for ply in range(5):
            if b.safe():
                ev = self.search(b.fen, self.args.nodes)
                white = ev['cp'] * (1 if b.fen.split()[1]=='w' else -1)
                trace.append(dict(ply=ply, fen=b.fen, evaluation=ev, white_cp=white))
                if ev['kind']=='cp' and self.args.low <= abs(white) <= self.args.high:
                    return dict(source=index, source_uci=ucis, extension=extension, fen=b.fen,
                        white_cp=white, stm_cp=ev['cp'], evaluation=ev, trace=trace, accepted=True)
            else:
                ev = None
            if ply == 4 or not b.moves: break
            # Evaluate a deterministic sample of legal children with OUR engine.
            # Rank from the mover's perspective, retain top six of the sample,
            # and permit at most 140 cp loss against its best sampled choice.
            candidates = list(b.moves); rng.shuffle(candidates)
            candidates = list(dict.fromkeys((ev['root_candidates'][:2] if ev else [])+candidates[:8]))
            ranked = []
            for move in candidates:
                child = self.apply(b.fen, move)
                if abs(child.material)>300 or child.pieces<=12: continue
                ce = self.search(child.fen, self.args.probe_nodes)
                if ce['kind'] != 'cp': continue
                ranked.append((-ce['cp'], move, child, ce))
            if not ranked: break
            ranked.sort(key=lambda x:x[0], reverse=True)
            top = [x for x in ranked[:6] if x[0]>=ranked[0][0]-140]
            target = rng.randint(self.args.low+10, self.args.high-10)
            chosen = min(top, key=lambda x: abs(abs(x[3]['cp'])-target)+(25 if x[2].check else 0))
            trace.append(dict(ply=ply, sampled=[dict(move=x[1], mover_cp=x[0], evaluation=x[3]) for x in ranked],
                chosen=chosen[1]))
            extension.append(chosen[1]); b = chosen[2]
        return dict(source=index, accepted=False, trace=trace)
    def close(self):
        for p, command in [(self.engine,'quit\n'), (self.board,None)]:
            try:
                if command: p.stdin.write(command); p.stdin.flush()
                p.stdin.close(); p.wait(timeout=10)
            except (OSError, subprocess.TimeoutExpired):
                # Only these task-owned children may be terminated.
                p.kill(); p.wait()

def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--prefix', default='pyther-unbal-v1')
    p.add_argument('--target', type=int, default=12000)
    p.add_argument('--limit', type=int, default=34700)
    p.add_argument('--workers', type=int, default=6, choices=range(1,7))
    p.add_argument('--nodes', type=int, default=30000)
    p.add_argument('--probe-nodes', type=int, default=1500)
    p.add_argument('--low', type=int, default=70)
    p.add_argument('--high', type=int, default=130)
    p.add_argument('--seed', type=int, default=20261030)
    args = p.parse_args()
    ctypes.windll.kernel32.SetPriorityClass(ctypes.windll.kernel32.GetCurrentProcess(),0x4000)
    options = dict(line.strip().split('=',1) for line in (LAB/'engines/x25-big7.options').read_text().splitlines() if line.strip())
    sources = []
    for line in (LAB/'books/pyther-unbal-v1.sources.tsv').read_text().splitlines():
        i, ucis, fields = line.split('\t',2)
        sources.append((int(i),ucis,Board(fields)))
    random.Random(args.seed).shuffle(sources)
    sources = sources[:args.limit]
    base = LAB/'books'/args.prefix
    config = dict(arguments=vars(args), sha256={str(f.relative_to(ROOT)).replace('\\','/'):sha(f) for f in
        [LAB/'books/8moves_v3.pgn',LAB/'engines/pawnstorm-xsearch5.exe',LAB/'engines/x26d-p25.nnue',
         LAB/'engines/x25-big7.options',pathlib.Path(__file__),LAB/'books/book-board.exe']}, options=options,
         band_convention='absolute White cp; either better side and either side to move', source_count=34700)
    config_file = base.with_suffix('.config.json')
    if config_file.exists():
        if json.loads(config_file.read_text())!=config: raise RuntimeError('config changed; use new prefix')
    else: config_file.write_text(json.dumps(config,indent=2)+'\n')
    journal = base.with_suffix('.journal.jsonl')
    records = [json.loads(line) for line in journal.read_text().splitlines()] if journal.exists() else []
    done = {r['source'] for r in records}
    seen = {' '.join(r['fen'].split()[:4]) for r in records if r['accepted']}
    remaining = [s for s in sources if s[0] not in done]
    started = time.monotonic()
    local = threading.local(); workers = []; workers_lock = threading.Lock()
    def task(source):
        if not hasattr(local,'worker'):
            local.worker = Worker(args, options)
            with workers_lock: workers.append(local.worker)
        return local.worker.process(source)
    pool = cf.ThreadPoolExecutor(args.workers)
    pending = []; cursor = 0
    try:
        with journal.open('a',buffering=1) as out:
            while len(seen)<args.target and (cursor<len(remaining) or pending):
                while len(pending)<args.workers and cursor<len(remaining):
                    pending.append(pool.submit(task,remaining[cursor])); cursor+=1
                r = pending.pop(0).result()
                out.write(json.dumps(r,separators=(',',':'))+'\n'); records.append(r)
                if r['accepted']: seen.add(' '.join(r['fen'].split()[:4]))
                if len(records)%100==0:
                    say(f'processed={len(records)} unique_accepted={len(seen)} elapsed_s={time.monotonic()-started:.0f}')
    finally:
        STOP.set(); pool.shutdown(wait=True,cancel_futures=True)
        for w in workers: w.close()
    accepted, keys = [], set()
    for r in sorted(records,key=lambda r:r['source']):
        if not r['accepted']: continue
        key = ' '.join(r['fen'].split()[:4])
        if key not in keys: accepted.append(r); keys.add(key)
    random.Random(args.seed).shuffle(accepted)
    accepted = accepted[:args.target]
    base.with_suffix('.epd').write_text(''.join(' '.join(r['fen'].split()[:4])+f' id "pyther-unbal-v1-{r["source"]}";\n' for r in accepted),encoding='ascii')
    with base.with_suffix('.provenance.jsonl').open('w') as out:
        for r in accepted: out.write(json.dumps(r,separators=(',',':'))+'\n')
    scores=sorted(abs(r['white_cp']) for r in accepted)
    stats=dict(processed=len(records),accepted_before_dedup=sum(r['accepted'] for r in records),unique=len(accepted),
        abs_cp=dict(min=min(scores,default=0),max=max(scores,default=0),mean=sum(scores)/max(1,len(scores)),
            quantiles={str(q):scores[min(len(scores)-1,int(q*(len(scores)-1)))] for q in [.1,.25,.5,.75,.9]} if scores else {}),
        depths=dict(Counter(r['evaluation']['depth'] for r in accepted)),
        extensions=dict(Counter(len(r['extension']) for r in accepted)),
        better_side=dict(Counter('white' if r['white_cp']>0 else 'black' for r in accepted)),
        stm=dict(Counter(r['fen'].split()[1] for r in accepted)),elapsed_s=time.monotonic()-started,
        epd_sha256=sha(base.with_suffix('.epd')))
    base.with_suffix('.stats.json').write_text(json.dumps(stats,indent=2)+'\n')
    say(json.dumps(stats))

if __name__=='__main__': main()
