import copy
import hashlib
import unittest
import remote_controls as r


def package(seed=b'one'):
    files = {}
    for kind, ext in [('core', 'js'), ('controller', 'qml'), ('loader', 'js')]:
        data = seed + kind.encode()
        digest = hashlib.sha256(data).hexdigest()
        files[r.ROOT + '/remote-' + kind + '-' + digest + '.' + ext] = digest
    entry = {'file': next(p for p in files if '/remote-loader-' in p),
             'handler': 'handleRemoteControls'}
    return {'owner': r.OWNER, 'version': 1, 'mode': 'final', 'files': files, 'entry': entry}


def chain():
    return [{'file': '/native.js', 'handler': h} for h in
            ['handleSafety', 'handleAccessibility', 'handleHomeGate', 'handleCPHotkeys', 'handleSystemKeys']]


class PureTests(unittest.TestCase):
    def test_insert_preserves_foreign_order_and_home(self):
        p = package(); before = chain(); frozen = copy.deepcopy(before)
        result = r.build_chain(before, p['entry'], [])
        self.assertEqual(result, before[:3] + [p['entry']] + before[3:])
        self.assertEqual(before, frozen)
        self.assertEqual(r.build_chain(result, p['entry'], [p['entry']]), result)

    def test_remove_only_exact_owned_dicts(self):
        e = package()['entry']; foreign = dict(e, extra=True)
        before = chain() + [e, foreign, e]
        self.assertEqual(r.remove_owned(before, [e]), chain() + [foreign])

    def test_refuse_ambiguous_anchor_or_unowned_remote(self):
        e = package()['entry']
        for before in [[], chain() + [chain()[3]], chain() + [e]]:
            with self.assertRaises(r.Error):
                r.build_chain(before, e, [])

    def test_package_rejects_trial_paths_count_hash_and_entry(self):
        r.validate_package(package())
        for mutate in [lambda p: p.update(mode='trial'), lambda p: p.update(version=True),
                       lambda p: p['files'].pop(next(iter(p['files']))),
                       lambda p: p['entry'].update(handler='other'),
                       lambda p: p['files'].update({'/tmp/remote-core-x.js': 'a'*64}),
                       lambda p: p['files'].update({next(iter(p['files'])): '0'*64})]:
            p = package(); mutate(p)
            with self.assertRaises(r.Error): r.validate_package(p)


class FakeIO:
    def __init__(self):
        self.live = chain(); self.state = None; self.pkg = package()
        self.events = []; self.fail = None; self.bad_readback = False

    def read_json(self, path, optional=False):
        return copy.deepcopy(self.state if path == r.STATE else self.pkg)

    def read_private(self, path):
        kind = r.FILE_RE.fullmatch(path).group(1)
        return b'one' + kind.encode()

    def atomic_json(self, path, value):
        self.events.append('state')
        if self.fail == 'state': raise OSError('injected state failure')
        self.state = copy.deepcopy(value)

    def get_filters(self):
        return copy.deepcopy(self.live)

    def set_filters(self, value):
        self.events.append('set')
        if self.fail == 'set': raise OSError('injected config failure')
        self.live = copy.deepcopy(value)
        if self.bad_readback: self.live.append({'handler': 'foreignConcurrent'})


