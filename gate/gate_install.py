import native_gate_control as c
import os,sys,pathlib,json,hashlib,re,subprocess,time
ROOT=pathlib.Path(c.ROOT)
HOOK=pathlib.Path('/var/lib/webosbrew/init.d/55-lg-native')
TRIAL=HOOK.with_name('54-lg-native-gate-trial')

class LinuxIO:
    def __init__(self):
        c.require(sys.platform.startswith('linux') and os.geteuid()==0,'Requires root Linux TV')
        c.secure_parents(str(ROOT/'sentinel'));c.require(ROOT.stat().st_mode&0o777==0o700,'Private root mode')
    def check_files(self,package):
        c.validate_state(package);c.require(len(package.get('files',{}))==3,'Expected three immutable gate files')
        for path,digest in package['files'].items():
            c.require(re.fullmatch(re.escape(str(ROOT))+r'/(?:gate-proof|native-home-gate|native-home-gate-loader)-'+digest+r'\.(?:js|qml)',path),'Non-addressed gate asset')
            c.require(hashlib.sha256(c.trusted_bytes(path)).hexdigest()==digest,'Gate asset hash mismatch')
        c.require(package['entry']['file'] in package['files'],'Missing loader asset')
    def load_state(self):return c.load_state()
    def get_filters(self):return c.get_filters(c.luna_call)
    def read_originals(self):return (c.trusted_bytes(str(HOOK)).decode(),c.trusted_bytes(str(ROOT/'rollback.py')).decode())
    def atomic(self,path,data,mode=0o600):
        path=pathlib.Path(path);c.secure_parents(str(path))
        if path.exists():c.trusted_bytes(str(path))
        tmp=path.with_name(path.name+'.gate-new')
        fd=os.open(str(tmp),os.O_WRONLY|os.O_CREAT|os.O_EXCL|os.O_NOFOLLOW,mode)
        with os.fdopen(fd,'wb') as f:f.write(data);f.flush();os.fsync(f.fileno())
        os.chmod(str(tmp),mode);os.replace(str(tmp),str(path));fd=os.open(str(path.parent),os.O_RDONLY|os.O_DIRECTORY)
        try:os.fsync(fd)
        finally:os.close(fd)
        c.require(c.trusted_bytes(str(path))==data,'Atomic write readback mismatch')
    def install_safety(self):
        if TRIAL.exists():c.require(c.trusted_bytes(str(TRIAL)).decode()==TRIAL_HOOK,'Foreign trial hook')
        self.atomic(HOOK,SAFE_HOOK.encode(),0o755);self.atomic(ROOT/'rollback.py',SAFE_ROLLBACK.encode(),0o644);self.atomic(TRIAL,TRIAL_HOOK.encode(),0o755)
    def save_state(self,state):self.atomic(ROOT/c.STATE_NAME,json.dumps(state,indent=2).encode())
    def arm_guard(self):
        from gate_trial_guard import read_token
        pending=ROOT/'native-home-gate.pending'
        if read_token() is not None:pending.unlink();time.sleep(1.3)
        token=os.urandom(16).hex();self.atomic(pending,json.dumps({'token':token}).encode())
        p=subprocess.run([sys.executable,'-B','-E','-s',str(ROOT/'gate_trial_guard.py'),'--spawn'],stdin=subprocess.DEVNULL,stdout=subprocess.PIPE,stderr=subprocess.PIPE,timeout=10)
        c.require(p.returncode==0,p.stderr.decode());d=json.loads(p.stdout);c.require(d['armed'] is True and type(d['guardPid']) is int,'Trial guard not armed');os.kill(d['guardPid'],0)
    def set_filters(self,chain):c.checked_call(c.luna_call,'setConfigs',{'configs':{c.KEY:chain}})

