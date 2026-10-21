"""Model only the Linux/LS2 boundary; execute the real controller and guards."""
import copy
import unittest
import native_home as n
from test_native_home import items, observed, ROOT

class FakeOps:
    def __init__(self, enabled=True, fg='com.webos.app.home', fail=None):
        self.manifest = items(); self.saved = []; self.journal = None
        self.files = {e['source']: observed(e['source'], ino=i + 20) for i, e in enumerate(self.manifest)}
        self.stock = {e['target']: observed(e['target'], 'a' * 64, i + 100) for i, e in enumerate(self.manifest)}
        self.files.update(copy.deepcopy(self.stock))
        self.live = []; self.events = []; self.enabled = enabled; self.fg = fg
        self.disabled = False; self.fail = fail; self.boot_id = 'boot-a'

    def hit(self, action, value=None):
        self.events.append((action, value))
        if action == self.fail:
            self.fail = None
            raise RuntimeError('injected ' + action)

    def is_disabled(self): return self.disabled
    def load_manifest(self): return copy.deepcopy(self.manifest)
    def load_journal(self): return copy.deepcopy(self.journal)
    def save_journal(self, journal):
        self.journal = copy.deepcopy(journal); self.saved.append(copy.deepcopy(journal))
        self.events.append(('journal', journal.get('action')))
    def disable(self): self.hit('disable'); self.disabled = True
    def foreground(self): return self.fg
    def preload(self): return self.enabled
    def set_preload(self, value): self.hit('preload', value); self.enabled = value
    def inspect(self, path):
        self.events.append(('inspect', path))
        return copy.deepcopy(self.files[path])
    def mounts(self): return copy.deepcopy(self.live)
    def copy_source(self, entry):
        self.hit('copy', entry['source'])
        self.files[entry['source']] = observed(entry['source'], entry['newHash'], 300 + len(self.files))
        self.files[entry['source']].update({k: entry[k] for k in ('uid', 'gid', 'mode')})
    def bind(self, entry):
        self.hit('bind', entry['target'])
        self.files[entry['target']] = dict(self.files[entry['source']], path=entry['target'], realpath=entry['target'])
        self.live.append(dict(target=entry['target'], id=500 + len(self.live), options=['rw']))
    def readonly(self, entry):
        self.hit('readonly', entry['target'])
        next(m for m in self.live if m['target'] == entry['target'])['options'] = ['ro']
    def unmount(self, entry):
        self.hit('unmount', entry['target'])
        self.live = [m for m in self.live if m['target'] != entry['target']]
        self.files[entry['target']] = copy.deepcopy(self.stock[entry['target']])
    def clear_checkpoint(self): self.hit('clear')
    def kill_home(self): self.hit('kill')
    def launch(self, payload): self.hit('launch', payload)

class Transaction(unittest.TestCase):
    def controller(self, ops):
        self.assertTrue(hasattr(n, 'Controller'), 'runtime transaction missing')
        return n.Controller(ops)

    def test_success_journals_before_mounts_and_preserves_original_metadata(self):
        o = FakeOps(); c = self.controller(o); c.apply()
        self.assertEqual(len(o.live), 6)
        self.assertTrue(o.enabled)
        firstbind = next(i for i, e in enumerate(o.events) if e[0] == 'bind')
        inspected = {v for k, v in o.events[:firstbind] if k == 'inspect'}
        self.assertTrue({e['target'] for e in o.manifest} <= inspected)
        for i, (action, target) in enumerate(o.events):
            if action in ('copy', 'bind', 'readonly', 'unmount', 'clear', 'kill', 'launch', 'preload'):
                self.assertEqual(o.events[i - 1][0], 'journal', (action, o.events[i-1]))
        for e in o.journal['mounts']:
            self.assertEqual(o.files[e['source']]['mode'], o.stock[e['target']]['mode'])
            self.assertIn('ro', next(m for m in o.live if m['target'] == e['target'])['options'])
        actions = [k for k, _ in o.events]
        self.assertLess(actions.index('clear'), actions.index('kill'))
        self.assertFalse(o.journal['preload']['pending'])

    def test_failure_restores_both_original_preload_values_and_owned_mounts(self):
        for enabled in (True, False):
            for failure in ('preload', 'copy', 'bind', 'readonly', 'clear', 'kill', 'launch'):
                with self.subTest(enabled=enabled, failure=failure):
                    o = FakeOps(enabled=enabled, fg='airplay', fail=failure)
                    c = self.controller(o)
                    with self.assertRaises(RuntimeError): c.apply()
                    self.assertEqual(o.enabled, enabled)
                    self.assertTrue(o.disabled)
                    self.assertEqual(o.live, [])
                    launches = [v for k, v in o.events if k == 'launch']
                    self.assertTrue(all(v.get('preload') == 'full' for v in launches))

    def test_hash_change_has_no_mount_and_leaves_disabled(self):
        o = FakeOps(); o.stock[o.manifest[-1]['target']]['digest'] = 'c' * 64
        o.files.update(copy.deepcopy(o.stock)); c = self.controller(o)
        with self.assertRaises(ValueError): c.apply()
        self.assertFalse(any(k == 'bind' for k, _ in o.events)); self.assertTrue(o.disabled)
        self.assertTrue(o.enabled)

    def test_update_removes_only_owned_legacy_mounts_once_then_checks_stock(self):
        o = FakeOps()
        old = items(3)
        for e in old: o.bind(e)
        o.journal = old
        o.events.clear()
        self.controller(o).apply()
        removed = [v for k, v in o.events if k == 'unmount']
        self.assertEqual(removed, list(reversed([e['target'] for e in old])))
        self.assertEqual(len(o.live), 6)

    def test_unowned_mount_and_corrupt_journal_never_unmounted(self):
        for corrupt in (False, True):
            o = FakeOps()
            o.live = [dict(target=items()[0]['target'], id=1, options=['ro'])]
            if corrupt:
                o.journal = {'version': 1, 'owner': 'someone-else', 'mounts': items(3)}
            with self.assertRaises(ValueError): self.controller(o).apply()
            self.assertFalse(any(k == 'unmount' for k, _ in o.events))

    def test_rollback_is_manifest_independent_background_and_persistent(self):
        o = FakeOps(enabled=False)
        self.controller(o).apply()
        o.manifest = None; o.events.clear()
        self.controller(o).rollback(background=True)
        self.assertTrue(o.disabled); self.assertFalse(o.enabled); self.assertFalse(o.live)
        self.assertEqual([v for k, v in o.events if k == 'launch'], [{'id': n.HOME, 'preload': 'full'}])
        o.events.clear(); self.controller(o).rollback(background=True)
        self.assertFalse(any(k == 'unmount' for k, _ in o.events))

    def test_disabled_is_noop_and_crash_preload_intent_recovers(self):
        o = FakeOps(); o.disabled = True
        self.controller(o).apply(); self.assertFalse(o.events)
        o = FakeOps(enabled=False)
        o.journal = dict(version=1, owner='lg-native-home-v1', mounts=[],
                         preload=dict(original=True, bootId='boot-a', pending=True))
        self.controller(o).rollback(background=True)
        self.assertTrue(o.enabled)

