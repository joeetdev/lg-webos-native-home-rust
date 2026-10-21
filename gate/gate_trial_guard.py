import time,json,os,pathlib,re,stat,subprocess,sys
ROOT=pathlib.Path('/var/lib/lg-native')

def read_token():
    p=ROOT/'native-home-gate.pending'
    try:s=p.lstat()
    except FileNotFoundError:return None
    if not (stat.S_ISREG(s.st_mode) and s.st_uid==0 and s.st_nlink==1 and not s.st_mode&0o022 and s.st_size<4096):raise ValueError('Untrusted trial token')
    token=json.loads(p.read_text())['token']
    if not isinstance(token,str) or re.fullmatch('[0-9a-f]{32}',token) is None:raise ValueError('Invalid trial token')
    return token



def watch(token,get_token,disable,clock=time.monotonic,sleep=time.sleep,timeout=240):
    deadline=clock()+timeout
    while clock()<deadline:
        if get_token()!=token:return 'cancelled'
        sleep(1)
    if get_token()!=token:return 'cancelled'
    disable()
    return 'recovered'

if __name__=='__main__':
    if not sys.platform.startswith('linux') or os.geteuid()!=0 or pathlib.Path(__file__).resolve()!=ROOT/'gate_trial_guard.py':raise SystemExit('Requires installed private Linux root guard')
    if sys.argv[1:]==['--spawn']:
        token=read_token();pid=None
        if token:
            g=subprocess.Popen([sys.executable,'-B','-E','-s',str(ROOT/'gate_trial_guard.py'),'--watch',token],stdin=subprocess.DEVNULL,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL,start_new_session=True,close_fds=True);pid=g.pid
        print(json.dumps({'guardPid':pid,'armed':token is not None}))
    elif len(sys.argv)==3 and sys.argv[1]=='--watch' and re.fullmatch('[0-9a-f]{32}',sys.argv[2]):
        def disable():
            subprocess.run([sys.executable,'-B','-E','-s',str(ROOT/'native_gate_control.py'),'--disable'],stdin=subprocess.DEVNULL,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL,timeout=30,check=True)
        print(json.dumps({'result':watch(sys.argv[2],read_token,disable)}))
    else:raise SystemExit('Expected --spawn or --watch TOKEN')
