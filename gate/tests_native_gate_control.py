"""Offline safety tests; no TV access or deploy commands."""
import copy
import hashlib
import importlib.util
import io
import json
import stat
import sys
from types import SimpleNamespace
from unittest.mock import patch
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent


def module():
    path = HERE / 'native_gate_control.py'
    if not path.exists():
        return None
    spec = importlib.util.spec_from_file_location('gate_control_under_test', path)
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result


def state(payload=b'generated loader bytes'):
    digest = hashlib.sha256(payload).hexdigest()
    return dict(version=1, owner='lg-native-home-gate-v1', entry={
        'file': '/var/lib/lg-native/native-home-gate-loader-' + digest + '.js',
        'handler': 'handleNativeHomeGate'}, originalKeyFilters17=[{'handler': 'old'}],
        files={'/var/lib/lg-native/native-home-gate-controller.qml': 'a' * 64},
        parentExtension={'preserve': True})


class PureOwnership(unittest.TestCase):
    def test_exact_owned_entry_only_is_removed_without_mutating_inputs(self):
        g = module()
        self.assertTrue(g and hasattr(g, 'remove_owned_filter'), 'pure owned-filter removal missing')
        s = state()
        foreign = [dict(s['entry'], file=state(b'other loader')['entry']['file']),
                   dict(s['entry'], handler='handleSomeoneElse'),
                   dict(s['entry'], extra=True), {'handler': 'handleSystemKeys'}, None]
        current = [foreign[0], s['entry'], *foreign[1:], copy.deepcopy(s['entry'])]
        before = copy.deepcopy((current, s))
        self.assertEqual(g.remove_owned_filter(current, s), foreign)
        self.assertEqual((current, s), before)
        self.assertEqual(g.remove_owned_filter(foreign, s), foreign)
        self.assertIs(g.validate_state(s), s)

    def test_state_refuses_noncanonical_or_foreign_ownership_and_bad_file_map(self):
        g = module()
        s = state()
        bad = [None, {}, dict(s, owner='foreign'), dict(s, version=True), dict(s, version=2)]
        for path in ('/var/lib/lg-native/native-home-gate-loader.js',
                     s['entry']['file'].replace('loader-', 'loader-../'),
                     s['entry']['file'].replace('/lg-native/', '/lg-native//'),
                     s['entry']['file'].replace('lg-native/', 'lg-native/../lg-native/'),
                     s['entry']['file'].replace('.js', '.js\n'),
                     s['entry']['file'].replace('loader-', 'loader-' + 'A'),
                     s['entry']['file'].replace('/var/', '/tmp/'), None):
            bad.append(dict(s, entry=dict(s['entry'], file=path)))
        bad += [dict(s, entry=dict(s['entry'], handler='handleSystemKeys')),
                dict(s, entry=dict(s['entry'], extra=True))]
        for files in ([], {'relative': 'a' * 64}, {'/var/lib/../evil': 'a' * 64},
                      {'/tmp/a\\b': 'a' * 64}, {'/tmp/a': 'A' * 64}, {'/tmp/a': False}):
            bad.append(dict(s, files=files))
        for case in bad:
            with self.subTest(case=case), self.assertRaises(ValueError):
                g.validate_state(case)
        with self.assertRaises(ValueError):
            g.remove_owned_filter({}, s)

    def test_atomic_transition_removes_every_recorded_generation_preserving_foreign_entries(self):
        g = module()
        old = state(b'trial generation')['entry']; new = state(b'final generation')['entry']
        s = state(b'final generation'); s['ownedEntries'] = [old, new]
        foreign = [state(b'foreign generation')['entry'], dict(old, extra='foreign'),
                   {'file': old['file'], 'handler': 'handleSystemKeys'}]
        self.assertIs(g.validate_state(s), s)
        # State is durably written BEFORE swapping the actual LS2 entry.
        for installed in ([old], [new], [old, new], []):
            with self.subTest(installed=installed):
                service = ConfigService([foreign[0], *installed, *foreign[1:]])
                snapshot = copy.deepcopy(s)
                result = g.disable_gate(s, service)
                self.assertEqual(service.filters, foreign)
                self.assertTrue(result['disabled'])
                self.assertEqual(result['changed'], bool(installed))
                self.assertEqual(s, snapshot)
        for entries in (None, {}, [], [old], [new, new],
                        [new, dict(old, handler='foreign')],
                        [new, dict(old, file=old['file'].replace('loader-', 'loader-../'))],
                        [new] + [state(str(i).encode())['entry'] for i in range(4)]):
            with self.subTest(entries=entries), self.assertRaises(ValueError):
                g.validate_state(dict(s, ownedEntries=entries))
        max_owned = [new] + [state(str(i).encode())['entry'] for i in range(3)]
        self.assertIs(g.validate_state(dict(s, ownedEntries=max_owned))['ownedEntries'], max_owned)


