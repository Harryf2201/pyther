"""CPU-only, pinned paired parity/speed checks. Run: py -3 testing/dualact_verify.py"""
from pathlib import Path
import ctypes, json, re, statistics, subprocess, time, random, struct
ROOT=Path(__file__).resolve().parents[2]
HERE=Path(__file__).resolve().parents[1]
OUT=HERE/'target/dualact-check'
FENS=re.findall(r'^    "([^"]+)"', (HERE/'src/bench.rs').read_text().split('/// Known perft')[0], re.M)
FENS.insert(0,'rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1')
OPTIONS=['setoption name '+k+' value '+v for k,v in (line.split('=',1) for line in (ROOT/'testing/engines/x25-big7.options').read_text().splitlines() if '=' in line)]
def health():
    while (ROOT/'testing/health/control.txt').read_text().strip().lower()=='pause':
        print('Health pause; waiting',flush=True);time.sleep(10)
def launch(exe,net):
    health()
    p=subprocess.Popen([str(exe)],stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.STDOUT,text=True,creationflags=0x4000)
    h=ctypes.windll.kernel32.OpenProcess(0x0200|0x0400,False,p.pid)
    if not h: raise ctypes.WinError()
    if not ctypes.windll.kernel32.SetProcessAffinityMask(h,4): raise ctypes.WinError()
    class Throttle(ctypes.Structure): _fields_=[('Version',ctypes.c_ulong),('ControlMask',ctypes.c_ulong),('StateMask',ctypes.c_ulong)]
    t=Throttle(1,1,0)
    if not ctypes.windll.kernel32.SetProcessInformation(h,4,ctypes.byref(t),ctypes.sizeof(t)): raise ctypes.WinError()
    ctypes.windll.kernel32.CloseHandle(h)
    send(p,'setoption name EvalFile value '+str(net))
    send(p,'isready')
    ready=until(p,'^readyok')
    if 'info string loaded network ' not in ready: raise RuntimeError(ready)
    for line in OPTIONS: send(p,line)
    return p

def send(p,s): p.stdin.write(s+'\n');p.stdin.flush()
def until(p,pattern):
    lines=[]
    while True:
        line=p.stdout.readline()
        if not line: raise RuntimeError('engine exited: '+''.join(lines))
        lines.append(line)
        if re.search(pattern,line): return ''.join(lines)
def close(p):send(p,'quit');p.wait()
def suite(exe,net,label):
    p=launch(exe,net);send(p,'bench 10');log=until(p,r'^\d+ nodes \d+ nps');nodes=[int(re.search(r'^(\d+) nodes',log,re.M)[1])]
    for fen in FENS[:6]:
        health();send(p,'ucinewgame');send(p,'position fen '+fen);send(p,'go depth 10')
        part=until(p,'^bestmove ');log+=part
        nodes.append(int(re.findall(r'info depth 10 .*? nodes (\d+)',part)[-1]))
    close(p);(OUT/(label+'.log')).write_text(log);return nodes

def timing(exe,net):
    p=launch(exe,net);send(p,'bench 11');s=until(p,r'^\d+ nodes \d+ nps');close(p)
    return list(map(int,re.search(r'^(\d+) nodes (\d+) nps',s,re.M).groups()))
def synthetic():
    OUT.mkdir(parents=True,exist_ok=True)
    r=random.Random(20261030)
    pack=lambda vals: struct.pack('<%sf'%len(vals),*vals)
    ft=struct.pack('<%sh'%(769*1024),*[r.randrange(-8,9) for _ in range(768*1024)],*[r.randrange(50,151) for _ in range(1024)])
    f=lambda n,scale:pack([r.uniform(-scale,scale) for _ in range(n)])
    l1=f(1024*8*16,.04)+f(8*16,.3)
    l2=f(16*8*32,.15)
    rest=f(8*32,.2)+f(32*8,.3)+f(8,.1)
    (OUT/'synthetic-ml.nnue').write_bytes(ft+l1+l2+rest)
    # Zero added weights preserve the search tree while exercising the full runtime layout.
    (OUT/'synthetic-dual.nnue').write_bytes(ft+l1+l2+bytes(16*8*32*4)+rest)

if __name__=='__main__':
    synthetic()
    base=OUT/'default.exe';dual=OUT/'dualact.exe';net=ROOT/'testing/engines/x25-big7.nnue'
    a=suite(ROOT/'testing/engines/pawnstorm-xsearch5.exe',net,'reference')
    b=suite(base,net,'default');assert a==b,(a,b)
    print('PARITY',a,flush=True)
    results=[]
    for i in range(8):
        pair={}
        for label in (['default','dual'] if i%2==0 else ['dual','default']):
            pair[label]=timing(base if label=='default' else dual,OUT/('synthetic-ml.nnue' if label=='default' else 'synthetic-dual.nnue'))
        assert pair['default'][0]==pair['dual'][0],pair
        results.append(pair);print(i,pair,flush=True)
    data={'parity':a,'timings':results,'median_ratio':statistics.median(p['dual'][1]/p['default'][1] for p in results[1:])}
    (OUT/'results.json').write_text(json.dumps(data,indent=2));print(json.dumps(data),flush=True)
