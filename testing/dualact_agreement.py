"""Compare trainer eval TSV (raw output) with engine nnue cp, without running a trainer.
Usage: py -3 testing/dualact_agreement.py trainer-eval.tsv trained.nnue
"""
from pathlib import Path
import sys,re,statistics,json
sys.dont_write_bytecode=True
from dualact_verify import launch,send,until,close,OUT,health
rows=[]
raw=Path(sys.argv[1]).read_bytes()
text=raw.decode('utf-16') if raw.startswith((b'\xff\xfe',b'\xfe\xff')) else raw.decode('utf-8-sig')
for line in text.splitlines():
    if re.match(r'^-?\d+\.\d+\t',line):
        value,fen=line.split('\t',1);rows.append((float(value)*400,fen))
if not rows:raise ValueError('No trainer TSV rows')
p=launch(OUT/'dualact.exe',Path(sys.argv[2]).resolve())
errors=[]
for cp,fen in rows:
    health()
    send(p,'position fen '+fen);send(p,'eval');text=until(p,r'^nnue:')
    match=re.search(r'nnue:\s+(-?\d+) cp',text)
    if not match:raise RuntimeError(text)
    engine=int(match[1]);err=engine-cp;errors.append(err)
    print(f'{cp:.4f}\t{engine}\t{err:.4f}\t{fen}')
close(p)
a=sorted(map(abs,errors))
print(json.dumps({'positions':len(rows),'mean_signed_cp':statistics.mean(errors),'mean_abs_cp':statistics.mean(a),'p95_abs_cp':a[min(len(a)-1,int(.95*len(a)))],'max_abs_cp':max(a),'rms_cp':(statistics.mean(e*e for e in errors))**.5}))