class ConfigService:
    """Fake LS2 boundary with real config state and exact payload capture."""
    def __init__(self, filters):
        self.filters = copy.deepcopy(filters)
        self.calls = []
        self.bad_readback = False
        self.fail = None

    def __call__(self, method, payload):
        self.calls.append((method, copy.deepcopy(payload)))
        if self.fail == method:
            return {'returnValue': False, 'errorText': 'injected LS2 error'}
        if method == 'com.webos.service.config/getConfigs':
            assert payload == {'configNames': ['com.webos.surfacemanager.keyFilters']}
            return {'returnValue': True, 'configs': {
                'com.webos.surfacemanager.keyFilters': copy.deepcopy(self.filters)}}
        assert method == 'com.webos.service.config/setConfigs'
        assert set(payload) == {'configs'}
        assert set(payload['configs']) == {'com.webos.surfacemanager.keyFilters'}
        if not self.bad_readback:
            self.filters = copy.deepcopy(payload['configs']['com.webos.surfacemanager.keyFilters'])
        return {'returnValue': True}


class Disable(unittest.TestCase):
    def test_disable_preserves_live_foreign_entries_verifies_write_and_is_idempotent(self):
        g = module()
        self.assertTrue(hasattr(g, 'disable_gate'), 'verified LS2 gate removal missing')
        s = state()
        foreign = [dict(file='/firmware/native.js', handler='handleSystemKeys'),
                   state(b'another generation')['entry'], dict(s['entry'], extra='foreign')]
        service = ConfigService([foreign[0], s['entry'], *foreign[1:]])
        self.assertEqual(g.disable_gate(s, service), {'disabled': True, 'changed': True})
        self.assertEqual(service.filters, foreign)
        self.assertEqual([c[0].rsplit('/', 1)[-1] for c in service.calls],
                         ['getConfigs', 'setConfigs', 'getConfigs'])
        service.calls.clear()
        self.assertEqual(g.disable_gate(s, service), {'disabled': True, 'changed': False})
        self.assertEqual(len(service.calls), 1)
        service.calls.clear()
        self.assertEqual(g.disable_gate(None, service), {'disabled': False, 'changed': False})
        self.assertEqual(service.calls, [])
        service = ConfigService([s['entry']]); service.bad_readback = True
        with self.assertRaises(ValueError):
            g.disable_gate(s, service)
        for method in ('getConfigs', 'setConfigs'):
            service = ConfigService([s['entry']]); service.fail = 'com.webos.service.config/' + method
            with self.subTest(method=method), self.assertRaises(ValueError):
                g.disable_gate(s, service)
        for response in ({}, {'returnValue': True}, {'returnValue': True, 'configs': {g.KEY: {}}}):
            with self.subTest(response=response), self.assertRaises(ValueError):
                g.disable_gate(s, lambda method, payload: response)

    def test_readback_detects_type_or_order_drift_in_foreign_filters(self):
        g = module(); s = state()
        before = [s['entry'], {'foreignVersion': 0}, {'handler': 'native'}]
        responses = [dict(returnValue=True, configs={g.KEY: before}), dict(returnValue=True),
                     dict(returnValue=True, configs={g.KEY: [{'foreignVersion': False}, {'handler': 'native'}]})]
        with self.assertRaisesRegex(ValueError, 'readback'):
            g.disable_gate(s, lambda method, payload: responses.pop(0))
        responses = [dict(returnValue=True, configs={g.KEY: before}), dict(returnValue=True),
                     dict(returnValue=True, configs={g.KEY: [{'handler': 'native'}, {'foreignVersion': 0}]})]
        with self.assertRaisesRegex(ValueError, 'readback'):
            g.disable_gate(s, lambda method, payload: responses.pop(0))


