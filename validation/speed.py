"""Six pairs per ISA, alternating engine order for each fresh-state position."""
import pathlib
import sys
import interleave

OUT = pathlib.Path(__file__).resolve().parent
for arch in ['native', 'avx2']:
    sys.argv = ['interleave.py', str(OUT/f'speed-{arch}.json'),
                str(OUT/f'reference-{arch}.exe'), str(OUT/f'frozen-{arch}.exe'),
                '--depth', '20', '--count', '12', '--pairs', '6']
    interleave.main()
