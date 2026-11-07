"""Paired, stratified external-pool analysis. Standard library only."""
import argparse
from collections import Counter, defaultdict
import hashlib
import json
import math
from pathlib import Path
import random
import re
import statistics


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def write_json(path, value):
    Path(path).write_text(json.dumps(value, indent=2, allow_nan=False) + '\n', encoding='utf-8')


def elo(p):
    return 400 * math.log10(p / (1 - p)) if 0 < p < 1 else None


def effect(cells):
    values = []
    for pairs in cells.values():
        c, r = (statistics.mean(p[i] for p in pairs) for i in (0, 1))
        ec, er = elo(c), elo(r)
        if ec is None or er is None:
            return None
        values.append(ec - er)
    return statistics.mean(values)


def interval(values):
    # Undefined bootstrap cells must remain undefined, never be clipped or discarded.
    if not values or any(v is None for v in values):
        return None
    values = sorted(values)
    return [values[int((len(values) - 1) * q)] for q in (0.025, 0.975)]


def analyse(schedule, records, seed=2026110701, replicates=10000):
    blocks = schedule['blocks']
    expected = {}
    starts = set()
    for b in blocks:
        normalized = ' '.join(b['fen'].split()[:4])
        if normalized in starts:
            raise ValueError('reused normalized opening')
        starts.add(normalized)
        for role in ('candidate', 'release'):
            for colour in ('white', 'black'):
                key = (b['id'], role, colour)
                if key in expected:
                    raise ValueError('duplicate schedule block')
                expected[key] = b
    seen, scores = set(), defaultdict(dict)
    terminations = Counter()
    wdl = defaultdict(Counter)
    defects = []
    duration = 0.0
    for g in records:
        key = (g['block'], g['role'], g['colour'])
        if key not in expected or key in seen:
            raise ValueError('unexpected or duplicate game')
        b = expected[key]
        for field in ('opponent', 'fen', 'shard'):
            if g[field] != b[field]:
                raise ValueError('game disagrees with schedule: ' + field)
        if g['result'] not in ('1-0', '0-1', '1/2-1/2'):
            raise ValueError('unfinished game')
        if g['options_hash'] != schedule['options_hash']:
            raise ValueError('wrong options')
        score = {'1-0': 1.0, '0-1': 0.0, '1/2-1/2': 0.5}[g['result']]
        if g['colour'] == 'black':
            score = 1 - score
        scores[b['id']][(g['role'], g['colour'])] = score
        seen.add(key)
        term = g['termination']
        terminations[term] += 1
        if any(word in term.lower() for word in ('crash', 'disconnect', 'illegal', 'stall')):
            defects.append(key)
        wdl[(g['opponent'], g['role'])][str(score)] += 1
        duration += g.get('duration_seconds', 0)
    if seen != set(expected):
        raise ValueError('incomplete blocks: expected %d games, got %d' % (len(expected), len(seen)))
    cells, runners = defaultdict(list), defaultdict(lambda: defaultdict(list))
    for b in blocks:
        row = scores[b['id']]
        pair = tuple((row[(role, 'white')] + row[(role, 'black')]) / 2
                     for role in ('candidate', 'release'))
        cells[b['opponent']].append(pair)
        runners[b['shard']][b['opponent']].append(pair)
    if len({len(v) for v in cells.values()}) != 1:
        raise ValueError('unequal opponent allocation')
    rng = random.Random(seed)
    boot = []
    opponent_boot = {k: [] for k in cells}
    for _ in range(replicates):
        sample = {k: rng.choices(v, k=len(v)) for k, v in cells.items()}
        boot.append(effect(sample))
        for k, pairs in sample.items():
            opponent_boot[k].append(effect({k: pairs}))
    ci = interval(boot)
    cluster_boot = []
    if len(runners) >= 2:
        for _ in range(replicates):
            sample = defaultdict(list)
            for runner in rng.choices(list(runners), k=len(runners)):
                for k, v in runners[runner].items():
                    sample[k].extend(v)
            cluster_boot.append(effect(sample))
    estimate = effect(cells)
    decision = 'inconclusive'
    if defects:
        decision = 'fail'
    elif ci:
        if ci[0] > 0:
            decision = 'improvement'
        elif ci[0] > -10:
            decision = 'non-regression'
        elif ci[1] < -10:
            decision = 'fail'
    rows = []
    for k, pairs in cells.items():
        c, r = (statistics.mean(p[i] for p in pairs) for i in (0, 1))
        covariance = statistics.mean((a - c) * (b - r) for a, b in pairs)
        d = [a - b for a, b in pairs]
        rows.append(dict(opponent=k, blocks=len(pairs), candidate_score=c, release_score=r,
                         candidate_wdl=dict(wdl[(k, 'candidate')]), release_wdl=dict(wdl[(k, 'release')]),
                         pool_elo=effect({k: pairs}), ci95=interval(opponent_boot[k]), covariance=covariance,
                         difference_variance=statistics.variance(d) if len(d) > 1 else None))
    loo = {k: effect({o: v for o, v in cells.items() if o != k}) for k in cells} if len(cells) > 1 else {}
    return dict(type='xengine', valid=True, games=len(records), blocks=len(blocks), pool_elo=estimate,
                ci95=ci, decision=decision, margin=-10, bootstrap_seed=seed, bootstrap_replicates=replicates,
                undefined_bootstrap=sum(v is None for v in boot),
                runner_ci95=interval(cluster_boot), runner_clusters=len(runners),
                raw_score_difference=statistics.mean(statistics.mean(a-b for a, b in v) for v in cells.values()),
                per_opponent=rows, leave_one_opponent_out=loo, termination_counts=dict(terminations),
                product_defects=len(defects), game_slot_seconds=duration)


