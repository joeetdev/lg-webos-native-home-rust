import copy,json,pathlib,unittest
import native_home as n
class DashboardTarget(unittest.TestCase):
 def setUp(self):
  self.entries=[e for e in json.loads((pathlib.Path(__file__).parent.parent/'gate/fixtures/home-final-manifest.json').read_text()) if not e['target'].endswith(('/HomeDashboard.qml','/MobileComponent.qml'))]
  self.assertEqual(len(self.entries),6)
  self.extra=dict(source=n.ROOT+'/categorized-HomeDashboard.qml',target=n.QML+'UserInterfaceLayer/Containers/Main/HomeDashboard.qml',originalHash='a'*64,newHash='b'*64)
 def test_accepts_seven_exact_home_targets(self):
  self.assertEqual(n.validate_manifest(self.entries+[self.extra]),self.entries+[self.extra])
 def test_legacy_six_remain_valid_for_recovery(self):
  self.assertEqual(n.validate_manifest(self.entries),self.entries)
 def test_eighth_or_foreign_target_rejected(self):
  bad=dict(self.extra,target=n.QML+'UserInterfaceLayer/Containers/Main/Other.qml',source=n.ROOT+'/extra.qml')
  with self.assertRaises(ValueError):n.validate_manifest(self.entries+[self.extra,bad])
  with self.assertRaises(ValueError):n.validate_manifest(self.entries+[bad])
if __name__=='__main__':unittest.main()
