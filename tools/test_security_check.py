import unittest
from security_check import inspect_content, inspect_name

class PublicationChecks(unittest.TestCase):
    def test_real_credential_markers_are_rejected(self):
        self.assertTrue(inspect_content('-----BEGIN '+ 'OPENSSH PRIVATE KEY-----'))
        self.assertTrue(inspect_content('gh'+'p_'+'A'*36))
    def test_private_network_and_profile_paths_are_rejected(self):
        self.assertTrue(inspect_content('192.'+'168.8.99'))
        self.assertTrue(inspect_content('C:'+ '/Us'+'ers/'+'example/private'))
        self.assertFalse(inspect_content('http://127.0.0.1:18767/remote'))
        self.assertFalse(inspect_content('/var/lib/lg-native/home-final-manifest.json'))
    def test_sensitive_and_firmware_artifact_names_rejected(self):
        for p in ['.env','x/id_ed25519','backup.zip','firmware.so','original.ipk']:
            self.assertTrue(inspect_name(p),p)
        self.assertFalse(inspect_name('remote/remote_controls.py'))
    def test_checker_and_its_tests_do_not_embed_private_samples(self):
        import pathlib
        for name in ('security_check.py','test_security_check.py'):
            self.assertFalse(inspect_content(pathlib.Path(__file__).with_name(name).read_text()),name)
    def test_proprietary_source_notice_rejected(self):
        self.assertTrue(inspect_content('Confidential computer '+ 'software. Valid license from LG required'))

if __name__=='__main__':unittest.main()
