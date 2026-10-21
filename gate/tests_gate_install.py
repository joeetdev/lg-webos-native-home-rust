import unittest,copy
import gate_install as g
class InstallTests(unittest.TestCase):
 def test_insert_before_system_keys_preserves_every_other_handler(self):
  before=[{'file':'/sys/a.js','handler':'safety'},{'file':'/usr/lib/qt5/qml/KeyFilters/systemUi.js','handler':'handleSystemKeys'},{'file':'/sys/z.js','handler':'later'}]
  entry={'file':'/var/lib/lg-native/native-home-gate-loader-'+'a'*64+'.js','handler':'handleNativeHomeGate'};saved=copy.deepcopy(before)
  self.assertEqual(g.build_chain(before,entry,None),[before[0],entry,before[1],before[2]]);self.assertEqual(before,saved)
 def test_next_state_records_both_entries_before_config_transition(self):
  a={'file':'/var/lib/lg-native/native-home-gate-loader-'+'a'*64+'.js','handler':'handleNativeHomeGate'};b=dict(a,file=a['file'].replace('a'*64,'b'*64))
  old={'version':1,'owner':'lg-native-home-gate-v1','entry':a,'ownedEntries':[a],'originalKeyFilters17':['original'],'originalHook':'hook','originalRollback':'rollback'}
  package={'version':1,'owner':'lg-native-home-gate-v1','entry':b,'files':{},'mode':'final'}
  state=g.next_state(package,old,['current'],'unused','unused')
  self.assertEqual(state['ownedEntries'],[a,b]);self.assertEqual(state['originalKeyFilters17'],['original']);self.assertEqual(state['originalHook'],'hook')
 def test_transaction_arms_recovery_and_durably_records_ownership_before_activation(self):
  from types import SimpleNamespace
  entry={'file':'/var/lib/lg-native/native-home-gate-loader-'+'a'*64+'.js','handler':'handleNativeHomeGate'}
  package={'version':1,'owner':'lg-native-home-gate-v1','entry':entry,'files':{},'mode':'trial'};events=[];filters=[{'file':'/usr/lib/qt5/qml/KeyFilters/systemUi.js','handler':'handleSystemKeys'}]
  io=SimpleNamespace(check_files=lambda p:events.append('files'),load_state=lambda:None,get_filters=lambda:list(filters),read_originals=lambda:('hook','rollback'),install_safety=lambda:events.append('safety'),save_state=lambda s:events.append('state'),arm_guard=lambda:events.append('guard'),set_filters=lambda x:(events.append('config'),filters.__setitem__(slice(None),x)))
  g.install(package,io);self.assertEqual(events,['files','safety','state','guard','config']);self.assertEqual(filters[0],entry)
 def test_recovery_wrapper_disables_gate_before_native_rollback(self):
  import sys,types
  from unittest.mock import patch
  events=[];helper=types.SimpleNamespace(load_state=lambda:{},luna_call=lambda *a:None,disable_gate=lambda *a:events.append('disable'))
  native=types.SimpleNamespace(cli=lambda action:events.append(action) or 0)
  with patch.dict(sys.modules,{'native_gate_control':helper,'native_home':native}),patch.object(sys,'argv',['rollback.py']),self.assertRaises(SystemExit):exec(g.SAFE_ROLLBACK,{'__name__':'__main__'})
  self.assertEqual(events,['disable','rollback']);self.assertIn('--recover-if-unready',g.SAFE_HOOK);self.assertIn('--disable',g.SAFE_HOOK)
 @unittest.skipIf(__import__('sys').platform.startswith('linux'),'host OS safety check')
 def test_live_io_refuses_nonlinux_host_before_any_write(self):
  with self.assertRaises(ValueError):g.LinuxIO()
if __name__=='__main__':unittest.main()
