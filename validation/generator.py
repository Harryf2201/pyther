"""Check build failure on invalid input, defaults, and case-insensitive overrides."""
import ast
import ctypes
import json
import os
import pathlib
import subprocess
import build

ROOT, OUT = build.ROOT, build.OUT

def main():
    assert ctypes.windll.kernel32.SetPriorityClass(ctypes.c_void_p(-1), 0x4000)
    options = ROOT/'frozen.options'
    original = options.read_bytes()
    env = os.environ.copy()
    env['RUSTFLAGS'] = '-C target-cpu=native'
    common = ['cargo', 'build', '--release', '--offline', '-j', '4', '--target-dir', str(OUT/'target-native-frozen'), '--features', 'frozen']
    results = {}
    try:
        for label, text in [('unknown', 'NotAParameter=1\n'), ('range-high', 'SprMatScale=1025\n'),
                            ('range-low', 'SprMatScale=-1\n'), ('integer', 'SprMatScale=no\n'),
                            ('syntax', 'SprMatScale\n')]:
            build.health()
            options.write_text(text)
            with (OUT/f'generator-{label}.log').open('w') as log:
                code = subprocess.run(common, cwd=ROOT, env=env, stdout=log, stderr=subprocess.STDOUT).returncode
            assert code != 0, label
            results[label] = 'cargo build rejected'
    finally:
        options.write_bytes(original)
        build.health()
        with (OUT/'generator-restored.log').open('w') as log:
            subprocess.run(common, cwd=ROOT, env=env, stdout=log, stderr=subprocess.STDOUT, check=True)

    # Exercise the actual compiled generator in an isolated fixture (no source edits).
    exe = next((OUT/'target-native-frozen/release/build').glob('pawnstorm-*/build-script-build.exe'))
    fixture = OUT/'generator-fixture'
    fixture.mkdir(exist_ok=True)
    env['OUT_DIR'] = str(fixture)
    env['CARGO_FEATURE_FROZEN'] = '1'
    for label, text in [('defaults', ''), ('override', '# comment\n sprmatscale = 400\nTmSoftPct=80\n')]:
        (fixture/'frozen.options').write_text(text)
        subprocess.run([str(exe)], cwd=fixture, env=env, stdout=subprocess.DEVNULL, check=True)
        values = ast.literal_eval((fixture/'frozen_values.rs').read_text().split(' = ')[1].rstrip(';\n'))
        if label == 'defaults': defaults = values
        else:
            # Exactly two entries differ; absent parameters retain their defaults.
            assert [(a, b) for a, b in zip(defaults, values) if a != b] == [(60, 80), (0, 400)]
    results['defaults-and-case-insensitive-overrides'] = 'passed'
    (OUT/'generator-results.json').write_text(json.dumps(results, indent=2))
    print('Generator validation passed', flush=True)

if __name__ == '__main__': main()
