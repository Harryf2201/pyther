"""Offline local matrix. Run from the frozen worktree; never writes other worktrees."""
import ctypes
import json
import os
import pathlib
import shutil
import subprocess
import time

ROOT = pathlib.Path(__file__).resolve().parents[1]
OUT = ROOT / 'validation'
CONTROL = ROOT.parent / 'testing/health/control.txt'

def health():
    while CONTROL.read_text().strip().lower().startswith('pause'):
        print('Health pause', flush=True)
        time.sleep(10)

def main():
    assert ctypes.windll.kernel32.SetPriorityClass(ctypes.c_void_p(-1), 0x4000)
    base = subprocess.check_output(['git', 'rev-parse', 'e6bfdf0'], cwd=ROOT, text=True).strip()
    if not (OUT / 'reference').exists():
        (OUT / 'reference').mkdir()
        subprocess.run(['git', 'archive', '--format=tar', '-o', str(OUT/'reference.tar'), base], cwd=ROOT, check=True)
        subprocess.run(['tar', '-xf', str(OUT/'reference.tar'), '-C', str(OUT/'reference')], check=True)
    (OUT/'base.json').write_text(json.dumps({'reference_commit': base}, indent=2))
    for arch, flags in [('native', '-C target-cpu=native'), ('avx2', '-C target-cpu=native -C target-feature=-avxvnni')]:
        for kind in ['reference', 'default', 'frozen']:
            source = OUT/'reference' if kind == 'reference' else ROOT
            target = OUT / ('target-' + arch + '-' + kind)
            env = os.environ.copy()
            env['RUSTFLAGS'] = flags
            # Existing NNUE tests replace a process-global net while search tests
            # read it; serialize the suite to avoid that pre-existing race.
            env['RUST_TEST_THREADS'] = '1'
            common = ['--release', '--offline', '-j', '4', '--manifest-path', str(source/'Cargo.toml'), '--target-dir', str(target)]
            if kind == 'frozen': common += ['--features', 'frozen']
            for action in ['build', 'test']:
                health()
                with (OUT/f'{kind}-{arch}-{action}.log').open('w') as log:
                    subprocess.run(['cargo', action] + common, cwd=ROOT, env=env, stdout=log, stderr=subprocess.STDOUT, check=True)
                print(action, kind, arch, 'passed', flush=True)
            shutil.copy2(target/'release/pawnstorm.exe', OUT/f'{kind}-{arch}.exe')

if __name__ == '__main__': main()
