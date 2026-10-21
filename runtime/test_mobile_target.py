import json,pathlib,unittest
import native_home as n
class MobileTarget(unittest.TestCase):
 def setUp(self):
  self.baseline=[e for e in json.loads((pathlib.Path(__file__).parent.parent/'gate/fixtures/home-final-manifest.json').read_text()) if not e['target'].endswith('/MobileComponent.qml')]
  self.assertEqual(len(self.baseline),7)
  self.extra=dict(source=n.ROOT+'/six-MobileComponent.qml',target=n.QML+'UserInterfaceLayer/Containers/Sub/MobileComponent.qml',originalHash='a'*64,newHash='b'*64)
 def test_accept_exact_eighth_visual_component(self):
  self.assertEqual(n.validate_manifest(self.baseline+[self.extra]),self.baseline+[self.extra])
 def test_legacy_seven_recovery(self):
  self.assertEqual(n.validate_manifest(self.baseline),self.baseline)
 def test_reject_foreign_ninth_component(self):
  extra=dict(self.extra,source=n.ROOT+'/bad.qml',target=n.QML+'UserInterfaceLayer/Containers/Sub/Other.qml')
  with self.assertRaises(ValueError):n.validate_manifest(self.baseline+[self.extra,extra])
  with self.assertRaises(ValueError):n.validate_manifest(self.baseline+[extra])
 def test_reject_duplicate_mobile(self):
  with self.assertRaises(ValueError):n.validate_manifest(self.baseline+[self.extra,self.extra])
if __name__=='__main__':unittest.main()
