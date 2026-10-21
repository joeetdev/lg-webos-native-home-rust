import pathlib,hashlib,json,sys
D=pathlib.Path(__file__).resolve().parent;R='/var/lib/lg-native';mode=sys.argv[1];assert mode in ['trial','final']
def sha(s):return hashlib.sha256(s.encode()).hexdigest()
proof=(D/'gate-proof.js').read_text();pn='gate-proof-'+sha(proof)+'.js'
qml=(D/'native-home-gate.qml').read_text().replace('gate-proof.js',pn)
if mode=='trial':
 qml=qml.replace('    property int attempts: 0\n','    property int attempts: 0\n'+(D/'gate-trial-trace.qmlpart').read_text(),1)
 qml=qml.replace('    Component.onCompleted: {\n','    Component.onCompleted: {\n        gate.recordBoot("init", null);\n',1)
 qml=qml.replace('        gate.attempts = 0;\n        gate.refresh();','        gate.attempts = 0;\n        gate.refresh();\n        gate.recordBoot("surface", item);',1)
qn='native-home-gate-'+sha(qml)+'.qml'
loader=(D/'native-home-gate-loader.js').read_text().replace(R+'/native-home-gate.qml',R+'/'+qn)
if mode=='trial':loader=loader.replace('component.createObject(root)','component.createObject(root, {traceSend:lunaCall.send})',1)
ln='native-home-gate-loader-'+sha(loader)+'.js'
files={pn:proof,qn:qml,ln:loader};out=D/('packed-'+mode);out.mkdir(exist_ok=True)
for n,s in files.items():
 p=out/n
 if p.exists():assert p.read_text()==s
 else:p.write_text(s,encoding='utf-8',newline='\n')
meta={'version':1,'owner':'lg-native-home-gate-v1','mode':mode,'entry':{'file':R+'/'+ln,'handler':'handleNativeHomeGate'},'files':{R+'/'+n:sha(s) for n,s in files.items()},'qml':R+'/'+qn,'proof':R+'/'+pn}
(out/'package.json').write_text(json.dumps(meta,indent=2));print(json.dumps(meta,indent=2))
