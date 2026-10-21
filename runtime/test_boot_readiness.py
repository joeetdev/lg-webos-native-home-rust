import unittest
from unittest.mock import patch
import native_home as n
class BootReadinessTests(unittest.TestCase):
 def test_boot_cli_requests_bounded_45_second_startup_budget(self):
  with patch.object(n,'LinuxOps') as factory,patch.object(n,'Controller'):
   ops=factory.return_value;ops.is_disabled.return_value=False;ops.claim_boot.return_value=True
   self.assertEqual(n.cli('apply',['--boot']),0)
   ops.wait_ready.assert_called_once_with(45.0)
 def test_preload_controller_starting_late_is_waited_for(self):
  ops=object.__new__(n.LinuxOps);clock=[0.0]
  def luna(method,payload,timeout):
   if 'preloadmanager' in method and clock[0]<15:raise TimeoutError('controller still starting')
   return {'returnValue':True}
  def sleep(t):clock[0]+=t
  with patch.object(n.time,'monotonic',side_effect=lambda:clock[0]),patch.object(n.time,'sleep',side_effect=sleep),patch.object(n.os.path,'lexists',return_value=False),patch.object(ops,'luna',side_effect=luna):
   ready=False
   try:ops.wait_ready(45.0);ready=True
   except TimeoutError:pass
   self.assertTrue(ready,'boot aborted before preload controller became ready')
   self.assertGreaterEqual(clock[0],15);self.assertLessEqual(clock[0],45)
if __name__=='__main__':unittest.main()
