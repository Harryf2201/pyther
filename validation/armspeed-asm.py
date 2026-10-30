"""Summarize emitted AArch64 assembly; counts are static instructions, not execution counts."""
import collections
import json
from pathlib import Path
import re
import sys

for label in sys.argv[1:]:
    root = Path(__file__).resolve().parent
    artifact = max((root / ('as-target-'+label) / 'aarch64-unknown-linux-gnu/release/deps').glob('pawnstorm-*.s'),key=lambda p:p.stat().st_size)
    asm = artifact.read_text()
    result = {}
    for name in ['refresh','update','evaluate','dense_l1_dotprod']:
        match = re.search(r'^(_RN[^:\n]*4nnue\d+'+name+r'):\n(.*?)^\s*\.size\s+\1', asm, re.M|re.S)
        if not match:
            (root/('as-'+label+'-'+name+'.asm')).write_text('; Function inlined or absent in '+artifact.name+'\n')
            continue
        body = match[2]
        mnemonics = collections.Counter(re.findall(r'^\t([a-z][a-z0-9]*)\s',body,re.M))
        result[name] = dict(instructions=sum(mnemonics.values()), mnemonics=dict(mnemonics),
            neon_i16_add_sub=len(re.findall(r'^\t(?:add|sub)\s+v\d+\.8h',body,re.M)),
            scalar_half_loads=mnemonics.get('ldrh',0),
            stack_vector_loads=len(re.findall(r'^\t(?:ldr|ldp)\s+[^\n]*[qd]\d+[^\n]*\[sp',body,re.M)),
            stack_vector_stores=len(re.findall(r'^\t(?:str|stp)\s+[^\n]*[qd]\d+[^\n]*\[sp',body,re.M)))
        (root/('as-'+label+'-'+name+'.asm')).write_text(body)
    (root/('as-'+label+'-asm-summary.json')).write_text(json.dumps(dict(artifact=artifact.name,functions=result),indent=2))
    for name, info in result.items():
        print(label,name,info['instructions'],'instructions;', info['mnemonics'].get('sdot',0),'sdot;',
              info['stack_vector_loads'],'stack-vector loads;',info['stack_vector_stores'],'stack-vector stores;',
              info['neon_i16_add_sub'],'NEON i16 add/sub;',info['scalar_half_loads'],'scalar half loads;',
              info['mnemonics'].get('fmla',0)+info['mnemonics'].get('fmadd',0),'FMA')
