import importlib.util
from pathlib import Path
import subprocess
import sys
import unittest
from unittest.mock import patch, MagicMock
import native_home as n

HERE = Path(__file__).parent

class EntryPoints(unittest.TestCase):
    def test_cli_boot_once_disabled_and_timeout_are_nonmutating(self):
        self.assertTrue(hasattr(n, 'cli'), 'entrypoint workflow missing')
        for scenario in ('disabled', 'already-run', 'not-ready', 'ready'):
            with self.subTest(scenario=scenario), patch.object(n, 'LinuxOps') as factory, \
                 patch.object(n, 'Controller') as controller:
                o = factory.return_value
                o.is_disabled.return_value = scenario == 'disabled'
                o.claim_boot.return_value = scenario != 'already-run'
                if scenario == 'not-ready': o.wait_ready.side_effect = TimeoutError('busy')
                code = n.cli('apply', ['--boot'])
                self.assertEqual(code, 1 if scenario == 'not-ready' else 0)
                self.assertEqual(controller.return_value.apply.called, scenario == 'ready')
                o.unlock.assert_called_once()

    def test_rollback_background_dispatch_is_manifest_independent(self):
        self.assertTrue(hasattr(n, 'cli'), 'entrypoint workflow missing')
        with patch.object(n, 'LinuxOps') as factory, patch.object(n, 'Controller') as controller:
            self.assertEqual(n.cli('rollback', ['--background']), 0)
            controller.return_value.rollback.assert_called_once_with(background=True)
            factory.return_value.load_manifest.assert_not_called()

    def test_scripts_help_and_hook_syntax_local_only(self):
        for name in ('nativeapply.py', 'rollback.py', '55-lg-native'):
            self.assertTrue((HERE / name).is_file(), 'missing entrypoint ' + name)
        for name in ('nativeapply.py', 'rollback.py'):
            result = subprocess.run([sys.executable, str(HERE / name), '--help'], capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn('usage:', result.stdout)
        result = subprocess.run(['sh', '-n', '55-lg-native'], cwd=HERE, capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        text = (HERE / '55-lg-native').read_text()
        self.assertIn('--boot', text)
        self.assertNotIn('while', text)
        self.assertNotIn('&', text.replace('2>&1', ''))  # stderr redirection is not backgrounding

if __name__ == '__main__': unittest.main()
