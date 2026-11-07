"""Frozen inputs, explicit schedules and one fresh fastchess process per game."""
import argparse
from concurrent.futures import ThreadPoolExecutor
from importlib.machinery import SourceFileLoader
import json
import os
from pathlib import Path
import random
import re
import shutil
import subprocess
import time

A = SourceFileLoader('analysis', str(Path(__file__).with_name('xengine-analysis.py'))).load_module()
P = SourceFileLoader('prep', str(Path(__file__).with_name('xengine-prep.py'))).load_module()


def read(path):
    return json.loads(Path(path).read_text())


def download(asset, directory):
    subprocess.run(['gh', 'release', 'download', 'testkit', '--pattern', asset, '--dir', str(directory)], check=True)


def prepare(asset):
    if not re.fullmatch(r'xengine-[a-f0-9]{32}\.json', asset):
        raise ValueError('invalid immutable request name')
    kit = Path('kit')
    kit.mkdir()
    download(asset, kit)
    request = read(kit / asset)
    if A.digest('.github/xengine/pool-v1.json') != request['pool_hash']:
        raise ValueError('requested pool lock differs from workflow checkout')
    for key, low, high in (('jobs', 1, 8), ('blocks', 1, 250), ('seed', 1, 2**53-1)):
        if type(request[key]) is not int or not low <= request[key] <= high:
            raise ValueError('invalid ' + key)
    if request['pool'] != 'xengine-pool-v1' or not re.fullmatch(r'\d+(?:\.\d+)?\+\d+(?:\.\d+)?', request['tc']):
        raise ValueError('invalid pool or time control')
    for role in ('candidate', 'release'):
        spec = request[role]
        if not re.fullmatch('[a-f0-9]{40}', spec['ref']):
            raise ValueError('full code commit required')
        for kind in ('nnue', 'options', 'ref_file'):
            item = spec[kind]
            if not re.fullmatch(r'xengine-[a-f0-9]{32}-(candidate|release)\.(nnue|options|ref)', item['asset']):
                raise ValueError('invalid spec asset')
            download(item['asset'], kit)
            if A.digest(kit / item['asset']) != item['sha256']:
                raise ValueError('frozen spec hash mismatch')
        if (kit / spec['ref_file']['asset']).read_text().strip() != spec['ref']:
            raise ValueError('frozen ref mismatch')
    book = request['book']
    if not re.fullmatch(r'[A-Za-z0-9._-]{1,60}\.(pgn|epd)', book['asset']):
        raise ValueError('invalid book')
    download(book['asset'], kit)
    if A.digest(kit / book['asset']) != book['sha256']:
        raise ValueError('book hash mismatch')
    subprocess.run(['rustc', '-O', 'tools/book-board.rs', '-o', 'book-board'], check=True)
    if book['asset'].endswith('.pgn'):
        raw = subprocess.check_output(['./book-board', 'pgn', str(kit / book['asset'])], text=True)
        fens = [line.split('\t')[2] for line in raw.splitlines()]
    else:
        raw = (kit / book['asset']).read_text().splitlines()
        input_text = ''.join(' '.join(line.split()[:4]) + ' 0 1\n' for line in raw if line.strip())
        normalized = subprocess.check_output(['./book-board'], input=input_text, text=True)
        fens = [line.split('\t')[0] for line in normalized.splitlines()]
    unique = {}
    for fen in fens:
        unique.setdefault(' '.join(fen.split()[:4]), fen)
    fens = list(unique.values())
    random.Random(request['seed']).shuffle(fens)
    pool = read('.github/xengine/pool-v1.json')
    count = request['jobs'] * request['blocks'] * len(pool['engines'])
    if count > len(fens):
        raise ValueError('not enough unique normalized openings')
    blocks = []
    for engine in pool['engines']:
        for shard in range(request['jobs']):
            for _ in range(request['blocks']):
                i = len(blocks)
                blocks.append(dict(id=f'b{i:06}', fen=fens[i], opponent=engine['id'], shard=shard))
    schedule = dict(inputs=request, pool_hash=A.digest('.github/xengine/pool-v1.json'),
                    options_hash=A.digest(kit / asset), blocks=blocks)
    A.write_json(kit / 'schedule.json', schedule)
    with open(os.environ['GITHUB_OUTPUT'], 'a') as out:
        out.write('shards=' + json.dumps(list(range(request['jobs']))) + '\n')


def options(exe, wanted):
    declarations = P.probe(exe)
    supported = {}
    for line in declarations:
        match = re.match(r'option name (.*?) type (\w+)(.*)', line)
        if match:
            supported[match[1].lower()] = (match[1], match[2], match[3])
    controls = {'hash': '16', 'threads': '1', 'ponder': 'false', 'ownbook': 'false',
                'usebook': 'false', 'book': 'false', 'learning': 'false', 'learn': 'false',
                'bookfile': '', 'bookpath': '', 'bookdepth': '0',
                'syzygypath': '', 'syzygyprobelimit': '0'}
    for key, value in controls.items():
        if key in supported:
            name, kind, _ = supported[key]
            if key in ('book', 'learning', 'learn') and kind != 'check':
                raise ValueError('unreviewed book/learning option ' + name)
            wanted[name] = value
    for required in ('hash', 'threads'):
        if required not in supported:
            raise ValueError('required UCI control missing')
    result = []
    for name, value in wanted.items():
        if name.lower() not in supported:
            raise ValueError('unsupported UCI option ' + name)
        result.append('option.' + supported[name.lower()][0] + '=' + value)
    return result


