import run, argparse, pathlib, json, hashlib
class Engine:
    def __init__(self, exe, variant=('defaults',0)):
        self.p=run.launch(exe)
        self.send('uci'); self.until('uciok')
        opts=run.OPTIONS | {'Threads':'1','Hash':'16','EvalFile':str(run.LAB/'engines/x25-big7.nnue')}
        if variant[0]!='defaults': opts[variant[0]]=str(variant[1])
        for k,v in opts.items(): self.send(f'setoption name {k} value {v}')
        self.ready()
    def send(self,s): self.p.stdin.write(s+'\n'); self.p.stdin.flush()
    def until(self,prefix):
        lines=[]
        while True:
            s=self.p.stdout.readline().strip()
            if not s and self.p.poll() is not None: raise RuntimeError('Engine exited')
            lines.append(s)
            if s.startswith(prefix): return lines
    def ready(self): self.send('isready'); self.until('readyok')
    def search(self,fen,depth):
        run.health(); self.send('ucinewgame'); self.send('position fen '+fen); self.ready()
        cpu_start=run.cpu(self.p); start=run.time.perf_counter()
        self.send(f'go depth {depth}'); lines=self.until('bestmove')
        elapsed=run.time.perf_counter()-start; cpu=run.cpu(self.p)-cpu_start
        info=[s for s in lines if s.startswith(f'info depth {depth} ')][-1]
        self.ready()
        return dict(fen=fen,nodes=int(run.re.search(r' nodes (\d+)',info)[1]),score=run.re.search(r' score (cp|mate) (-?\d+)',info)[0],best=lines[-1],pv=info.split(' pv ')[-1],seconds=elapsed,cpu_seconds=cpu,info=info)
    def close(self):
        if self.p.poll() is None: self.send('quit'); self.p.wait(timeout=15)
def main():
    a=argparse.ArgumentParser(); a.add_argument('out'); a.add_argument('exes',nargs='+'); a.add_argument('--depth',type=int,default=20); a.add_argument('--pairs',type=int,default=6); a.add_argument('--count',type=int,default=8); args=a.parse_args()
    assert len(args.exes) == 2, 'exactly two engines per comparison'
    engines={}; result=[]
    pathlib.Path(args.out+'.meta.json').write_text(json.dumps(dict(
        depth=args.depth, count=args.count, pairs=args.pairs,
        sha256={f'{i}:{pathlib.Path(e).name}': hashlib.sha256(pathlib.Path(e).read_bytes()).hexdigest() for i,e in enumerate(args.exes)},
        options=run.OPTIONS, threads=1, hash_mb=16,
        net_sha256=hashlib.sha256((run.LAB/'engines/x25-big7.nnue').read_bytes()).hexdigest(),
        fens=run.FENS[:args.count]), indent=2))
    try:
        for e in args.exes: engines[e]=Engine(e)
        # Untimed warm-up before paired measurements.
        for engine in engines.values(): engine.search(run.FENS[0], min(args.depth, 15))
        for round in range(args.pairs):
            rows={e:[] for e in args.exes}
            for i,fen in enumerate(run.FENS[:args.count]):
                order=args.exes if (round+i)%2==0 else args.exes[::-1]
                reference=None
                for e in order:
                    r=engines[e].search(fen,args.depth)
                    if reference is not None:
                        assert all(reference[k]==r[k] for k in ['fen','nodes','score','best','pv']), f'Parity failure {e} {i}'
                    reference=r; rows[e].append(r)
            result.append(rows); pathlib.Path(args.out).write_text(json.dumps(result,indent=2))
            for e,rs in rows.items(): print(round,pathlib.Path(e).name,round_rate(rs),flush=True)
    finally:
        for e in engines.values(): e.close()
def round_rate(rows): return round(sum(r['nodes'] for r in rows)/sum(r['seconds'] for r in rows))
if __name__=='__main__': main()
