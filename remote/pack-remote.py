import pathlib,json,hashlib
D=pathlib.Path(__file__).resolve().parent;R='/var/lib/lg-native/'
def build(mode,out):
 assert mode in ('trial','final');out.mkdir(parents=True,exist_ok=True);files={}
 def add(prefix,text,extension):
  b=text.encode();h=hashlib.sha256(b).hexdigest();n=prefix+'-'+h+extension;(out/n).write_bytes(b);files[R+n]=h;return R+n
 core=add('remote-core',(D/'remote-core.js').read_text(),'.js');qml=add('remote-controller',(D/'remote-controller.qml').read_text(),'.qml');s=(D/'remote-loader.template.js').read_text().replace('__CORE_URL__',json.dumps('file://'+core)).replace('__QML_URL__',json.dumps('file://'+qml)).replace('__TRIAL_MS__','0')
 trace='''function remoteTrace(event){var x=new XMLHttpRequest();x.open('POST','http://127.0.0.1:18767/remote');x.setRequestHeader('Content-Type','application/json');x.send(JSON.stringify({time:Date.now(),event:event,blocked:remoteController?remoteController.pointerBlocked:null,power:global.powerState}));}\n'''
 s=s.replace('__TRACE_KEYS__',"if(key===18874398||key===18874413)remoteTrace('key:'+key+':'+pressed+':'+autoRepeat+':'+deviceId+':model='+!!remoteControls);" if mode=='trial' else '')
 s=s.replace('__TRACE_MOVIES__',"remoteTrace('movies');" if mode=='trial' else '').replace('__TRACE_INIT__',"if(typeof remoteController.backend.cursorVisibleChanged.connect!=='function')throw new Error('Cursor notification unavailable');remoteController.pointerBlockedChanged.connect(function(){remoteTrace('pointer');});remoteTrace('ready');" if mode=='trial' else '')
 if mode=='trial':
  s=trace+s.replace('function init() {','function init() {try {').replace('\n}\nfunction handleRemoteControls',"\n}catch(e){remoteTrace('error:'+String(e));throw e;}\n}\nfunction handleRemoteControls")
 entry={'file':add('remote-loader',s,'.js'),'handler':'handleRemoteControls'};p={'version':1,'owner':'lg-native-remote-controls-v1','mode':mode,'entry':entry,'files':files};(out/'package.json').write_text(json.dumps(p,indent=2));return p
if __name__=='__main__':
 for mode in ('trial','final'):print(json.dumps(build(mode,D/('remote-packed-'+mode)),indent=2))
