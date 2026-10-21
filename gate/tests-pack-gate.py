import hashlib,json,pathlib,subprocess,sys,unittest
D=pathlib.Path(__file__).resolve().parent
class PackageTests(unittest.TestCase):
 def test_content_addressed_final_bundle(self):
  p=subprocess.run([sys.executable,str(D/'pack-gate.py'),'final'],capture_output=True,text=True)
  self.assertEqual(p.returncode,0,p.stderr)
  m=json.loads(p.stdout);out=D/'packed-final'
  self.assertEqual(m['entry']['handler'],'handleNativeHomeGate');self.assertEqual(len(m['files']),3)
  for path,digest in m['files'].items():
   data=(out/pathlib.PurePosixPath(path).name).read_bytes();self.assertEqual(hashlib.sha256(data).hexdigest(),digest);self.assertIn(digest,path)
  q=(out/pathlib.PurePosixPath(m['qml']).name).read_text();loader=(out/pathlib.PurePosixPath(m['entry']['file']).name).read_text()
  self.assertIn(pathlib.PurePosixPath(m['proof']).name,q);self.assertIn(m['qml'],loader)
  for data in [q,loader]:
   self.assertNotIn('127.0.0.1',data);self.assertNotIn('bootEvents',data)
 def test_trial_trace_is_bounded_and_uses_immutable_sources(self):
  p=subprocess.run([sys.executable,str(D/'pack-gate.py'),'trial'],capture_output=True,text=True);self.assertEqual(p.returncode,0,p.stderr)
  m=json.loads(p.stdout);out=D/'packed-trial'
  q=(out/pathlib.PurePosixPath(m['qml']).name).read_text();loader=(out/pathlib.PurePosixPath(m['entry']['file']).name).read_text()
  self.assertIn('bootEvents',q);self.assertIn('127.0.0.1:18766/boot',q);self.assertIn('publishAttempts < 150',q)
  self.assertIn('traceSend:lunaCall.send',loader);self.assertIn('gate.recordBoot("init", null)',q);self.assertIn('gate.recordBoot("surface", item)',q)
  for path,digest in m['files'].items():self.assertEqual(hashlib.sha256((out/pathlib.PurePosixPath(path).name).read_bytes()).hexdigest(),digest)
if __name__=='__main__':unittest.main()