class RecoveryRegressions(unittest.TestCase):
    def test_repeated_apply_reuses_immutable_copy_paths_without_accumulation(self):
        o = FakeOps()
        n.Controller(o).apply()
        first = [e['source'] for e in o.journal['mounts']]
        n.Controller(o).apply()
        second = [e['source'] for e in o.journal['mounts']]
        self.assertEqual(second, first)
        self.assertEqual(len(set(second)), 6)

    def test_failed_restore_retains_original_for_next_invocation(self):
        o = FakeOps()
        class FailRestore(FakeOps):
            def set_preload(self, value):
                if value: raise RuntimeError('restore API unavailable')
                super().set_preload(value)
        o = FailRestore()
        with self.assertRaises(RuntimeError): n.Controller(o).apply()
        self.assertTrue(o.journal['preload']['pending'])
        self.assertTrue(o.journal['preload']['original'])
        o.set_preload = lambda value: FakeOps.set_preload(o, value)
        n.Controller(o).rollback(background=True)
        self.assertTrue(o.enabled)

    def test_old_boot_pending_does_not_restore_stale_value_when_policy_unavailable(self):
        o = FakeOps(enabled=False)
        o.journal = dict(version=1, owner=n.OWNER, mounts=[],
                         preload=dict(original=True, bootId='boot-old', pending=True))
        o.preload = lambda: (_ for _ in ()).throw(RuntimeError('policy unavailable'))
        with self.assertRaises(Exception): n.Controller(o).rollback(background=True)
        self.assertFalse(o.enabled, 'previous-boot state must not overwrite current policy')

    def test_firmware_drift_under_legacy_mounts_unmounts_owned_then_aborts(self):
        o = FakeOps(); old = items(3)
        for e in old: o.bind(e)
        o.journal = old
        o.stock[old[0]['target']]['digest'] = 'c' * 64
        o.events.clear()
        with self.assertRaises(ValueError): n.Controller(o).apply()
        self.assertEqual(o.live, [])
        self.assertFalse(any(k == 'bind' for k, v in o.events))
        self.assertTrue(o.disabled)

    def test_foreground_changes_mid_transaction_never_steal_focus(self):
        o = FakeOps()
        original_copy = o.copy_source
        def copy_source(e):
            original_copy(e); o.fg = 'airplay'
        o.copy_source = copy_source
        n.Controller(o).apply()
        self.assertEqual([v for k, v in o.events if k == 'launch'], [{'id': n.HOME, 'preload': 'full'}])

    def test_foreground_api_failure_uses_background_and_restores_pending_state(self):
        o = FakeOps(enabled=False)
        o.journal = dict(version=1, owner=n.OWNER, mounts=[],
                         preload=dict(original=True, bootId='boot-a', pending=True))
        def unavailable(): raise RuntimeError('foreground API unavailable')
        o.foreground = unavailable
        n.Controller(o).rollback()
        self.assertTrue(o.enabled)
        self.assertEqual([v for k, v in o.events if k == 'launch'], [{'id': n.HOME, 'preload': 'full'}])

    def test_failed_write_ahead_log_prevents_the_mount_side_effect(self):
        o = FakeOps(); saved = o.save_journal
        failed = [False]
        def journal(data):
            if data.get('action') == 'bind' and not failed[0]:
                failed[0] = True
                raise OSError('disk failure before bind')
            saved(data)
        o.save_journal = journal
        with self.assertRaises(OSError): n.Controller(o).apply()
        self.assertFalse(any(k == 'bind' for k, v in o.events))
        self.assertTrue(o.enabled); self.assertTrue(o.disabled)

    def test_criu_clear_refusal_never_kills_home_or_retries_kill(self):
        o = FakeOps()
        def refuse(): raise RuntimeError('CRIU restoration active')
        o.clear_checkpoint = refuse
        with self.assertRaises(ValueError): n.Controller(o).apply()
        self.assertFalse(any(k == 'kill' for k, v in o.events))
        self.assertFalse(o.live); self.assertTrue(o.enabled)

if __name__ == '__main__': unittest.main()