def metadata(mode=stat.S_IFREG | 0o600, **updates):
    fields = dict(st_mode=mode, st_uid=0, st_nlink=1, st_dev=3, st_ino=5, st_size=100)
    fields.update(updates)
    return SimpleNamespace(**fields)


class TrustedState(unittest.TestCase):
    def test_state_is_read_only_after_root_parent_file_and_fd_identity_validation(self):
        g = module()
        self.assertTrue(hasattr(g, 'load_state'), 'standalone trusted state loader missing')
        path = g.ROOT + '/' + g.STATE_NAME
        encoded = json.dumps(state()).encode()
        file_meta = metadata(st_size=len(encoded))
        parent_meta = metadata(stat.S_IFDIR | 0o700)
        def read_with(raw=encoded, file=file_meta, fd=file_meta, parent=parent_meta, missing=False):
            def lstat(p):
                if p == path:
                    if missing: raise FileNotFoundError(p)
                    return file
                return parent
            with patch.object(g.os, 'lstat', side_effect=lstat), \
                 patch.object(g.os, 'fstat', return_value=fd), \
                 patch.object(g.os, 'open', return_value=17) as opened, \
                 patch.object(g.os, 'close'), \
                 patch.object(g.os, 'fdopen', return_value=io.BytesIO(raw)), \
                 patch.object(g.os, 'O_NONBLOCK', 0x800, create=True), \
                 patch.object(g.os, 'O_NOFOLLOW', 0x20000, create=True):
                result = g.load_state(g.ROOT)
                if not missing:
                    self.assertEqual(opened.call_args.args[0], path)
                    self.assertTrue(opened.call_args.args[1] & g.os.O_NOFOLLOW)
                else:
                    opened.assert_not_called()
                return result
        self.assertEqual(read_with(), state())
        self.assertIsNone(read_with(missing=True))
        for change in (dict(st_uid=1000), dict(st_nlink=2), dict(st_mode=stat.S_IFLNK | 0o600),
                       dict(st_mode=stat.S_IFREG | 0o622), dict(st_mode=stat.S_IFDIR | 0o700),
                       dict(st_size=1048577)):
            bad = metadata(**dict(file_meta.__dict__, **change))
            with self.subTest(change=change), self.assertRaises(ValueError):
                read_with(file=bad)
        with self.assertRaises(ValueError):
            read_with(fd=metadata(st_ino=999))
        for parent in (metadata(stat.S_IFDIR | 0o777), metadata(stat.S_IFLNK | 0o700),
                       metadata(stat.S_IFDIR | 0o700, st_uid=1000)):
            with self.subTest(parent=parent), self.assertRaises(ValueError):
                read_with(parent=parent)
        for raw in (b'{"owner":"foreign","owner":"lg-native-home-gate-v1"}', b'{}',
                    b'not JSON', b'x' * 1048577):
            with self.subTest(raw=raw[:60]), self.assertRaises(ValueError):
                read_with(raw=raw)

    def test_missing_private_root_is_also_no_state_but_untrusted_root_is_not_absence(self):
        g = module()
        parent = metadata(stat.S_IFDIR | 0o755)
        def lstat(path):
            if path == g.ROOT:
                raise FileNotFoundError(path)
            return parent
        with patch.object(g.os, 'lstat', side_effect=lstat), \
             patch.object(g.os, 'open', side_effect=AssertionError('must not open missing state')):
            self.assertIsNone(g.load_state())
        def untrusted(path):
            if path == g.ROOT:
                return metadata(stat.S_IFLNK | 0o700)
            return parent
        with patch.object(g.os, 'lstat', side_effect=untrusted):
            with self.assertRaises(ValueError):
                g.load_state()


