"""Per-position parity for both feature sets and ISAs, plus frozen UCI behavior."""
import json
import pathlib
import interleave

OUT = pathlib.Path(__file__).resolve().parent

def uci_contract(exe):
    e = interleave.Engine(exe)
    try:
        e.send('uci')
        listing = e.until('uciok')
        assert any(s.startswith('option name SprMatScale type spin ') for s in listing)
        before = e.search(interleave.run.FENS[0], 15)
        e.send('setoption name sprmatscale value 400')
        e.send('isready')
        assert e.until('readyok') == ['readyok'], 'same value should be silent'
        for value in [0, 9999, -1]:
            e.send(f'setoption name SPRMATSCALE value {value}')
            e.send('isready')
            assert e.until('readyok') == ['info string SprMatScale is frozen at 400 in this build', 'readyok']
        after = e.search(interleave.run.FENS[0], 15)
        assert all(before[k] == after[k] for k in ['nodes', 'score', 'best', 'pv'])
    finally:
        e.close()

def main():
    for arch in ['native', 'avx2']:
        for kind in ['default', 'frozen']:
            for depth, count in [(15, 12), (17, 8)]:
                import sys
                sys.argv = ['interleave.py', str(OUT/f'parity-{kind}-{arch}-d{depth}.json'),
                            str(OUT/f'reference-{arch}.exe'), str(OUT/f'{kind}-{arch}.exe'),
                            '--depth', str(depth), '--count', str(count), '--pairs', '1']
                interleave.main()
        uci_contract(OUT/f'frozen-{arch}.exe')
        print('UCI contract passed', arch, flush=True)
    (OUT/'uci-contract.json').write_text(json.dumps({'native': 'passed', 'avx2': 'passed'}, indent=2))

if __name__ == '__main__': main()
