"""Offline ARM-speed kit. All artifacts stay in this worktree, under validation/as-*.
Health pauses suspend only children started here. No network, GPU, deployment or remote writes.
"""
import argparse
import ctypes
import hashlib
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import sys
import threading
import time
import interleave
import run

ROOT = run.ROOT
OUT = ROOT / 'validation'
CONTROL = ROOT.parent / 'testing/health/control.txt'
CHILDREN = []
LOCK = threading.Lock()

def paused():
    return CONTROL.exists() and 'pause' in CONTROL.read_text().lower()

def health():
    while paused(): time.sleep(.2)

def register(p):
    with LOCK: CHILDREN.append(p)
    return p

def watchdog():
    suspended = set()
    while True:
        with LOCK: children = list(CHILDREN)
        for p in children:
            if p.poll() is not None:
                suspended.discard(p.pid)
                continue
            pause = paused()
            if pause != (p.pid in suspended):
                if os.name == 'nt':
                    # Include compiler/linker descendants of our cargo process, never unrelated jobs.
                    handles = [ctypes.c_void_p(int(p._handle))]
                    descendants = []
                    if getattr(p, '_build', False):
                        class Entry(ctypes.Structure):
                            _fields_ = [('size',ctypes.c_ulong),('usage',ctypes.c_ulong),('pid',ctypes.c_ulong),('heap',ctypes.c_size_t),('module',ctypes.c_ulong),('threads',ctypes.c_ulong),('parent',ctypes.c_ulong),('priority',ctypes.c_long),('flags',ctypes.c_ulong),('exe',ctypes.c_wchar*260)]
                        k = ctypes.windll.kernel32
                        k.CreateToolhelp32Snapshot.restype = ctypes.c_void_p
                        snap = k.CreateToolhelp32Snapshot(2, 0)
                        e = Entry(); e.size = ctypes.sizeof(e); edges = []
                        ok = k.Process32FirstW(ctypes.c_void_p(snap), ctypes.byref(e))
                        while ok:
                            edges.append((e.pid,e.parent)); ok=k.Process32NextW(ctypes.c_void_p(snap),ctypes.byref(e))
                        k.CloseHandle(ctypes.c_void_p(snap))
                        own = {p.pid}
                        while True:
                            nxt = own | {pid for pid, parent in edges if parent in own}
                            if nxt == own: break
                            own = nxt
                        k.OpenProcess.restype = ctypes.c_void_p
                        for pid in own - {p.pid}:
                            h = k.OpenProcess(0x0800, False, pid)
                            if h: descendants.append(ctypes.c_void_p(h))
                        handles += descendants
                    fn = ctypes.windll.ntdll.NtSuspendProcess if pause else ctypes.windll.ntdll.NtResumeProcess
                    for h in handles: fn(h)
                    for h in descendants: ctypes.windll.kernel32.CloseHandle(h)
                else:
                    os.killpg(p.pid, signal.SIGSTOP if pause else signal.SIGCONT)
                if pause: suspended.add(p.pid)
                else: suspended.discard(p.pid)
        time.sleep(.1)

def monitored(cmd, log, env=None, cwd=ROOT, build=False):
    health()
    kwargs = dict(creationflags=0x4000 | 0x08000000) if os.name == 'nt' else dict(start_new_session=True)
    with (OUT/log).open('w') as f:
        p = subprocess.Popen(cmd, cwd=cwd, env=env, stdout=f, stderr=subprocess.STDOUT, **kwargs)
        p._build = build
        register(p).wait()
    if p.returncode: raise RuntimeError(f'command failed: validation/{log}')

def binary(label):
    return OUT / ('as-'+label+('.exe' if os.name=='nt' else '.engine'))