class Recovery(unittest.TestCase):
    def test_recovery_keeps_ready_gate_but_checker_errors_still_remove_only_gate(self):
        g = module()
        self.assertTrue(hasattr(g, 'recover_if_unready'), 'fail-safe conditional recovery missing')
        s = state(); foreign = {'file': '/native.js', 'handler': 'handleSystemKeys'}
        service = ConfigService([s['entry'], foreign])
        result = g.recover_if_unready(s, service, checker=lambda: True)
        self.assertTrue(result['ready']); self.assertFalse(result['disabled'])
        self.assertEqual(service.calls, [])
        def broken():
            raise ImportError('native Home runtime cannot be imported')
        for checker in (lambda: False, broken, lambda: None, lambda: 'unproven'):
            service = ConfigService([s['entry'], foreign])
            result = g.recover_if_unready(s, service, checker=checker)
            self.assertFalse(result['ready']); self.assertTrue(result['disabled'])
            self.assertEqual(service.filters, [foreign])
        service = ConfigService([s['entry']]); service.bad_readback = True
        with self.assertRaises(ValueError):
            g.recover_if_unready(s, service, checker=broken)
        service = ConfigService([])
        result = g.recover_if_unready(None, service, checker=broken)
        self.assertFalse(result['ready']); self.assertFalse(result['disabled'])
        self.assertEqual(service.calls, [])


