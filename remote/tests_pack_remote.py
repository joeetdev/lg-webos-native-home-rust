import unittest,pathlib,importlib.util,tempfile,hashlib,json
D=pathlib.Path(__file__).resolve().parent
class Pack(unittest.TestCase):
 def test_trial_and_final_immutable_dependencies(self):
  spec=importlib.util.spec_from_file_location('pack',D/'pack-remote.py');m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m)
  with tempfile.TemporaryDirectory(dir=str(D)) as t:
   a=m.build('trial',pathlib.Path(t)/'trial');b=m.build('final',pathlib.Path(t)/'final')
   for p,sub in [(a,'trial'),(b,'final')]:
    self.assertEqual(len(p['files']),3);self.assertEqual(p['entry']['handler'],'handleRemoteControls')
    for name,h in p['files'].items():
     text=(pathlib.Path(t)/sub/pathlib.PurePosixPath(name).name).read_bytes();self.assertEqual(hashlib.sha256(text).hexdigest(),h);self.assertIn(h,pathlib.PurePosixPath(name).name)
   trial=(pathlib.Path(t)/'trial'/pathlib.PurePosixPath(a['entry']['file']).name).read_text();self.assertIn('trialMilliseconds = 0',trial)
   self.assertNotEqual(a['entry'],b['entry']);s=(pathlib.Path(t)/'final'/pathlib.PurePosixPath(b['entry']['file']).name).read_text();self.assertNotIn('127.0.0.1',s);self.assertNotIn('__',s);self.assertIn('trialMilliseconds = 0',s)
if __name__=='__main__':unittest.main()
