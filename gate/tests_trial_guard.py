import unittest,importlib.util,pathlib
P=pathlib.Path(__file__).with_name('gate_trial_guard.py')
class GuardTests(unittest.TestCase):
 def load(self):
  s=importlib.util.spec_from_file_location('trial_guard',P);m=importlib.util.module_from_spec(s);s.loader.exec_module(m);return m
 def test_expiry_disables_only_the_same_unconfirmed_trial(self):
  m=self.load();now=[0];calls=[]
  def sleep(n):now[0]+=n
  self.assertEqual(m.watch('token',lambda:'token',lambda:calls.append('disabled'),lambda:now[0],sleep,3),'recovered')
  self.assertEqual(calls,['disabled'])
 def test_commit_cancels_guard_immediately_without_waiting(self):
  m=self.load();now=[0];calls=[]
  self.assertEqual(m.watch('token',lambda:None,lambda:calls.append('disabled'),lambda:now[0],lambda n:now.__setitem__(0,now[0]+n),3),'cancelled')
  self.assertEqual((now[0],calls),(0,[]))
 def test_read_trusted_nonce_and_reject_untrusted_token_file(self):
  import json,stat,types
  from unittest.mock import MagicMock
  m=self.load();p=MagicMock();m.ROOT=MagicMock();m.ROOT.__truediv__.return_value=p
  p.lstat.return_value=types.SimpleNamespace(st_mode=stat.S_IFREG|0o600,st_uid=0,st_nlink=1,st_size=50)
  p.read_text.return_value=json.dumps({'token':'a'*32});self.assertEqual(m.read_token(),'a'*32)
  for field,bad in [('st_mode',stat.S_IFLNK|0o600),('st_uid',1),('st_nlink',2),('st_mode',stat.S_IFREG|0o622),('st_size',5000)]:
   before=getattr(p.lstat.return_value,field);setattr(p.lstat.return_value,field,bad)
   with self.subTest(field=field,bad=bad),self.assertRaises(ValueError):m.read_token()
   setattr(p.lstat.return_value,field,before)
 def test_absent_token_means_cancelled_and_malformed_nonce_is_rejected(self):
  import types,stat,json
  from unittest.mock import MagicMock
  m=self.load();p=MagicMock();m.ROOT=MagicMock();m.ROOT.__truediv__.return_value=p;p.lstat.side_effect=FileNotFoundError
  self.assertIsNone(m.read_token());p.lstat.side_effect=None
  p.lstat.return_value=types.SimpleNamespace(st_mode=stat.S_IFREG|0o600,st_uid=0,st_nlink=1,st_size=50)
  for v in [None,12,'short','A'*32,'../'+'a'*32]:
   p.read_text.return_value=json.dumps({'token':v})
   with self.subTest(value=v),self.assertRaises(ValueError):m.read_token()
 @unittest.skipIf(__import__('sys').platform.startswith('linux'),'host-only CLI safety gate')
 def test_cli_cannot_spawn_on_nonlinux_host(self):
  import subprocess,sys
  p=subprocess.run([sys.executable,str(P),'--spawn'],capture_output=True,text=True)
  self.assertNotEqual(p.returncode,0)
if __name__=='__main__':unittest.main()
