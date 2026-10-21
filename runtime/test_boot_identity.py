import unittest,io,stat
from unittest.mock import patch
from types import SimpleNamespace
import native_home as n
K='2bb46c05-edcf-4649-b8f1-8afe6be375a1'
class BootIdentityTests(unittest.TestCase):
 def identity(self,mtime,unsafe=False):
  def info(path):
   if path==n.ROOT:return SimpleNamespace(st_mode=stat.S_IFDIR|0o700)
   self.assertEqual(path,'/tmp/webosbrew_startup')
   return SimpleNamespace(st_mode=(stat.S_IFLNK if unsafe else stat.S_IFREG)|0o644,st_uid=0,st_nlink=1,st_mtime_ns=mtime)
  with patch.object(n.sys,'platform','linux'),patch.object(n.os,'geteuid',return_value=0,create=True),patch.object(n.LinuxOps,'secure_parents'),patch.object(n.os,'lstat',side_effect=info),patch('builtins.open',side_effect=lambda *a,**k:io.StringIO(K)):
   return n.LinuxOps().boot_id
 def test_fastboot_same_kernel_uuid_new_hbc_start_is_new_boot(self):
  self.assertNotEqual(self.identity(1792458000000000000),self.identity(1792459000000000000))
 def test_same_start_stamp_is_stable(self):
  self.assertEqual(self.identity(1792458000000000000),self.identity(1792458000000000000))
 def test_untrusted_start_marker_rejected(self):
  with self.assertRaises(ValueError):self.identity(1792458000000000000,unsafe=True)
if __name__=='__main__':unittest.main()
