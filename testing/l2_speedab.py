"""Pinned CPU-only alternating equal-tree speed test; all paths remain in the lab.
Run from lab root; see L2FAST.md for commands. Requires cached system Python only.
"""
import sys
sys.dont_write_bytecode=True
import argparse,json,re,statistics,time
from pathlib import Path
from dualact_verify import ROOT,HERE,FENS,OUT,launch,send,until,close,health

def run(exe,net,depth,positions,repeats,cpu):
    p=launch(exe,net,affinity=1<<cpu);send(p,'setoption name Threads value 1');nodes=[];elapsed=0;times=[];engine_ms=[]
    try:
        for fen in FENS[:positions] * repeats:
            health();send(p,'ucinewgame');send(p,'position fen '+fen)
            send(p,'isready');until(p,'^readyok')
            start=time.perf_counter();send(p,f'go depth {depth}')
            log=until(p,'^bestmove ');dt=time.perf_counter()-start;elapsed+=dt;times.append(dt)
            info=re.findall(rf'^info depth {depth} .*? nodes (\d+)',log,re.M)
            if not info:raise RuntimeError(log)
            nodes.append(int(info[-1]))
            engine_ms.append(int(re.findall(rf"^info depth {depth} .*? time (\d+)",log,re.M)[-1]))
    finally:close(p)
    return {'nodes':nodes,'total_nodes':sum(nodes),'seconds':elapsed,'nps':sum(nodes)/elapsed,'position_seconds':times,'engine_ms':engine_ms,'engine_nps':sum(nodes)*1000/max(1,sum(engine_ms))}

if __name__=='__main__':
    ap=argparse.ArgumentParser();ap.add_argument('--aexe',type=Path,required=True);ap.add_argument('--anet',type=Path,required=True);ap.add_argument('--bexe',type=Path,required=True);ap.add_argument('--bnet',type=Path,required=True);ap.add_argument('--out',type=Path,required=True);ap.add_argument('--pairs',type=int,default=6);ap.add_argument('--depth',type=int,default=15);ap.add_argument('--positions',type=int,default=8);ap.add_argument('--repeats',type=int,default=1);ap.add_argument('--cpu',type=int,default=2);args=ap.parse_args()
    sets={'a':(args.aexe.resolve(),args.anet.resolve()),'b':(args.bexe.resolve(),args.bnet.resolve())};pairs=[]
    for i in range(args.pairs+1):
        pair={}
        for label in (['a','b'] if i%2==0 else ['b','a']):
            exe,net=sets[label];pair[label]=run(exe,net,args.depth,args.positions,args.repeats,args.cpu)
        assert pair['a']['nodes']==pair['b']['nodes'],pair
        ratio=pair['b']['nps']/pair['a']['nps'];pair['ratio']=ratio;pair['engine_ratio']=pair['b']['engine_nps']/pair['a']['engine_nps']
        print('warmup' if i==0 else i,pair['a']['total_nodes'],round(pair['a']['nps']),round(pair['b']['nps']),round(ratio,6),flush=True)
        if i:pairs.append(pair)
    result={'depth':args.depth,'cpu':args.cpu,'repeats':args.repeats,'positions':FENS[:args.positions],'pairs':pairs,'median_ratio':statistics.median(p['ratio'] for p in pairs),'min_ratio':min(p['ratio'] for p in pairs),'max_ratio':max(p['ratio'] for p in pairs),'median_engine_ratio':statistics.median(p['engine_ratio'] for p in pairs)}
    args.out.write_text(json.dumps(result,indent=2));print('RESULT',result['median_ratio'],result['min_ratio'],result['max_ratio'],flush=True)