def synthetic():
    schedule = dict(options_hash='synthetic', blocks=[])
    records = []
    for k in range(4):
        for i in range(40):
            b = dict(id=f'{k}-{i}', opponent=str(k), shard=i % 2,
                     fen=f'synthetic-{k}-{i} w - - 0 1')
            schedule['blocks'].append(b)
            for role in ('candidate', 'release'):
                for colour in ('white', 'black'):
                    records.append(dict(block=b['id'], role=role, colour=colour, result='1/2-1/2',
                                        options_hash='synthetic', termination='normal', **{f:b[f] for f in ('opponent','shard','fen')}))
    result = analyse(schedule, records, replicates=1000)
    assert result['pool_elo'] == 0 and result['ci95'] == [0, 0]
    assert result['decision'] == 'non-regression'
    for bad in (records[:-1], records + records[:1]):
        try:
            analyse(schedule, bad, replicates=10)
        except ValueError:
            pass
        else:
            raise AssertionError('corruption accepted')
    changed = [dict(g, result='1-0' if g['colour'] == 'white' else '0-1')
               if g['role'] == 'candidate' else g for g in records]
    assert analyse(schedule, changed, replicates=10)['decision'] == 'inconclusive'
    print('Synthetic A/A = 0 [0, 0]; missing/duplicate games rejected; boundary scores inconclusive')


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--self-test', action='store_true')
    parser.add_argument('--artifacts')
    parser.add_argument('--output')
    args = parser.parse_args()
    if args.self_test:
        synthetic()
        return
    root = Path(args.artifacts)
    schedules = list(root.rglob('schedule.json'))
    if not schedules or len({digest(p) for p in schedules}) != 1:
        raise ValueError('missing or inconsistent schedule')
    schedule = json.loads(schedules[0].read_text())
    records = []
    record_files = sorted(root.rglob('records.jsonl'))
    if len(record_files) != schedule['inputs']['jobs']:
        raise ValueError('wrong number of completed shards')
    configurations = []
    for p in record_files:
        shard_records = [json.loads(line) for line in p.read_text().splitlines() if line.strip()]
        configurations.append(digest(p.parent / 'engines.json'))
        if digest(p.parent / 'pool-lock.json') != schedule['pool_hash']:
            raise ValueError('archived pool lock mismatch')
        games = re.split(r'(?=^\[XBlock )', (p.parent / 'games.pgn').read_text(), flags=re.M)
        parsed = {}
        for game in games:
            if not game.strip():
                continue
            tags = dict(re.findall(r'^\[(\w+) "(.*)"\]$', game, re.M))
            role = tags['XRole']
            colour = 'white' if tags['White'] == role else 'black'
            key = (tags['XBlock'], role, colour)
            if key in parsed:
                raise ValueError('duplicate PGN game')
            parsed[key] = tags
        if len(parsed) != len(shard_records):
            raise ValueError('PGN/record count mismatch')
        for record in shard_records:
            tags = parsed[(record['block'], record['role'], record['colour'])]
            opponent = tags['Black'] if record['colour'] == 'white' else tags['White']
            if (tags['Result'] != record['result'] or tags['FEN'] != record['fen']
                    or opponent != record['opponent']):
                raise ValueError('PGN/record identity, opening or result mismatch')
        records.extend(shard_records)
    if len(set(configurations)) != 1:
        raise ValueError('runner engine binaries/options differ')
    out = Path(args.output)
    out.mkdir(parents=True, exist_ok=True)
    try:
        result = analyse(schedule, records)
        result.update(pool_hash=schedule['pool_hash'], schedule_hash=digest(schedules[0]),
                      inputs=schedule['inputs'], options_hash=schedule['options_hash'],
                      engines_hash=configurations[0])
        timings = [json.loads(p.read_text()) for p in root.rglob('timing.json')]
        result['runner_seconds'] = sum(t['seconds'] for t in timings)
        result['games_per_runner_second'] = result['games'] / result['runner_seconds']
        write_json(out / 'results.json', result)
    except ValueError as exc:
        write_json(out / 'results.json', dict(type='xengine', valid=False, decision='fail', error=str(exc)))
        raise
    with (out / 'games.pgn').open('wb') as dest:
        for p in sorted(root.rglob('games.pgn')):
            dest.write(p.read_bytes())
    print('Elo: %s; CI95: %s; %s' % (result['pool_elo'], result['ci95'], result['decision']))
    print('Games: %d, complete paired external-pool games' % result['games'])


if __name__ == '__main__':
    main()