def play(shard):
    schedule = read('kit/schedule.json')
    pool = read('.github/xengine/pool-v1.json')
    if A.digest('.github/xengine/pool-v1.json') != schedule['pool_hash']:
        raise ValueError('pool manifest mismatch')
    flags = set(Path('/proc/cpuinfo').read_text().split())
    if not set(pool['required_cpu']) <= flags:
        raise ValueError('runner lacks locked CPU features')
    inputs = schedule['inputs']
    commands = {}
    for role in ('candidate', 'release'):
        spec = inputs[role]
        work = Path('build-' + role)
        subprocess.run(['git', 'worktree', 'add', '--detach', str(work), spec['ref']], check=True)
        subprocess.run(['cargo', '+1.90.0', 'build', '--locked', '--release', '-p', 'pawnstorm'], cwd=work, check=True)
        exe = Path('eng-' + role)
        shutil.copy2(work / 'target/release/pawnstorm', exe)
        wanted = {}
        for line in Path('kit', spec['options']['asset']).read_text().splitlines():
            if line.strip():
                name, value = line.split('=', 1)
                wanted[name.strip()] = value.strip()
        wanted['EvalFile'] = str(Path('kit', spec['nnue']['asset']).resolve())
        commands[role] = [str(exe.resolve()), options(exe, wanted)]
    for engine in pool['engines']:
        exe = Path('pool', engine['id'])
        exe.chmod(0o755)
        if A.digest(exe) != engine['binary_sha256']:
            raise ValueError('pool binary mismatch')
        commands[engine['id']] = [str(exe.resolve()), options(exe, {})]
    fc = Path('pool/fc')
    fc.chmod(0o755)
    if A.digest(fc) != pool['fastchess']['binary_sha256']:
        raise ValueError('fastchess mismatch')
    out = Path('result')
    out.mkdir(exist_ok=True)
    shutil.copy2('kit/schedule.json', out / 'schedule.json')
    shutil.copy2('.github/xengine/pool-v1.json', out / 'pool-lock.json')
    A.write_json(out / 'engines.json', {name: dict(binary_sha256=A.digest(cmd[0]), options=cmd[1])
                                      for name, cmd in commands.items()})

    def block(b):
        folder = out / b['id']
        folder.mkdir()
        opening = folder / 'opening.pgn'
        opening.write_text('[SetUp "1"]\n[FEN "' + b['fen'] + '"]\n[Result "*"]\n\n*\n')
        records, pgns = [], []
        roles = ('candidate', 'release') if int(b['id'][1:]) % 2 == 0 else ('release', 'candidate')
        for role in roles:
            for colour in ('white', 'black'):
                names = [role, b['opponent']] if colour == 'white' else [b['opponent'], role]
                pgn = folder / (role + '-' + colour + '.pgn')
                args = [str(fc.resolve())]
                for name in names:
                    cmd, opts = commands[name]
                    args += ['-engine', 'cmd=' + cmd, 'name=' + name, 'proto=uci'] + opts
                args += ['-each', 'tc=' + inputs['tc'], '-openings', 'file=' + str(opening.resolve()),
                         'format=pgn', 'order=sequential', '-rounds', '1', '-games', '1', '-concurrency', '1',
                         '-pgnout', 'file=' + str(pgn.resolve())]
                log = folder / (role + '-' + colour + '.log')
                started = time.monotonic()
                with log.open('w') as stream:
                    subprocess.run(args, stdout=stream, stderr=subprocess.STDOUT, check=True, timeout=600)
                text = pgn.read_text()
                tags = dict(re.findall(r'^\[(\w+) "(.*)"\]$', text, re.M))
                if tags.get('White') != names[0] or tags.get('Black') != names[1]:
                    raise ValueError('PGN identity mismatch')
                if tags.get('FEN') != b['fen']:
                    raise ValueError('PGN opening mismatch')
                # Preserve actual termination and final reason, including time losses/defects.
                comments = re.findall(r'\{([^{}]*)\}', text)
                reason = tags.get('Termination', '') + ' ' + (comments[-1] if comments else '')
                termination = 'normal'
                for category in ('illegal', 'crash', 'disconnect', 'stall', 'time', 'mate', 'repetition', 'fifty', 'material', 'stalemate'):
                    if category in reason.lower():
                        termination = category
                        break
                record = dict(block=b['id'], role=role, colour=colour, result=tags.get('Result'),
                              opponent=b['opponent'], fen=b['fen'], shard=shard,
                              options_hash=schedule['options_hash'], termination=termination,
                              reason=reason, duration_seconds=time.monotonic()-started)
                records.append(record)
                pgns.append('[XBlock "' + b['id'] + '"]\n[XRole "' + role + '"]\n' + text)
        (folder / 'records.jsonl').write_text(''.join(json.dumps(g) + '\n' for g in records))
        (folder / 'combined.pgn').write_text('\n'.join(pgns))
        print('BLOCK ' + b['id'] + ' complete', flush=True)
        return folder

    # Two slots on standard runners, further bounded by available CPU count.
    slots = max(1, min(2, os.cpu_count() or 1))
    start = time.monotonic()
    with ThreadPoolExecutor(max_workers=slots) as workers:
        folders = list(workers.map(block, [b for b in schedule['blocks'] if b['shard'] == shard]))
    with (out / 'records.jsonl').open('wb') as records, (out / 'games.pgn').open('wb') as pgn:
        for folder in folders:
            records.write((folder / 'records.jsonl').read_bytes())
            pgn.write((folder / 'combined.pgn').read_bytes())
    # Individual files remain available for diagnosis but are not counted twice by analysis.
    for folder in folders:
        (folder / 'records.jsonl').rename(folder / 'records.block.jsonl')
    A.write_json(out / 'timing.json', dict(seconds=time.monotonic()-start, slots=slots, shard=shard))


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--prepare')
    parser.add_argument('--shard', type=int)
    args = parser.parse_args()
    if args.prepare:
        prepare(args.prepare)
    else:
        play(args.shard)