class TransactionTests(unittest.TestCase):
    def test_install_durable_ownership_before_set_and_repeat_no_reload(self):
        io = FakeIO(); r.install(io, '/package.json')
        self.assertEqual(io.events, ['state', 'set'])
        self.assertEqual(io.state['owned'], [io.pkg['entry']])
        self.assertEqual(io.live, chain()[:3] + [io.pkg['entry']] + chain()[3:])
        r.install(io, '/package.json')
        self.assertEqual(io.events, ['state', 'set'])
        r.check(io)

    def test_state_failure_prevents_config_write(self):
        io = FakeIO(); io.fail = 'state'
        with self.assertRaises(OSError): r.install(io, '/package.json')
        self.assertEqual(io.live, chain()); self.assertNotIn('set', io.events)

    def test_config_failure_retains_recoverable_ownership(self):
        io = FakeIO(); io.fail = 'set'
        with self.assertRaises(OSError): r.install(io, '/package.json')
        self.assertEqual(io.state['owned'], [io.pkg['entry']])
        io.fail = None; r.disable(io)
        self.assertEqual(io.live, chain())

    def test_bad_readback_is_failure_not_blanket_restore(self):
        io = FakeIO(); io.bad_readback = True
        with self.assertRaises(r.Error): r.install(io, '/package.json')
        self.assertIsNotNone(io.state)
        self.assertEqual(io.live[-1], {'handler': 'foreignConcurrent'})

    def test_swap_journals_both_entries_and_disable_preserves_live_foreign(self):
        io = FakeIO(); r.install(io, '/package.json'); old = io.pkg['entry']
        io.pkg = package(b'two')
        io.read_private = lambda path: b'two' + r.FILE_RE.fullmatch(path).group(1).encode()
        r.install(io, '/package.json')
        self.assertEqual(io.state['owned'], [old, io.pkg['entry']])
        extra = dict(old, foreign=True); io.live.extend([extra, {'handler': 'newForeign'}])
        r.disable(io)
        self.assertEqual(io.live, chain() + [extra, {'handler': 'newForeign'}])
        count = io.events.count('set'); r.disable(io)
        self.assertEqual(io.events.count('set'), count)

    def test_disable_without_state_cannot_claim_ownership(self):
        io = FakeIO(); io.live.append(io.pkg['entry']); before = copy.deepcopy(io.live)
        r.disable(io); self.assertEqual(io.live, before); self.assertEqual(io.events, [])

    def test_hash_failure_prevents_journal_and_config(self):
        io = FakeIO(); io.read_private = lambda path: b'tampered'
        with self.assertRaises(r.Error): r.install(io, '/package.json')
        self.assertEqual(io.events, [])

    def test_check_rejects_missing_duplicate_other_remote_and_corruption(self):
        io = FakeIO(); r.install(io, '/package.json')
        good = copy.deepcopy(io.live)
        for bad in [chain(), good + [io.pkg['entry']], good + [package(b'two')['entry']]]:
            io.live = bad
            with self.assertRaises(r.Error): r.check(io)
        io.live = good; io.read_private = lambda path: b'corrupt'
        with self.assertRaises(r.Error): r.check(io)


