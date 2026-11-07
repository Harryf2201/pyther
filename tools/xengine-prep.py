"""Acquire the approved pool on Actions only. Lock mode records, match mode verifies."""
import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
import urllib.request

from importlib.machinery import SourceFileLoader
analysis = SourceFileLoader('analysis', str(Path(__file__).with_name('xengine-analysis.py'))).load_module()


def run(args, cwd=None):
    subprocess.run(args, cwd=cwd, check=True)


def check(actual, expected, label, lock):
    if expected is None and lock:
        return
    if actual != expected:
        raise ValueError('SHA-256/identity mismatch: ' + label)


def acquire(item, dest, lock):
    urllib.request.urlretrieve(item['url'], dest)
    actual = analysis.digest(dest)
    check(actual, item['sha256'], item['url'], lock)
    item['sha256'] = actual


def unpack(archive, dest):
    with tarfile.open(archive) as tar:
        tar.extractall(dest, filter='data')
    return next(p for p in dest.iterdir() if p.is_dir())


def probe(exe):
    result = subprocess.run([str(exe.resolve())], input='uci\nquit\n', text=True,
                            capture_output=True, timeout=30, check=True)
    lines = result.stdout.splitlines()
    if 'uciok' not in lines:
        raise ValueError('UCI handshake failed')
    # Only engine identity and option declarations are public provenance.
    return [s for s in lines if s.startswith(('id name ', 'option name '))]


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--lock', action='store_true')
    args = parser.parse_args()
    if os.environ.get('GITHUB_ACTIONS') != 'true':
        raise RuntimeError('Pool acquisition/builds are restricted to GitHub Actions')
    manifest = json.loads(Path('.github/xengine/pool-v1.json').read_text())
    out = Path('pool')
    out.mkdir(exist_ok=True)
    fingerprint = '\n'.join(subprocess.check_output(cmd, text=True) for cmd in
                            (['g++-14', '--version'], ['ld', '--version'], ['make', '--version']))
    fingerprint += analysis.digest(shutil.which('g++-14'))
    fingerprint += analysis.digest(shutil.which('ld'))
    import hashlib
    compiler_hash = hashlib.sha256(fingerprint.encode()).hexdigest()
    check(compiler_hash, manifest['toolchain']['sha256'], 'toolchain', args.lock)
    manifest['toolchain'].update(sha256=compiler_hash, identity=fingerprint)
    analysis.write_json(out / 'observed-lock.json', manifest)
    if (out / 'verified.json').exists() and not args.lock:
        for engine in manifest['engines']:
            check(analysis.digest(out / (engine['id'] + '-source.tar.gz')),
                  engine['source']['sha256'], engine['id'] + ' cached source', False)
            if engine.get('net'):
                check(analysis.digest(out / (engine['id'] + '-network')),
                      engine['net']['sha256'], engine['id'] + ' cached network', False)
            check(analysis.digest(out / engine['id']), engine['binary_sha256'], engine['id'], False)
            check(probe(out / engine['id']), engine['uci'], engine['id'] + ' UCI', False)
        check(analysis.digest(out / 'fc'), manifest['fastchess']['binary_sha256'], 'fastchess', False)
        return
    work = Path('pool-work')
    work.mkdir(exist_ok=True)
    for engine in manifest['engines']:
        name = engine['id']
        dest = work / name
        dest.mkdir(exist_ok=True)
        archive = dest / 'source.tar.gz'
        acquire(engine['source'], archive, args.lock)
        source = unpack(archive, dest)
        # Distribute the exact source and licence with the binary artifact.
        shutil.copy2(archive, out / (name + '-source.tar.gz'))
        shutil.copy2(source / engine['licence_path'], out / (name + '-LICENSE'))
        target = out / name
        if engine.get('asset'):
            acquire(engine['asset'], target, args.lock)
        if engine.get('checksums'):
            checksum_file = dest / 'checksums.txt'
            acquire(engine['checksums'], checksum_file, args.lock)
            lines = checksum_file.read_text().splitlines()
            asset_name = engine['asset']['url'].rsplit('/', 1)[1]
            if not any(analysis.digest(target) in line and asset_name in line for line in lines):
                raise ValueError('upstream Carp checksum mismatch')
        if engine.get('net'):
            acquire(engine['net'], source / engine['net_path'], args.lock)
            shutil.copy2(source / engine['net_path'], out / (name + '-network'))
        if engine.get('commands'):
            for command in engine['commands']:
                run(command, cwd=source)
            shutil.copy2(source / engine['output'], target)
        target.chmod(0o755)
        actual = analysis.digest(target)
        check(actual, engine['binary_sha256'], name + ' binary', args.lock)
        engine['binary_sha256'] = actual
        identity = probe(target)
        check(identity, engine['uci'], name + ' UCI', args.lock)
        engine['uci'] = identity
        analysis.write_json(out / 'observed-lock.json', manifest)
        print(name + ' SHA-256 ' + actual, flush=True)
    fc = manifest['fastchess']
    archive = work / 'fastchess.tar'
    acquire(fc, archive, args.lock)
    fcdir = work / 'fastchess'
    fcdir.mkdir(exist_ok=True)
    with tarfile.open(archive) as tar:
        tar.extractall(fcdir, filter='data')
    executable = next(p for p in fcdir.rglob('fastchess') if p.is_file())
    shutil.copy2(executable, out / 'fc')
    (out / 'fc').chmod(0o755)
    check(analysis.digest(out / 'fc'), fc['binary_sha256'], 'fastchess binary', args.lock)
    fc['binary_sha256'] = analysis.digest(out / 'fc')
    analysis.write_json(out / 'observed-lock.json', manifest)
    analysis.write_json(out / 'verified.json', {'manifest': analysis.digest(out / 'observed-lock.json')})
    print('Pool lock complete. Pin pool/observed-lock.json before match dispatch.')


if __name__ == '__main__':
    main()
