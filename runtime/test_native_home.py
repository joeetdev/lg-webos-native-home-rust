import copy
import importlib.util
import unittest

ROOT = '/var/lib/lg-native'
QML = '/usr/palm/applications/com.webos.app.home/qml/'
REL = ('UserInterfaceLayer/Containers/MainView.qml',
       'UserInterfaceLayer/Containers/Main/MainRibbon.qml',
       'UserInterfaceLayer/Containers/Main/TopShelf.qml',
       'Model/ShelfList.js',
       'Interfaces/ConnectionsInterfaces/ConnectionsInterfaces.qml',
       'Configurations/SystemServiceResponseProcessors.js')

def module():
    spec = importlib.util.find_spec('native_home')
    if spec is None:
        return None
    return __import__('native_home')

def items(count=6):
    return [dict(source=ROOT + '/candidate/file%d' % i, target=QML + rel,
                 originalHash='a' * 64, newHash='b' * 64)
            for i, rel in enumerate(REL[:count])]

class Guards(unittest.TestCase):
    def test_manifest_guard_exact_allowlist_hashes_and_staging(self):
        n = module()
        self.assertTrue(n and hasattr(n, 'validate_manifest'), 'manifest guard missing')
        self.assertEqual(n.validate_manifest(items()), items())
        for count in (3, 4, 5):
            with self.assertRaises(ValueError):
                n.validate_manifest(items(count))
            self.assertEqual(n.validate_manifest(items(count), staging=True), items(count))
        bad = []
        for target in ('/etc/passwd', '/tmp/criu_enable', QML + '../qml/' + REL[0],
                       '/mnt/lg/cmn_data/sdp/sdx/detailconfig.json'):
            x = items(); x[0]['target'] = target; bad.append(x)
        for source in (ROOT + '/../evil', ROOT + '/a/../b', ROOT + '//b', ROOT + '/a\\b',
                       ROOT + '/home-mounts.json', ROOT + '/native_home.py', ROOT + '/home-disabled',
                       ROOT + '/candidate\x00bad', ROOT + '-other/file'):
            x = items(); x[0]['source'] = source; bad.append(x)
        for h in ('', 'a' * 40, 'G' * 64, 'A' * 64, None):
            x = items(); x[0]['newHash'] = h; bad.append(x)
        x = items(); x[1]['target'] = x[0]['target']; bad.append(x)
        x = items(); x[1]['source'] = x[0]['source']; bad.append(x)
        bad += [items(2), items() + [items()[0]], {}, None]
        for case in bad:
            with self.subTest(case=case), self.assertRaises(ValueError):
                n.validate_manifest(case, staging=True)

def observed(path, digest='b' * 64, ino=12):
    return dict(path=path, realpath=path, regular=True, symlink=False,
                uid=0, gid=0, mode=0o644, nlink=1, dev=2, ino=ino, digest=digest)

class Planning(unittest.TestCase):
    def test_all_hashes_and_metadata_checked_before_plan(self):
        n = module()
        self.assertTrue(hasattr(n, 'plan_mounts'), 'pure mount planner missing')
        manifest = items()
        sources = {x['source']: observed(x['source']) for x in manifest}
        targets = {x['target']: observed(x['target'], 'a' * 64, 99) for x in manifest}
        self.assertEqual(len(n.plan_mounts(manifest, sources, targets)), 6)
        for field, value in (('digest', 'c' * 64), ('symlink', True), ('uid', 1000),
                             ('mode', 0o666), ('nlink', 2), ('regular', False),
                             ('realpath', '/etc/passwd')):
            bad = copy.deepcopy(sources); bad[manifest[-1]['source']][field] = value
            with self.subTest(field=field), self.assertRaises(ValueError):
                n.plan_mounts(manifest, bad, targets)
        bad = copy.deepcopy(targets); bad[manifest[-1]['target']]['digest'] = 'c' * 64
        with self.assertRaises(ValueError):
            n.plan_mounts(manifest, sources, bad)

    def test_owned_mount_requires_inode_hash_metadata_and_single_mount(self):
        n = module()
        self.assertTrue(hasattr(n, 'owned_mount'), 'ownership guard missing')
        entry = items()[0]
        src = observed(entry['source']); dst = observed(entry['target'])
        mounts = [dict(target=entry['target'], id=55, options=['ro'])]
        self.assertTrue(n.owned_mount(entry, src, dst, mounts, 'boot-a'))
        self.assertFalse(n.owned_mount(entry, src, dst, [], 'boot-a'))
        for change in ({'ino': 13}, {'dev': 3}, {'digest': 'c' * 64}, {'uid': 1000}):
            bad = dict(dst, **change)
            with self.subTest(change=change), self.assertRaises(ValueError):
                n.owned_mount(entry, src, bad, mounts, 'boot-a')
        for badmounts in (mounts * 2,):
            with self.assertRaises(ValueError):
                n.owned_mount(entry, src, dst, badmounts, 'boot-a')
        modern = dict(entry, mountId=55, bootId='boot-a', sourceDev=2, sourceIno=12)
        self.assertTrue(n.owned_mount(modern, src, dst, mounts, 'boot-a'))
        for bad in (dict(modern, mountId=56), dict(modern, bootId='boot-b'),
                    dict(modern, sourceIno=999)):
            with self.assertRaises(ValueError):
                n.owned_mount(bad, src, dst, mounts, 'boot-a')

    def test_foreground_guard_and_exact_process(self):
        n = module()
        self.assertTrue(hasattr(n, 'launch_payload'), 'foreground planner missing')
        for fg in ('', 'com.webos.app.home', 'com.webos.app.livetv', 'com.webos.app.lgchannels'):
            self.assertEqual(n.launch_payload(fg), {'id': 'com.webos.app.home'})
            self.assertEqual(n.launch_payload(fg, background=True)['preload'], 'full')
        for fg in ('airplay', 'com.webos.app.hdmi1', 'netflix', None, 'unknown'):
            self.assertEqual(n.launch_payload(fg)['preload'], 'full')
        self.assertEqual(n.home_pids({10: '/usr/bin/com.webos.app.home',
                                    11: '/usr/bin/com.webos.app.home.old',
                                    12: '/usr/bin/com.webos.app.home (deleted)',
                                    13: '/usr/sbin/sdx'}), [10])

class RegressionGuards(unittest.TestCase):
    def test_manifest_cannot_inject_journal_identity_or_metadata(self):
        import native_home as n
        for extra in ('mountId', 'bootId', 'sourceIno', 'sourceDev', 'inputSource', 'state', 'mode'):
            m = items(); m[0][extra] = 'injected'
            with self.subTest(extra=extra), self.assertRaises(ValueError): n.validate_manifest(m)

    def test_journal_rejects_protected_duplicate_and_traversal_targets(self):
        import native_home as n
        for mutate in (lambda m: m.append(dict(m[0])),
                       lambda m: m[0].update(target='/tmp/criu_enable'),
                       lambda m: m[0].update(source=ROOT + '/a/../../evil')):
            m = items(3); mutate(m)
            with self.assertRaises(ValueError): n.validate_journal(m)
        with self.assertRaises(ValueError):
            n.validate_journal(dict(owner=n.OWNER, version=1, mounts=[],
                                    preload=dict(original='true', pending=True, bootId='a')))

if __name__ == '__main__':
    unittest.main()