def build(a):
    source = ROOT
    if a.ref:
        source = OUT / ('as-source-'+a.label)
        source.mkdir(exist_ok=False)
        archive = OUT / ('as-source-'+a.label+'.tar')
        subprocess.run(['git','archive','--format=tar','-o',str(archive),a.ref],cwd=ROOT,check=True)
        subprocess.run(['tar','-xf',str(archive),'-C',str(source)],check=True)
    flags = {'native':'-C target-cpu=native', 'avx2':'-C target-cpu=native -C target-feature=-avxvnni',
             'scalar':'-C target-cpu=x86-64', 'arm':'-C target-cpu=neoverse-n1',
             'arm-neon':'-C target-cpu=neoverse-n1 -C target-feature=-dotprod'}[a.arch]
    env = dict(os.environ, RUSTFLAGS=flags, RUST_TEST_THREADS='1', RUST_MIN_STACK='67108864')
    target = OUT / ('as-target-'+a.label)
    common = ['--release','--offline','-j','4','--manifest-path',str(source/'Cargo.toml'),'--target-dir',str(target)]
    if a.features: common += ['--features',a.features]
    if a.asm:
        monitored(['cargo','rustc','--lib']+common+['--target','aarch64-unknown-linux-gnu','--','--emit=asm'], 'as-'+a.label+'-asm.log', env, source, True)
    else:
        for action in ['build'] + (['test'] if a.test else []):
            monitored(['cargo',action]+common, 'as-'+a.label+'-'+action+'.log', env, source, True)
        shutil.copy2(target/'release'/('pawnstorm.exe' if os.name=='nt' else 'pawnstorm'), binary(a.label))
    meta = dict(ref=a.ref or subprocess.check_output(['git','rev-parse','HEAD'],cwd=ROOT,text=True).strip(),
                flags=flags,features=a.features,net_sha256=hashlib.sha256((run.LAB/'engines/x25-big7.nnue').read_bytes()).hexdigest(),
                frozen_options_sha256=hashlib.sha256((source/'frozen.options').read_bytes()).hexdigest())
    if not a.asm: meta['exe_sha256'] = hashlib.sha256(binary(a.label).read_bytes()).hexdigest()
    (OUT/('as-'+a.label+'-build.json')).write_text(json.dumps(meta,indent=2))
    print('built',a.label,flush=True)

def compare(a):
    for label in [a.reference,a.label]:
        monitored([str(binary(label)),'bench'], 'as-'+label+'-bench.log')
    import re
    totals = [int(re.search(r'(\d+) nodes', (OUT/('as-'+l+'-bench.log')).read_text())[1]) for l in [a.reference,a.label]]
    assert totals[0] == totals[1], totals
    print('bench parity',totals[0],flush=True)
    original = run.launch
    def launch(exe):
        if os.name == 'nt': p = original(exe)
        else:
            health()
            def setup(): os.sched_setaffinity(0, {int(os.environ.get('BENCH_CPU','0'))})
            p = subprocess.Popen([str(Path(exe).resolve())], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                 stderr=subprocess.DEVNULL, text=True, start_new_session=True, preexec_fn=setup)
        return register(p)
    run.launch = launch
    sys.argv = ['interleave.py',str(OUT/('as-'+a.label+'-'+a.action+'.json')),str(binary(a.reference)),str(binary(a.label)),
                '--depth',str(a.depth),'--count','12','--pairs',str(a.pairs)]
    interleave.main()

def main():
    if os.name == 'nt': assert ctypes.windll.kernel32.SetPriorityClass(ctypes.c_void_p(-1),0x4000)
    else: os.nice(10)
    threading.Thread(target=watchdog,daemon=True).start()
    p=argparse.ArgumentParser(); sub=p.add_subparsers(dest='action',required=True)
    b=sub.add_parser('build'); b.add_argument('label'); b.add_argument('--ref'); b.add_argument('--features',default=''); b.add_argument('--arch',choices=['native','avx2','scalar','arm','arm-neon'],default='native'); b.add_argument('--test',action='store_true'); b.add_argument('--asm',action='store_true')
    for cmd in ['check','speed']:
        q=sub.add_parser(cmd); q.add_argument('label'); q.add_argument('reference'); q.add_argument('--depth',type=int,default=15 if cmd=='check' else 20); q.add_argument('--pairs',type=int,default=1 if cmd=='check' else 6)
    q=sub.add_parser('profile'); q.add_argument('label'); q.add_argument('--iterations',type=int,default=20000); q.add_argument('--depth',type=int,default=15)
    a=p.parse_args()
    if a.action=='build': build(a)
    elif a.action in ['check','speed']: compare(a)
    else:
        monitored([str(binary(a.label)),'profile',str(a.iterations),str(a.depth),str(run.LAB/'engines/x25-big7.nnue'),str(run.LAB/'engines/x25-big7.options')], 'as-'+a.label+'-profile.log')
        print('profile saved',a.label,flush=True)

if __name__=='__main__': main()
