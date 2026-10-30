"""Summarize paired whole-suite NPS, retaining per-round ratios and parity evidence."""
import json
from pathlib import Path
import statistics as stats
import sys

root=Path(__file__).resolve().parent
for path in sys.argv[1:]:
    p=Path(path)
    rounds=json.loads(p.read_text())
    rates=[[],[]]; cpu=[[],[]]; ratios=[]; cpu_ratios=[]
    for row in rounds:
        a,b=row.values()
        assert len(a)==len(b)==12
        assert all(all(x[k]==y[k] for k in ['fen','nodes','score','best','pv']) for x,y in zip(a,b))
        for j,rs in enumerate([a,b]):
            n=sum(x['nodes'] for x in rs)
            rates[j].append(n/sum(x['seconds'] for x in rs))
            cpu[j].append(n/sum(x['cpu_seconds'] for x in rs))
        ratios.append(100*(rates[1][-1]/rates[0][-1]-1))
        cpu_ratios.append(100*(cpu[1][-1]/cpu[0][-1]-1))
    out=dict(rounds=len(rounds), nodes=sum(x['nodes'] for x in a),
        reference_median_nps=stats.median(rates[0]), candidate_median_nps=stats.median(rates[1]),
        paired_percent=ratios, paired_median_percent=stats.median(ratios),
        paired_cpu_percent=cpu_ratios,paired_cpu_median_percent=stats.median(cpu_ratios),
        parity='nodes/score/best/ponder/PV identical per position in every round')
    p.with_suffix('.summary.json').write_text(json.dumps(out,indent=2))
    print(p.name, 'rounds=',len(rounds),'nodes=',out['nodes'],
          'median NPS=',round(out['reference_median_nps']),round(out['candidate_median_nps']),
          'paired %%= %.2f [%.2f, %.2f]'%(stats.median(ratios),min(ratios),max(ratios)),
          'CPU %%= %.2f'%stats.median(cpu_ratios))
