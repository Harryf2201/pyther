"""Confirm generated starts with exactly EPD's reset counters; retain in-band results.

Run after build-unbalanced-book.py finishes. Stdlib only, same six-process resource cap.
The raw journal/config remain untouched; final provenance retains the original eval.
"""
import argparse
import concurrent.futures as cf
import ctypes
import importlib.util
import json
import pathlib
import random
import threading
import time
from collections import Counter

spec=importlib.util.spec_from_file_location('builder',pathlib.Path(__file__).with_name('build-unbalanced-book.py'))
b=importlib.util.module_from_spec(spec); spec.loader.exec_module(b)

def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--prefix',default='pyther-unbal-v1')
    p.add_argument('--workers',type=int,choices=range(1,7),default=6)
    p.add_argument('--wait-for-generation',action='store_true')
    a=p.parse_args()
    ctypes.windll.kernel32.SetPriorityClass(ctypes.windll.kernel32.GetCurrentProcess(),0x4000)
    base=b.LAB/'books'/a.prefix
    if a.wait_for_generation:
        while not base.with_suffix('.stats.json').exists(): time.sleep(1)
    config=json.loads(base.with_suffix('.config.json').read_text())
    args=argparse.Namespace(**config['arguments'])
    args.workers=a.workers
    # Ensure the raw journal really belongs to the supplied engine/net/options/code.
    for relative,digest in config['sha256'].items():
        if b.sha(b.ROOT/relative)!=digest: raise RuntimeError('input changed: '+relative)
    rows=[json.loads(line) for line in base.with_suffix('.journal.jsonl').read_text().splitlines()]
    seen=set(); raw=[]
    for r in sorted(rows,key=lambda r:r['source']):
        if not r['accepted']: continue
        key=' '.join(r['fen'].split()[:4])
        if key not in seen: raw.append(r); seen.add(key)
    journal=base.with_suffix('.confirmation.jsonl')
    confirmed=[json.loads(line) for line in journal.read_text().splitlines()] if journal.exists() else []
    done={r['source'] for r in confirmed}
    local=threading.local(); workers=[]; lock=threading.Lock()
    def task(r):
        if not hasattr(local,'worker'):
            local.worker=b.Worker(args,config['options'])
            with lock: workers.append(local.worker)
        fen=' '.join(r['fen'].split()[:4])+' 0 1'
        board=local.worker.apply(fen)
        if not board.safe(): raise RuntimeError('unsafe generated position')
        ev=local.worker.search(fen,args.nodes)
        white=ev['cp']*(1 if fen.split()[1]=='w' else -1)
        return dict(source=r['source'],fen=fen,evaluation=ev,white_cp=white,
            accepted=ev['kind']=='cp' and args.low<=abs(white)<=args.high)
    started=time.monotonic(); pool=cf.ThreadPoolExecutor(a.workers)
    try:
        with journal.open('a',buffering=1) as out:
            # executor.map queues work but only six worker-owned engines can search.
            for r in pool.map(task,[r for r in raw if r['source'] not in done]):
                out.write(json.dumps(r,separators=(',',':'))+'\n'); confirmed.append(r)
                if len(confirmed)%500==0: b.say(f'confirmed={len(confirmed)} elapsed_s={time.monotonic()-started:.0f}')
    finally:
        b.STOP.set(); pool.shutdown(wait=True,cancel_futures=True)
        for w in workers: w.close()
    lookup={r['source']:r for r in confirmed}; final=[]
    for r in raw:
        c=lookup[r['source']]
        if not c['accepted']: continue
        r['generation_fen']=r['fen']; r['generation_evaluation']=r['evaluation']
        r['generation_white_cp']=r['white_cp']
        r.update(fen=c['fen'],evaluation=c['evaluation'],white_cp=c['white_cp'],stm_cp=c['evaluation']['cp'])
        r['trace'].append(dict(stage='epd_counter_confirmation',fen=c['fen'],evaluation=c['evaluation'],white_cp=c['white_cp']))
        final.append(r)
    random.Random(args.seed).shuffle(final)
    base.with_suffix('.epd').write_text(''.join(' '.join(r['fen'].split()[:4])+f' id "pyther-unbal-v1-{r["source"]}";\n' for r in final),encoding='ascii')
    with base.with_suffix('.provenance.jsonl').open('w') as out:
        for r in final: out.write(json.dumps(r,separators=(',',':'))+'\n')
    scores=sorted(abs(r['white_cp']) for r in final)
    stats=dict(source_count=config['source_count'],processed_sources=len(rows),
        generated_accepted_before_dedup=sum(r['accepted'] for r in rows),
        generated_unique=len(raw),canonical_confirmations=len(confirmed),
        rejected_after_counter_reset=len(raw)-len(final),unique=len(final),
        abs_cp=dict(min=min(scores,default=0),max=max(scores,default=0),mean=sum(scores)/max(1,len(scores)),
            quantiles={str(q):scores[min(len(scores)-1,int(q*(len(scores)-1)))] for q in [.1,.25,.5,.75,.9]} if scores else {}),
        depths=dict(Counter(r['evaluation']['depth'] for r in final)),
        completed_iteration_nodes=dict(min=min(r['evaluation']['nodes'] for r in final),max=max(r['evaluation']['nodes'] for r in final)),
        extensions=dict(Counter(len(r['extension']) for r in final)),
        better_side=dict(Counter('white' if r['white_cp']>0 else 'black' for r in final)),
        stm=dict(Counter(r['fen'].split()[1] for r in final)),
        final_counter_convention='EPD defaults: halfmove 0, fullmove 1',confirmation_elapsed_s=time.monotonic()-started,
        epd_sha256=b.sha(base.with_suffix('.epd')),provenance_sha256=b.sha(base.with_suffix('.provenance.jsonl')),
        confirmation_script_sha256=b.sha(pathlib.Path(__file__)))
    base.with_suffix('.stats.json').write_text(json.dumps(stats,indent=2)+'\n')
    base.with_suffix('.confirmed').write_text('complete\n')
    b.say(json.dumps(stats))
    if not 10000<=len(final)<=20000: raise RuntimeError('final count outside requested range')

if __name__=='__main__': main()