class LinuxBoundaryTests(unittest.TestCase):
    def test_metadata_rejects_symlink_nonroot_open_permissions_and_large(self):
        from types import SimpleNamespace
        import stat
        def meta(mode=stat.S_IFREG | 0o600, uid=0, size=10):
            return SimpleNamespace(st_mode=mode, st_uid=uid, st_size=size)
        r.validate_metadata(meta())
        for st in [meta(stat.S_IFLNK | 0o600), meta(uid=1), meta(stat.S_IFREG | 0o644),
                   meta(size=r.LIMIT + 1), meta(stat.S_IFDIR | 0o700)]:
            with self.assertRaises(r.Error): r.validate_metadata(st)
        r.validate_metadata(meta(stat.S_IFDIR | 0o755), directory=True)
        with self.assertRaises(r.Error):
            r.validate_metadata(meta(stat.S_IFDIR | 0o777), directory=True)

    def test_luna_uses_devnull_timeout_exact_schema_and_rejects_failure(self):
        import json
        import subprocess
        from unittest.mock import patch
        from types import SimpleNamespace
        io = object.__new__(r.LinuxIO)
        responses = [SimpleNamespace(returncode=0, stdout=json.dumps({'returnValue': True,
                     'configs': {r.KEY: chain()}}).encode()),
                     SimpleNamespace(returncode=0, stdout=b'{"returnValue":true}'),
                     SimpleNamespace(returncode=0, stdout=b'{"returnValue":false}')]
        with patch('subprocess.run', side_effect=responses) as run:
            self.assertEqual(io.get_filters(), chain()); io.set_filters(chain())
            with self.assertRaises(r.Error): io.get_filters()
            for call in run.call_args_list:
                self.assertEqual(call.kwargs['stdin'], subprocess.DEVNULL)
                self.assertEqual(call.kwargs['timeout'], 10)
                self.assertEqual(call.args[0][:3], ['luna-send', '-n', '1'])
            argv = run.call_args_list[1].args[0]
            self.assertEqual(json.loads(argv[-1]), {'configs': {r.KEY: chain()}})
            self.assertEqual(argv[-2], 'luna://com.webos.service.config/setConfigs')

    def test_timeout_is_safe_cli_failure_without_dump(self):
        import contextlib
        import io as streams
        import subprocess
        io = FakeIO()
        io.get_filters = lambda: (_ for _ in ()).throw(subprocess.TimeoutExpired('secret', 10))
        out = streams.StringIO()
        with contextlib.redirect_stdout(out):
            self.assertEqual(r.main(['--install', '/package.json'], io=io), 1)
        self.assertNotIn('secret', out.getvalue())
        self.assertNotIn('set', io.events)

    def test_atomic_state_orders_fsync_replace_directory_fsync_and_cleans_failure(self):
        import contextlib
        import io as streams
        from unittest.mock import patch
        class Stream(streams.BytesIO):
            def fileno(self): return 9
            def close(self): pass
        io = object.__new__(r.LinuxIO)
        io.parents = lambda path: None
        io.read_private = lambda path: b'old'
        events = []
        with contextlib.ExitStack() as stack:
            stack.enter_context(patch('tempfile.mkstemp', return_value=(9, '/scratch-owned')))
            stack.enter_context(patch('os.fdopen', return_value=Stream()))
            chmod = stack.enter_context(patch('os.fchmod', create=True))
            sync = stack.enter_context(patch('os.fsync', side_effect=lambda fd: events.append(('fsync', fd))))
            replace = stack.enter_context(patch('os.replace', side_effect=lambda a, b: events.append(('replace', b))))
            stack.enter_context(patch('os.O_DIRECTORY', 0, create=True))
            stack.enter_context(patch('os.O_NOFOLLOW', 0, create=True))
            stack.enter_context(patch('os.open', return_value=10))
            stack.enter_context(patch('os.close'))
            stack.enter_context(patch('os.path.exists', return_value=True))
            unlink = stack.enter_context(patch('os.unlink'))
            io.atomic_json(r.STATE, {'owned': []})
            self.assertEqual(events, [('fsync', 9), ('replace', r.STATE), ('fsync', 10)])
            chmod.assert_called_once_with(9, 0o600)
            replace.reset_mock(); sync.side_effect = OSError('injected disk failure')
            with self.assertRaises(OSError): io.atomic_json(r.STATE, {'owned': []})
            replace.assert_not_called(); self.assertEqual(unlink.call_count, 2)

    def test_read_private_rejects_symlink_before_open(self):
        import stat
        from unittest.mock import patch
        from types import SimpleNamespace
        io = object.__new__(r.LinuxIO); io.parents = lambda path: None
        with patch('os.lstat', return_value=SimpleNamespace(st_uid=0, st_mode=stat.S_IFLNK | 0o600,
                                                          st_size=10)), patch('os.open') as opened:
            with self.assertRaises(r.Error): io.read_private('/owned')
            opened.assert_not_called()

    def test_cli_dispatches_without_device_and_suppresses_exception_detail(self):
        import contextlib
        import io as streams
        io = FakeIO(); out = streams.StringIO()
        with contextlib.redirect_stdout(out):
            self.assertEqual(r.main(['--install', '/package.json'], io=io), 0)
            self.assertEqual(r.main(['--check'], io=io), 0)
            self.assertEqual(r.main(['--disable'], io=io), 0)
            self.assertEqual(r.main(['--check'], io=io), 1)
        self.assertNotIn('Traceback', out.getvalue())


if __name__ == '__main__':
    unittest.main()