SAFE_HOOK='''#!/bin/sh
# Apply existing native Home; release presentation gate on integrity failure.
set -eu
PATH=/usr/sbin:/usr/bin:/sbin:/bin
export PATH
ROOT=/var/lib/lg-native
[ -d "$ROOT" ] || exit 0
if [ -e "$ROOT/home-disabled" ] || [ -L "$ROOT/home-disabled" ]; then
    python3 -B -E -s "$ROOT/native_gate_control.py" --disable >"$ROOT/home-boot.log" 2>&1
    exit 0
fi
rc=0
python3 -B -E -s "$ROOT/nativeapply.py" --boot >"$ROOT/home-boot.log" 2>&1 || rc=$?
python3 -B -E -s "$ROOT/native_gate_control.py" --recover-if-unready >>"$ROOT/home-boot.log" 2>&1
exit "$rc"
'''
SAFE_ROLLBACK='''#!/usr/bin/env python3
"""Release owned presentation gate before existing native rollback."""
import sys
sys.dont_write_bytecode=True
if __name__=='__main__':
    if '--help' not in sys.argv:
        from native_gate_control import disable_gate,load_state,luna_call
        disable_gate(load_state(),luna_call)
    from native_home import cli
    sys.exit(cli('rollback'))
'''
TRIAL_HOOK='''#!/bin/sh
PATH=/usr/sbin:/usr/bin:/sbin:/bin
export PATH
exec /usr/bin/python3 -B -E -s /var/lib/lg-native/gate_trial_guard.py --spawn
'''

def install(package,io):
    io.check_files(package);previous=io.load_state();current=io.get_filters();hook,rollback=io.read_originals()
    chain=build_chain(current,package['entry'],previous);state=next_state(package,previous,current,hook,rollback)
    io.install_safety();io.save_state(state);io.arm_guard();io.set_filters(chain)
    c.require(io.get_filters()==chain,'Gate installation readback mismatch')
    return state

def next_state(package,previous,current,hook,rollback):
    c.validate_state(package)
    entries=list(previous.get('ownedEntries',[previous['entry']])) if previous else []
    if package['entry'] not in entries:entries.append(package['entry'])
    state=dict(package,ownedEntries=entries,originalKeyFilters17=previous['originalKeyFilters17'] if previous else current,originalHook=previous['originalHook'] if previous else hook,originalRollback=previous['originalRollback'] if previous else rollback)
    return c.validate_state(state)

def build_chain(current,entry,state):
    c.validate_entry(entry)
    remaining=c.remove_owned_filter(current,state) if state else list(current)
    slots=[i for i,e in enumerate(remaining) if isinstance(e,dict) and e.get('handler')=='handleSystemKeys' and e.get('file')=='/usr/lib/qt5/qml/KeyFilters/systemUi.js']
    c.require(len(slots)==1,'Expected one original system key handler')
    c.require(not any(isinstance(e,dict) and e.get('handler')=='handleNativeHomeGate' for e in remaining),'Unknown Home gate reference')
    remaining.insert(slots[0],entry)
    return remaining

if __name__=='__main__':
    io=LinuxIO()
    if sys.argv[1:]==['--check']:
        print(json.dumps({'nativeReady':c.native_ready(),'keyFilters':io.get_filters()}))
    elif len(sys.argv)==3 and sys.argv[1]=='--install':
        path=sys.argv[2];c.require(path in [str(ROOT/'gate-package-trial.json'),str(ROOT/'gate-package-final.json')],'Unknown package path')
        package=json.loads(c.trusted_bytes(path).decode());state=install(package,io)
        print(json.dumps({'installed':True,'entry':state['entry'],'filterCount':len(io.get_filters()),'nativeReady':c.native_ready()}))
    elif sys.argv[1:]==['--commit']:
        state=io.load_state();c.require(state['mode']=='final','Only clean final gate can commit')
        filters=io.get_filters();c.require(filters.count(state['entry'])==1,'Final reference missing');c.require(c.native_ready(),'Native overlays not ready')
        from gate_trial_guard import read_token
        pending=ROOT/'native-home-gate.pending'
        if read_token() is not None:pending.unlink()
        if TRIAL.exists():c.require(c.trusted_bytes(str(TRIAL)).decode()==TRIAL_HOOK,'Foreign trial hook');TRIAL.unlink()
        time.sleep(1.3);print(json.dumps({'committed':True,'filterCount':len(filters),'nativeReady':True}))
    else:raise SystemExit('Expected --check / --install PACKAGE / --commit')
