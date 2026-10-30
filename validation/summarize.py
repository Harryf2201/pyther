import json
import pathlib
import statistics

OUT = pathlib.Path(__file__).resolve().parent

def rate(rows, clock):
    return sum(r['nodes'] for r in rows) / sum(r[clock] for r in rows)

def spread(xs):
    return dict(median=statistics.median(xs), minimum=min(xs), maximum=max(xs))

def main():
    summary = {}
    for arch in ['native', 'avx2']:
        data = json.loads((OUT/f'speed-{arch}.json').read_text())
        exes = list(data[0])
        record = {'rounds': len(data), 'nodes_per_engine_round': sum(r['nodes'] for r in data[0][exes[0]])}
        for clock in ['seconds', 'cpu_seconds']:
            baseline = [rate(pair[exes[0]], clock) for pair in data]
            candidate = [rate(pair[exes[1]], clock) for pair in data]
            record[clock] = dict(reference_nps=spread(baseline), frozen_nps=spread(candidate),
                                 paired_percent=spread([100*(b/a-1) for a,b in zip(baseline,candidate)]))
        summary[arch] = record
    for path in sorted(OUT.glob('parity-*.json')):
        if '.meta.' in path.name: continue
        pair = json.loads(path.read_text())[0]
        summary[path.stem] = {pathlib.Path(e).name: dict(positions=len(rows), nodes=sum(r['nodes'] for r in rows)) for e,rows in pair.items()}
    (OUT/'summary.json').write_text(json.dumps(summary, indent=2))
    print(json.dumps(summary, indent=2))

if __name__ == '__main__': main()
