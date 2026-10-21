import hashlib
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
import native_home as n
from test_native_home import items

HERE = Path(__file__).parent

class Adapter(unittest.TestCase):
    def adapter(self):
        self.assertTrue(hasattr(n, 'LinuxOps'), 'Linux adapter missing')
        return n.LinuxOps.__new__(n.LinuxOps)

    def test_exact_command_vectors_no_shell_global_state_or_other_processes(self):
        o = self.adapter(); e = items()[0]
        with patch.object(n.subprocess, 'run') as run:
            run.return_value.returncode = 0; run.return_value.stdout = '{}'
            o.bind(e); o.readonly(e); o.unmount(e)
            self.assertEqual([c.args[0] for c in run.call_args_list], [
                ['mount', '--bind', e['source'], e['target']],
                ['mount', '-o', 'remount,bind,ro', e['target']], ['umount', e['target']]])
            for call in run.call_args_list:
                self.assertFalse(call.kwargs.get('shell', False)); self.assertLessEqual(call.kwargs['timeout'], 10)
        with patch.object(o, 'trusted_file'), patch.object(o, 'command') as command:
            o.clear_checkpoint()
            self.assertEqual(command.call_args.args[0], [n.sys.executable, '-B', '-E', '-s', n.ROOT + '/clear-home-checkpoint.py', '--apply'])
        with patch.object(o, 'executables', return_value={1: n.EXE, 2: n.EXE + '.old', 3: '/usr/sbin/sdx'}), \
             patch.object(n.os, 'readlink', return_value=n.EXE), patch.object(n.os, 'kill') as kill, \
             patch.object(n.time, 'sleep'), patch.object(n.signal, 'SIGKILL', 9, create=True):
            o.kill_home()
            kill.assert_called_once_with(1, n.signal.SIGKILL)

    def test_immutable_existing_copy_is_verified_and_never_rewritten(self):
        o = self.adapter()
        e = dict(items()[0], inputSource=items()[0]['source'], source=n.ROOT + '/mount-copies/test', uid=0, gid=0, mode=0o644)
        with patch.object(o, 'trusted_file', return_value={'uid': 0, 'gid': 0, 'mode': 0o644}) as trusted, \
             patch.object(o, 'secure_parents'), patch.object(n.os.path, 'lexists', return_value=True), \
             patch.object(n.os, 'O_NOFOLLOW', 0x20000, create=True), \
             patch.object(n.os, 'open', side_effect=AssertionError('existing copy must not be rewritten')) as opened:
            o.copy_source(e)
            opened.assert_not_called()
            self.assertIn(((e['source'], e['newHash']),), [(c.args,) for c in trusted.call_args_list])

    def test_luna_contract_readback_and_background_payload(self):
        o = self.adapter()
        policy = {'returnValue': True, 'applications': [{'id': n.HOME, 'isEnabled': False}]}
        with patch.object(o, 'command', return_value=json.dumps(policy)):
            self.assertFalse(o.preload())
        with patch.object(o, 'luna', return_value={'returnValue': True}) as luna, \
             patch.object(o, 'preload', return_value=False):
            o.set_preload(False); o.launch(n.launch_payload('airplay'))
            self.assertEqual(luna.call_args.args, ('com.webos.applicationManager/launch', {'id': n.HOME, 'preload': 'full'}))
        with patch.object(o, 'command', return_value='{"returnValue":false}'):
            with self.assertRaises(ValueError): o.luna('service/method', {})

    def test_mountinfo_parser_decodes_paths_and_duplicate_mounts(self):
        self.adapter()
        raw = '10 1 8:1 /file /some\\040path ro,relatime - ext4 /dev/a rw\n11 1 8:1 /file /some\\040path ro - ext4 /dev/a rw\n'
        mounts = n.parse_mountinfo(raw)
        self.assertEqual([m['target'] for m in mounts], ['/some path', '/some path'])
        self.assertEqual([m['id'] for m in mounts], [10, 11])
        self.assertIn('ro', mounts[0]['options'])
        with self.assertRaises(ValueError): n.parse_mountinfo('broken')

    def test_readiness_deadline_with_criu_active_has_no_luna_or_mutations(self):
        o = self.adapter(); clock = [0.0]
        def sleep(seconds): clock[0] += seconds
        with patch.object(n.time, 'monotonic', side_effect=lambda: clock[0]), \
             patch.object(n.time, 'sleep', side_effect=sleep), \
             patch.object(n.os.path, 'lexists', return_value=True), patch.object(o, 'luna') as luna:
            with self.assertRaises(TimeoutError): o.wait_ready(8.0)
            self.assertLessEqual(clock[0], 8.0); self.assertFalse(luna.called)

    def test_boot_claim_is_once_per_boot(self):
        o = self.adapter(); o.boot_id = 'boot-a'; store = {}
        with patch.object(o, 'read_json', side_effect=lambda path, optional=False: store.get(path)), \
             patch.object(o, 'write_json', side_effect=lambda path, data: store.update({path: data})):
            self.assertTrue(o.claim_boot()); self.assertFalse(o.claim_boot())
            o.boot_id = 'boot-b'; self.assertTrue(o.claim_boot())

    def test_checkpoint_helper_is_unmodified_and_pinned(self):
        self.adapter()
        candidate = HERE / 'clear-home-checkpoint.py'
        self.assertTrue(candidate.exists(), 'exact existing helper copy missing')
        self.assertEqual(hashlib.sha256(candidate.read_bytes()).hexdigest(), n.CHECKPOINT_HELPER_SHA256)

if __name__ == '__main__': unittest.main()