def load_local(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result


def applied_eight():
    """Produce journal/mounts with the ACTUAL existing apply controller."""
    runtime = HERE.parent / 'runtime'
    n = load_local('native_home_fixture', runtime / 'native_home.py')
    base = load_local('base_fixture', runtime / 'test_native_home.py')
    with patch.dict(sys.modules, {'native_home': n, 'test_native_home': base}):
        boundary = load_local('ops_fixture', runtime / 'test_runtime.py')
    ops = boundary.FakeOps()
    ops.manifest = [dict(source=n.ROOT + '/candidate/file%d' % i, target=target,
                         originalHash='a' * 64, newHash=hashlib.sha256(target.encode()).hexdigest())
                    for i, target in enumerate(sorted(n.ALLOWED))]
    ops.files = {e['source']: dict(base.observed(e['source'], e['newHash'], i + 20), mode=493)
                 for i, e in enumerate(ops.manifest)}
    ops.stock = {e['target']: dict(base.observed(e['target'], 'a' * 64, i + 100), mode=493)
                 for i, e in enumerate(ops.manifest)}
    ops.files.update(copy.deepcopy(ops.stock))
    n.Controller(ops).apply()
    ops.events.clear()
    return n, ops


class ReadOnlyBoundary:
    """Expose only reads, so unexpected methods fail before any side effect."""
    def __init__(self, inner):
        self.inner = inner

    def __getattr__(self, key):
        if key not in {'boot_id', 'is_disabled', 'load_manifest', 'load_journal', 'mounts', 'inspect'}:
            raise AssertionError('Health checker attempted non-read operation: ' + key)
        return getattr(self.inner, key)


class NativeReadiness(unittest.TestCase):
    def test_real_controller_generated_eight_readonly_owned_overlays_are_ready_without_writes(self):
        g = module()
        self.assertTrue(hasattr(g, 'native_ready'), 'read-only native health checker missing')
        n, ops = applied_eight()
        before = copy.deepcopy((ops.files, ops.live, ops.journal, ops.enabled, ops.disabled))
        self.assertIs(g.native_ready(ops=ReadOnlyBoundary(ops), native=n), True)
        self.assertEqual(before, (ops.files, ops.live, ops.journal, ops.enabled, ops.disabled))
        self.assertTrue(all(event[0] == 'inspect' for event in ops.events))

    def test_unhealthy_or_incoherent_native_proofs_release_gate_without_touching_home(self):
        g = module()
        mutations = {
            'disabled': lambda o: setattr(o, 'disabled', True),
            'six manifest': lambda o: setattr(o, 'manifest', o.manifest[:6]),
            'no journal': lambda o: setattr(o, 'journal', None),
            'legacy journal': lambda o: setattr(o, 'journal', o.journal['mounts']),
            'seven journal': lambda o: o.journal['mounts'].pop(),
            'no preload': lambda o: o.journal.pop('preload'),
            'pending preload': lambda o: o.journal['preload'].update(pending=True),
            'old preload boot': lambda o: o.journal['preload'].update(bootId='old'),
            'wrong journal owner': lambda o: o.journal.update(owner='foreign'),
            'old mount boot': lambda o: o.journal['mounts'][0].update(bootId='old'),
            'missing mount boot': lambda o: o.journal['mounts'][0].pop('bootId'),
            'missing mount id': lambda o: o.journal['mounts'][0].pop('mountId'),
            'missing source inode': lambda o: o.journal['mounts'][0].pop('sourceIno'),
            'missing source device': lambda o: o.journal['mounts'][0].pop('sourceDev'),
            'wrong mount id': lambda o: o.journal['mounts'][0].update(mountId=999),
            'not mounted state': lambda o: o.journal['mounts'][0].update(state='bind'),
            'not readonly': lambda o: o.live[0].update(options=['rw']),
            'stacked bind': lambda o: o.live.append(copy.deepcopy(o.live[0])),
            'missing bind': lambda o: o.live.pop(),
            'foreign inode': lambda o: o.files[o.journal['mounts'][0]['target']].update(ino=99999),
            'bad target digest': lambda o: o.files[o.journal['mounts'][0]['target']].update(digest='c' * 64),
            'bad copy digest': lambda o: o.files[o.journal['mounts'][0]['source']].update(digest='c' * 64),
            'bad input digest': lambda o: o.files[o.manifest[0]['source']].update(digest='c' * 64),
            'source symlink': lambda o: o.files[o.journal['mounts'][0]['source']].update(symlink=True),
            'source nonroot': lambda o: o.files[o.journal['mounts'][0]['source']].update(uid=1000),
            'target writable': lambda o: o.files[o.journal['mounts'][0]['target']].update(mode=0o777),
            'journal metadata drift': lambda o: o.journal['mounts'][0].update(mode=0o644),
            'manifest newHash drift': lambda o: o.manifest[0].update(newHash='f' * 64),
            'manifest originalHash drift': lambda o: o.manifest[0].update(originalHash='f' * 64),
            'manifest inputSource drift': lambda o: o.manifest[0].update(source=o.manifest[0]['source'] + '-other'),
        }
        s = state(); s.pop('files')
        for label, mutate in mutations.items():
            with self.subTest(label=label):
                n, ops = applied_eight(); mutate(ops)
                before = copy.deepcopy((ops.files, ops.live, ops.journal, ops.enabled, ops.disabled))
                service = ConfigService([{'handler': 'native'}, s['entry']])
                result = g.recover_if_unready(s, service,
                    checker=lambda: g.native_ready(ops=ReadOnlyBoundary(ops), native=n))
                self.assertFalse(result['ready'], label)
                self.assertTrue(result['disabled'])
                self.assertEqual(service.filters, [{'handler': 'native'}])
                self.assertEqual(before, (ops.files, ops.live, ops.journal, ops.enabled, ops.disabled))
                self.assertTrue(all(event[0] == 'inspect' for event in ops.events))


class NativeLoader(unittest.TestCase):
    def test_default_checker_uses_trusted_fixed_native_module_not_cwd_or_sys_modules(self):
        g = module()
        self.assertTrue(hasattr(g, 'load_native'), 'trusted lazy native runtime loader missing')
        runtime_bytes = (HERE.parent / 'runtime' / 'native_home.py').read_bytes()
        with patch.object(g, 'trusted_bytes', return_value=runtime_bytes) as reader, \
             patch.dict(sys.modules, {'native_home': SimpleNamespace(Controller='foreign')}):
            loaded = g.load_native()
            self.assertTrue(callable(loaded.Controller))
            self.assertEqual(loaded.__file__, g.ROOT + '/native_home.py')
            reader.assert_called_once_with(g.ROOT + '/native_home.py')
        n, ops = applied_eight()
        with patch.object(g, 'load_native', return_value=n) as loader, \
             patch.object(n, 'LinuxOps', return_value=ReadOnlyBoundary(ops)) as factory:
            service = ConfigService([state()['entry']])
            result = g.recover_if_unready(state(), service)
            self.assertTrue(result['ready']); self.assertEqual(service.calls, [])
            loader.assert_called_once(); factory.assert_called_once_with()
        with patch.object(g, 'load_native', side_effect=SyntaxError('broken runtime')):
            service = ConfigService([state()['entry']])
            result = g.recover_if_unready(state(), service)
            self.assertFalse(result['ready']); self.assertTrue(result['disabled'])
            self.assertEqual(service.filters, [])


class DeviceCall(unittest.TestCase):
    def test_luna_process_is_bounded_noninteractive_checks_transport_and_return_value(self):
        g = module()
        self.assertTrue(hasattr(g, 'luna_call'), 'standalone bounded Luna adapter missing')
        method = 'com.webos.service.config/getConfigs'
        payload = {'configNames': [g.KEY]}
        with patch.object(g.subprocess, 'run') as run:
            run.return_value = SimpleNamespace(returncode=0, stdout='{"returnValue":true}', stderr='')
            self.assertEqual(g.luna_call(method, payload), {'returnValue': True})
            args, kwargs = run.call_args
            self.assertEqual(args[0][:4], ['luna-send', '-n', '1', 'luna://' + method])
            self.assertEqual(json.loads(args[0][4]), payload)
            self.assertEqual(kwargs['stdin'], g.subprocess.DEVNULL)
            self.assertEqual(kwargs['timeout'], 10)
            self.assertFalse(kwargs.get('shell', False))
            self.assertNotIn('.', kwargs['env']['PATH'].split(':'))
            for result in (SimpleNamespace(returncode=1, stdout='{"returnValue":true}', stderr='failed'),
                           SimpleNamespace(returncode=0, stdout='{"returnValue":false}', stderr=''),
                           SimpleNamespace(returncode=0, stdout='not JSON', stderr=''),
                           SimpleNamespace(returncode=0, stdout='[]', stderr='')):
                run.return_value = result
                with self.subTest(result=result), self.assertRaises(ValueError):
                    g.luna_call(method, payload)
            run.side_effect = g.subprocess.TimeoutExpired('luna-send', 10)
            with self.assertRaises(g.subprocess.TimeoutExpired):
                g.luna_call(method, payload)
        with patch.object(g.subprocess, 'run') as run:
            with self.assertRaises(ValueError):
                g.luna_call('com.webos.applicationManager/launch', {})
            run.assert_not_called()


class CommandLine(unittest.TestCase):
    def test_root_cli_disable_never_imports_home_and_recovery_errors_do_not_block_removal(self):
        g = module()
        self.assertTrue(hasattr(g, 'cli'), 'JSON safety CLI missing')
        s = state()
        def run_cli(argv, saved=s, checker=None, service=None, platform='linux', uid=0):
            service = service or ConfigService([s['entry'], {'handler': 'native'}])
            output = io.StringIO()
            with patch.object(g.sys, 'platform', platform), \
                 patch.object(g.os, 'geteuid', return_value=uid, create=True), \
                 patch.object(g, 'load_state', return_value=saved), \
                 patch.object(g, 'load_native', side_effect=ImportError('must not import Home')) as loader, \
                 patch.object(g.sys, 'stdout', output):
                code = g.cli(argv, call=service, checker=checker)
                if argv == ['--disable'] or saved is None:
                    loader.assert_not_called()
            return code, json.loads(output.getvalue()), service
        code, result, service = run_cli(['--disable'])
        self.assertEqual(code, 0); self.assertTrue(result['disabled'])
        self.assertEqual(service.filters, [{'handler': 'native'}])
        code, result, service = run_cli(['--recover-if-unready'])
        self.assertEqual(code, 0); self.assertFalse(result['ready']); self.assertTrue(result['disabled'])
        code, result, service = run_cli(['--recover-if-unready'], checker=lambda: True)
        self.assertEqual(code, 0); self.assertTrue(result['ready']); self.assertFalse(result['disabled'])
        self.assertEqual(service.calls, [])
        for argv in (['--disable'], ['--recover-if-unready']):
            code, result, service = run_cli(argv, saved=None)
            self.assertEqual(code, 0); self.assertFalse(result['disabled'])
            self.assertEqual(service.calls, [])
        for platform, uid in (('win32', 0), ('linux', 1000)):
            code, result, service = run_cli(['--disable'], platform=platform, uid=uid)
            self.assertEqual(code, 1); self.assertIn('error', result)
            self.assertEqual(service.calls, [])
        service = ConfigService([s['entry']]); service.bad_readback = True
        code, result, _ = run_cli(['--disable'], service=service)
        self.assertEqual(code, 1); self.assertIn('readback', result['error'])


if __name__ == '__main__':
    unittest.main()
