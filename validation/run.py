import subprocess, ctypes, re, time, pathlib, os
if os.name == 'nt':
    assert ctypes.windll.kernel32.SetPriorityClass(ctypes.c_void_p(-1), 0x4000)
ROOT = pathlib.Path(__file__).resolve().parents[1]
LAB = ROOT.parent / 'testing'
FENS = re.findall(r'(?m)^    "([^"]+)",', (ROOT/'src/bench.rs').read_text().split('/// Known perft')[0])
FENS.insert(0, 'rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1')
OPTIONS = {}
for text in (ROOT/'frozen.options').read_text().splitlines():
    text = text.strip()
    if text and not text.startswith('#'):
        name, value = text.split('=', 1)
        OPTIONS[name.strip()] = value.strip()
def health():
    while (LAB/'health/control.txt').exists() and (LAB/'health/control.txt').read_text().strip().lower().startswith('pause'):
        print('Health pause',flush=True); time.sleep(10)
def launch(exe):
    health()
    if os.name != 'nt':
        def setup():
            os.nice(10)
            os.sched_setaffinity(0, {int(os.environ.get('BENCH_CPU', '0'))})
        return subprocess.Popen([str(pathlib.Path(exe).resolve())], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True, preexec_fn=setup)
    p = subprocess.Popen([str(pathlib.Path(exe).resolve())], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True, creationflags=0x4000)
    k = ctypes.windll.kernel32
    assert k.GetPriorityClass(ctypes.c_void_p(int(p._handle))) == 0x4000
    assert k.SetProcessAffinityMask(ctypes.c_void_p(int(p._handle)), 1)
    class Throttle(ctypes.Structure):
        _fields_=[('Version',ctypes.c_ulong),('ControlMask',ctypes.c_ulong),('StateMask',ctypes.c_ulong)]
    t=Throttle(1,1,0); ok=k.SetProcessInformation(ctypes.c_void_p(int(p._handle)),4,ctypes.byref(t),ctypes.sizeof(t)); assert ok
    return p
def cpu(p):
    if os.name != 'nt':
        fields = pathlib.Path(f'/proc/{p.pid}/stat').read_text().rsplit(')', 1)[1].split()
        return (int(fields[11]) + int(fields[12])) / os.sysconf('SC_CLK_TCK')
    times=[ctypes.c_ulonglong() for _ in range(4)]
    ok=ctypes.windll.kernel32.GetProcessTimes(ctypes.c_void_p(int(p._handle)),*[ctypes.byref(t) for t in times])
    assert ok
    return (times[2].value+times[3].value)*1e-7
