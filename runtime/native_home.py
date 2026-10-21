"""Fail-closed, root-only native Home override candidate (Python >= 3.8)."""
import posixpath
import re
import hashlib
import json
import os
import signal
import stat
import subprocess
import sys
import time

ROOT = '/var/lib/lg-native'
HOME = 'com.webos.app.home'
EXE = '/usr/bin/' + HOME
QML = '/usr/palm/applications/' + HOME + '/qml/'
ALLOWED = frozenset(QML + p for p in (
    'UserInterfaceLayer/Containers/MainView.qml',
    'UserInterfaceLayer/Containers/Main/MainRibbon.qml',
    'UserInterfaceLayer/Containers/Main/TopShelf.qml',
    'UserInterfaceLayer/Containers/Main/HomeDashboard.qml',
    'UserInterfaceLayer/Containers/Sub/MobileComponent.qml',
    'Model/ShelfList.js',
    'Interfaces/ConnectionsInterfaces/ConnectionsInterfaces.qml',
    'Configurations/SystemServiceResponseProcessors.js'))
RESERVED = frozenset(('home-mounts.json', 'home-final-manifest.json', 'home-disabled',
    'home-boot.json', 'home.lock', 'home-preload-trial.json', 'checkpoint-backup.json',
    'native_home.py', 'nativeapply.py', 'rollback.py', 'clear-home-checkpoint.py',
    '55-lg-native', 'home-preload-runtime.json', 'home-boot.log'))


def require(ok, message):
    if not ok:
        raise ValueError(message)


def source_path(path):
    require(isinstance(path, str) and path.startswith(ROOT + '/'), 'Source outside owned root')
    require(posixpath.normpath(path) == path and '\\' not in path and
            all(ord(c) >= 33 and ord(c) < 127 for c in path), 'Noncanonical source')
    require(not set(path[len(ROOT) + 1:].split('/')) & RESERVED, 'Reserved runtime path')
    return path


def validate_entries(entries, minimum=0):
    require(isinstance(entries, list) and minimum <= len(entries) <= len(ALLOWED), 'Invalid entry count')
    sources, targets = set(), set()
    for item in entries:
        require(isinstance(item, dict), 'Entry must be an object')
        target, source = item.get('target'), item.get('source')
        require(isinstance(target, str) and target in ALLOWED, 'Target is not an allowed Home file')
        source_path(source)
        require(target not in targets and source not in sources, 'Duplicate source/target')
        sources.add(source); targets.add(target)
        for key in ('originalHash', 'newHash'):
            require(isinstance(item.get(key), str) and re.fullmatch('[a-f0-9]{64}', item[key]),
                    'Expected lowercase SHA256: ' + key)
    return entries


def validate_manifest(entries, staging=False):
    validate_entries(entries, minimum=3 if staging else 6)
    for entry in entries:
        require(set(entry) == {'source', 'target', 'originalHash', 'newHash'},
                'Manifest must contain only source, target, originalHash, newHash')
    return entries


def validate_file(info, path, digest=None):
    require(info['path'] == path and info['realpath'] == path and
            info['regular'] and not info['symlink'], 'Unsafe file: ' + path)
    require(info['uid'] == 0 and not info['mode'] & 0o7022 and info['nlink'] == 1,
            'Unsafe ownership/mode/link count: ' + path)
    if digest is not None:
        require(info['digest'] == digest, 'Hash mismatch: ' + path)
    return info


def plan_mounts(entries, sources, targets):
    validate_entries(entries, minimum=3)
    result = []
    for entry in entries:
        validate_file(sources[entry['source']], entry['source'], entry['newHash'])
        meta = validate_file(targets[entry['target']], entry['target'], entry['originalHash'])
        result.append(dict(entry, uid=meta['uid'], gid=meta['gid'], mode=meta['mode']))
    return result


def owned_mount(entry, source, target, mounts, boot_id):
    validate_entries([entry])
    exact = [m for m in mounts if m['target'] == entry['target']]
    if not exact:
        return False
    require(len(exact) == 1, 'Stacked mount refused: ' + entry['target'])
    validate_file(source, entry['source'], entry['newHash'])
    validate_file(target, entry['target'], entry['newHash'])
    require((source['dev'], source['ino']) == (target['dev'], target['ino']),
            'Mount is not owned: ' + entry['target'])
    for key, expected in (('sourceDev', source['dev']), ('sourceIno', source['ino']),
                          ('mountId', exact[0]['id']), ('bootId', boot_id)):
        if entry.get(key) is not None:
            require(entry[key] == expected, 'Ownership identity mismatch: ' + key)
    return True


def launch_payload(previous_foreground, background=False):
    result = {'id': HOME}
    if background or previous_foreground not in ('', HOME, 'com.webos.app.livetv', 'com.webos.app.lgchannels'):
        result['preload'] = 'full'
    return result


def home_pids(executables):
    return sorted(pid for pid, exe in executables.items() if exe == EXE)


OWNER = 'lg-native-home-v1'


def validate_journal(raw):
    if raw is None:
        return dict(version=1, owner=OWNER, mounts=[])
    if isinstance(raw, list):
        # Legacy trial migration still requires live inode/hash/mount proof.
        validate_entries(raw)
        return dict(version=1, owner=OWNER, mounts=raw)
    require(isinstance(raw, dict) and raw.get('version') == 1 and
            raw.get('owner') == OWNER, 'Unknown journal owner/version')
    validate_entries(raw.get('mounts'))
    preload = raw.get('preload')
    if preload is not None:
        require(isinstance(preload, dict) and type(preload.get('original')) is bool and
                type(preload.get('pending')) is bool and isinstance(preload.get('bootId'), str),
                'Invalid preload recovery record')
    return raw


class Controller:
    """Side effects are confined to Ops; every intent is durably saved first."""
    def __init__(self, ops):
        self.ops = ops
        self.journal = None

    def load(self):
        self.journal = validate_journal(self.ops.load_journal())

    def intent(self, action, entry=None):
        self.journal['action'] = action
        self.journal['actionTarget'] = entry['target'] if entry else None
        if entry is not None:
            entry['state'] = action
        self.ops.save_journal(self.journal)

    def is_owned(self, entry):
        mounts = self.ops.mounts()
        if not any(m['target'] == entry['target'] for m in mounts):
            return False
        return owned_mount(entry, self.ops.inspect(entry['source']),
                           self.ops.inspect(entry['target']), mounts, self.ops.boot_id)

    def remove_owned(self):
        errors = []
        for entry in reversed(self.journal['mounts']):
            try:
                if not self.is_owned(entry):
                    continue
                self.intent('unmount', entry)
                self.ops.unmount(entry)
                require(not any(m['target'] == entry['target'] for m in self.ops.mounts()),
                        'Unmount did not remove override')
                entry['state'] = 'removed'
                self.ops.save_journal(self.journal)
            except Exception as exc:
                errors.append(str(exc))
        if errors:
            raise ValueError('Owned cleanup incomplete: ' + '; '.join(errors))

    def assert_no_overrides(self):
        for m in self.ops.mounts():
            # A directory override can hide the actual firmware just like a file bind.
            require(not any(t == m['target'] or t.startswith(m['target'].rstrip('/') + '/')
                            for t in ALLOWED if m['target'] not in ('/', '/usr')),
                    'Unexpected Home/ancestor mount: ' + m['target'])

    def begin_preload(self):
        previous = self.ops.preload()
        require(type(previous) is bool, 'Home runtime preload value missing')
        saved = self.journal.get('preload')
        if saved and saved['pending'] and saved['bootId'] == self.ops.boot_id:
            previous = saved['original']
        self.journal['preload'] = dict(original=previous, bootId=self.ops.boot_id, pending=True)
        self.intent('preload-disable')
        self.ops.set_preload(False)

    def restore_preload(self):
        saved = self.journal.get('preload')
        if not saved or not saved['pending'] or saved['bootId'] != self.ops.boot_id:
            return
        # Even a failed journal write must not prevent restoring a durable pending intent.
        journal_error = None
        try:
            self.intent('preload-restore')
        except Exception as exc:
            journal_error = exc
        self.ops.set_preload(saved['original'])
        require(self.ops.preload() is saved['original'], 'Home preload restore readback failed')
        saved['pending'] = False
        self.ops.save_journal(self.journal)
        if journal_error:
            raise journal_error

    def foreground_or_unknown(self):
        try:
            return self.ops.foreground()
        except Exception:
            return None  # Unknown must never become the explicitly safe empty foreground.

    def restart(self, previous, background=False):
        self.intent('clear-checkpoint')
        self.ops.clear_checkpoint()
        self.intent('kill-exact-home')
        self.ops.kill_home()
        # Recheck too: a user may have changed to HDMI/casting during the transaction.
        current = self.foreground_or_unknown()
        force_background = background or 'preload' in launch_payload(previous) or 'preload' in launch_payload(current)
        payload = launch_payload(previous, force_background)
        self.intent('launch-home')
        self.ops.launch(payload)

    def disable(self):
        self.intent('disable')
        self.ops.disable()

    def apply(self, staging=False):
        if self.ops.is_disabled():
            return 'disabled'
        self.load()
        previous = self.foreground_or_unknown()
        try:
            self.begin_preload()
            manifest = validate_manifest(self.ops.load_manifest(), staging)
            sources = {e['source']: self.ops.inspect(e['source']) for e in manifest}
            for e in manifest:
                validate_file(sources[e['source']], e['source'], e['newHash'])
            self.remove_owned()
            self.assert_no_overrides()
            targets = {e['target']: self.ops.inspect(e['target']) for e in manifest}
            plan = plan_mounts(manifest, sources, targets)  # ALL stock hashes before ANY mount.
            self.journal['mounts'] = []
            for planned in plan:
                identity = [planned[k] for k in ('target', 'newHash', 'uid', 'gid', 'mode')]
                copy_id = hashlib.sha256(json.dumps(identity, separators=(',', ':')).encode('utf-8')).hexdigest()
                entry = dict(planned, inputSource=planned['source'],
                             source=ROOT + '/mount-copies/' + copy_id,
                             bootId=self.ops.boot_id)
                self.journal['mounts'].append(entry)
                self.intent('copy', entry)
                self.ops.copy_source(entry)
                src = validate_file(self.ops.inspect(entry['source']), entry['source'], entry['newHash'])
                entry.update(sourceDev=src['dev'], sourceIno=src['ino'])
                require(not any(m['target'] == entry['target'] for m in self.ops.mounts()), 'Target became mounted')
                validate_file(self.ops.inspect(entry['target']), entry['target'], entry['originalHash'])
                self.intent('bind', entry)
                self.ops.bind(entry)
                require(self.is_owned(entry), 'Bind ownership readback failed')
                entry['mountId'] = next(m['id'] for m in self.ops.mounts() if m['target'] == entry['target'])
                self.intent('readonly', entry)
                self.ops.readonly(entry)
                require(self.is_owned(entry) and 'ro' in next(m['options'] for m in self.ops.mounts()
                        if m['target'] == entry['target']), 'Read-only bind readback failed')
                entry['state'] = 'mounted'
                self.ops.save_journal(self.journal)
            self.restart(previous)
            return 'applied'
        except BaseException as original:
            try:
                self.disable()
                self.remove_owned()
                self.assert_no_overrides()
                self.restart(previous, background=True)
            except BaseException as recovery:
                raise ValueError('Apply failed: %s; rollback incomplete: %s' % (original, recovery)) from original
            raise
        finally:
            self.restore_preload()

    def rollback(self, background=False):
        # Persistent disable is fail-safe even if the journal cannot be parsed.
        try:
            self.load()
        except BaseException:
            self.ops.disable()
            raise
        self.disable()
        previous = self.foreground_or_unknown()
        try:
            self.begin_preload()
            self.remove_owned()
            self.assert_no_overrides()
            self.restart(previous, background)
            return 'rolled-back'
        finally:
            self.restore_preload()


CHECKPOINT_HELPER_SHA256 = 'a3e71aa6e3de494e247731bae7e4cd3363e89beb42c50d6dc17b25d728952865'


def parse_mountinfo(text):
    result = []
    for line in text.splitlines():
        fields = line.split()
        require(len(fields) >= 10 and '-' in fields, 'Malformed mountinfo')
        decode = lambda s: re.sub(r'\\([0-7]{3})', lambda m: chr(int(m[1], 8)), s)
        result.append(dict(id=int(fields[0]), target=decode(fields[4]), options=fields[5].split(',')))
    return result


class LinuxOps:
    """Fixed device paths; no configurable root, shell commands, or broad deletes."""
    def __init__(self):
        require(sys.platform.startswith('linux') and os.geteuid() == 0, 'Requires root on Linux; not a host installer')
        self.secure_parents(ROOT + '/sentinel')
        root = os.lstat(ROOT)
        require(stat.S_IMODE(root.st_mode) == 0o700, 'Owned root must be mode 0700')
        with open('/proc/sys/kernel/random/boot_id') as stream:
            self.boot_id = stream.read().strip()
        require(re.fullmatch('[0-9a-f-]{36}', self.boot_id), 'Invalid kernel boot ID')
        # LG fast boot restores the kernel UUID. HBC recreates this marker per boot.
        startup = os.lstat('/tmp/webosbrew_startup')
        require(stat.S_ISREG(startup.st_mode) and startup.st_uid == 0 and
                startup.st_nlink == 1 and not startup.st_mode & 0o022,
                'Untrusted HBC startup marker')
        self.boot_id += ':' + str(startup.st_mtime_ns)
        self.lock_fd = None

    def secure_parents(self, path):
        require(posixpath.isabs(path) and posixpath.normpath(path) == path, 'Noncanonical path')
        parent = posixpath.dirname(path)
        while True:
            st = os.lstat(parent)
            require(stat.S_ISDIR(st.st_mode) and st.st_uid == 0 and not st.st_mode & 0o022,
                    'Unsafe parent directory: ' + parent)
            if parent == '/': break
            parent = posixpath.dirname(parent)

    def inspect(self, path):
        self.secure_parents(path)
        before = os.lstat(path)
        require(stat.S_ISREG(before.st_mode), 'Not a regular nonsymlink file: ' + path)
        fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW)
        with os.fdopen(fd, 'rb') as stream:
            st = os.fstat(stream.fileno())
            require((st.st_dev, st.st_ino) == (before.st_dev, before.st_ino), 'File changed during open')
            digest = hashlib.sha256()
            while True:
                block = stream.read(65536)
                if not block: break
                digest.update(block)
        return dict(path=path, realpath=os.path.realpath(path), regular=stat.S_ISREG(st.st_mode),
                    symlink=stat.S_ISLNK(st.st_mode), uid=st.st_uid, gid=st.st_gid,
                    mode=stat.S_IMODE(st.st_mode), nlink=st.st_nlink, dev=st.st_dev,
                    ino=st.st_ino, digest=digest.hexdigest())

    def trusted_file(self, path, digest=None):
        return validate_file(self.inspect(path), path, digest)

    def read_json(self, path, optional=False):
        if optional and not os.path.lexists(path): return None
        self.trusted_file(path)
        require(os.stat(path).st_size <= 1024 * 1024, 'Oversized state file')
        def unique(pairs):
            out = {}
            for key, value in pairs:
                require(key not in out, 'Duplicate JSON key')
                out[key] = value
            return out
        with open(path) as stream:
            return json.load(stream, object_pairs_hook=unique)

    def fsync_dir(self, path):
        fd = os.open(path, os.O_RDONLY | os.O_DIRECTORY)
        try: os.fsync(fd)
        finally: os.close(fd)

    def write_json(self, path, data):
        require(path in {ROOT + '/' + p for p in ('home-mounts.json', 'home-boot.json')}, 'Unowned state path')
        self.secure_parents(path)
        if os.path.lexists(path): self.trusted_file(path)
        tmp = path + '.tmp-' + os.urandom(16).hex()
        fd = os.open(tmp, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
        try:
            with os.fdopen(fd, 'w') as stream:
                json.dump(data, stream, sort_keys=True, indent=2)
                stream.write('\n'); stream.flush(); os.fsync(stream.fileno())
            os.replace(tmp, path)
            self.fsync_dir(ROOT)
            require(self.read_json(path) == data, 'Journal readback mismatch')
        finally:
            if os.path.exists(tmp): os.unlink(tmp)

    def lock(self):
        import fcntl
        path = ROOT + '/home.lock'
        self.secure_parents(path)
        if os.path.lexists(path): self.trusted_file(path)
        self.lock_fd = os.open(path, os.O_RDWR | os.O_CREAT | os.O_NOFOLLOW, 0o600)
        fcntl.flock(self.lock_fd, fcntl.LOCK_EX | fcntl.LOCK_NB)

    def unlock(self):
        if self.lock_fd is not None:
            os.close(self.lock_fd); self.lock_fd = None

    def is_disabled(self):
        # Any marker, even an unexpected symlink, disables apply; never follows it.
        return os.path.lexists(ROOT + '/home-disabled')

    def disable(self):
        path = ROOT + '/home-disabled'
        if os.path.lexists(path): return
        fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
        try: os.fsync(fd)
        finally: os.close(fd)
        self.fsync_dir(ROOT)
        require(self.is_disabled(), 'Disable marker readback failed')

    def load_manifest(self): return self.read_json(ROOT + '/home-final-manifest.json')
    def load_journal(self): return self.read_json(ROOT + '/home-mounts.json', optional=True)
    def save_journal(self, data): self.write_json(ROOT + '/home-mounts.json', data)

    def command(self, args, timeout=8):
        result = subprocess.run(args, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                                stderr=subprocess.PIPE, universal_newlines=True, timeout=timeout,
                                env=dict(os.environ, PATH='/usr/sbin:/usr/bin:/sbin:/bin'))
        require(result.returncode == 0, 'Command failed: %r: %s' % (args, result.stderr))
        return result.stdout

    def luna(self, method, payload, timeout=5):
        result = json.loads(self.command(['luna-send', '-n', '1', 'luna://' + method,
                                        json.dumps(payload, separators=(',', ':'))], timeout=timeout))
        require(result.get('returnValue') is True, 'LS2 failure: ' + repr(result))
        return result

    def foreground(self):
        return self.luna('com.webos.applicationManager/getForegroundAppInfo', {}).get('appId')

    def preload(self):
        response = self.luna('com.webos.service.preloadmanager/getPreloadPolicy', {})
        homes = [a for a in response.get('applications', []) if a.get('id') == HOME]
        require(len(homes) == 1 and type(homes[0].get('isEnabled')) is bool, 'Invalid Home preload response')
        return homes[0]['isEnabled']

    def set_preload(self, enabled):
        require(type(enabled) is bool, 'Invalid preload value')
        self.luna('com.webos.service.preloadmanager/setPreloadPolicy', {'application': {'id': HOME, 'isEnabled': enabled}})
        require(self.preload() is enabled, 'Preload change readback mismatch')

    def mounts(self):
        with open('/proc/self/mountinfo') as stream: return parse_mountinfo(stream.read())

    def copy_source(self, entry):
        source_path(entry['inputSource']); source_path(entry['source'])
        self.trusted_file(entry['inputSource'], entry['newHash'])
        if os.path.lexists(entry['source']):
            meta = self.trusted_file(entry['source'], entry['newHash'])
            require(all(meta[k] == entry[k] for k in ('uid', 'gid', 'mode')), 'Existing copy metadata mismatch')
            return  # Immutable content-addressed copy: never truncate/rewrite a backing inode.
        folder = ROOT + '/mount-copies'
        if not os.path.lexists(folder):
            os.mkdir(folder, 0o700); self.fsync_dir(ROOT)
        self.secure_parents(entry['source'])
        fd = os.open(entry['source'], os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
        with os.fdopen(fd, 'wb') as output:
            with open(entry['inputSource'], 'rb') as source:
                while True:
                    block = source.read(65536)
                    if not block: break
                    output.write(block)
            output.flush()
            os.fchown(output.fileno(), entry['uid'], entry['gid'])
            os.fchmod(output.fileno(), entry['mode'])
            os.fsync(output.fileno())
        self.fsync_dir(folder)
        meta = self.trusted_file(entry['source'], entry['newHash'])
        require(all(meta[k] == entry[k] for k in ('uid', 'gid', 'mode')), 'Copied metadata mismatch')

    def bind(self, entry): self.command(['mount', '--bind', entry['source'], entry['target']])
    def readonly(self, entry): self.command(['mount', '-o', 'remount,bind,ro', entry['target']])
    def unmount(self, entry): self.command(['umount', entry['target']])

    def clear_checkpoint(self):
        self.trusted_file(ROOT + '/clear-home-checkpoint.py', CHECKPOINT_HELPER_SHA256)
        self.trusted_file(ROOT + '/checkpoint-backup.json')
        self.command([sys.executable, '-B', '-E', '-s', ROOT + '/clear-home-checkpoint.py', '--apply'])

    def executables(self):
        result = {}
        for name in os.listdir('/proc'):
            if not name.isdigit(): continue
            try: result[int(name)] = os.readlink('/proc/' + name + '/exe')
            except (FileNotFoundError, ProcessLookupError): pass
        return result

    def kill_home(self):
        for pid in home_pids(self.executables()):
            try:
                # Recheck exact executable immediately before the signal, never pkill/killall.
                if os.readlink('/proc/%d/exe' % pid) == EXE: os.kill(pid, signal.SIGKILL)
            except (FileNotFoundError, ProcessLookupError): pass
        time.sleep(0.5)

    def launch(self, payload): self.luna('com.webos.applicationManager/launch', payload)

    def claim_boot(self):
        path = ROOT + '/home-boot.json'
        saved = self.read_json(path, optional=True)
        if saved and saved.get('bootId') == self.boot_id: return False
        self.write_json(path, dict(bootId=self.boot_id, attempted=True))
        return True

    def wait_ready(self, budget=8.0):
        deadline = time.monotonic() + min(45.0, max(0, budget))
        last = 'CRIU restore active'
        while time.monotonic() < deadline:
            if not os.path.lexists('/tmp/criu_restoring'):
                try:
                    for method in ('com.webos.applicationManager/getForegroundAppInfo',
                                   'com.webos.service.preloadmanager/getPreloadPolicy'):
                        remaining = deadline - time.monotonic()
                        if remaining <= 0: raise TimeoutError('Readiness deadline')
                        self.luna(method, {}, timeout=min(1.5, remaining))
                    return
                except (ValueError, OSError, subprocess.SubprocessError, TimeoutError) as exc:
                    last = str(exc)
            remaining = deadline - time.monotonic()
            if remaining > 0: time.sleep(min(0.25, remaining))
        raise TimeoutError('Home readiness budget exhausted: ' + last)


def cli(action, argv=None):
    import argparse
    parser = argparse.ArgumentParser(description='Guarded native Home ' + action + '; local device root only')
    if action == 'apply':
        parser.add_argument('--boot', action='store_true', help='one attempt per boot, bounded readiness wait')
        parser.add_argument('--staging', action='store_true', help='permit 3-7 distinct known QML targets')
    else:
        parser.add_argument('--background', action='store_true', help='always relaunch stock Home with preload=full')
    args = parser.parse_args(argv)
    ops = None
    old_handler = signal.getsignal(signal.SIGTERM)
    def interrupted(signum, frame):
        raise KeyboardInterrupt('Terminated; unwinding Home transaction')
    try:
        signal.signal(signal.SIGTERM, interrupted)
        ops = LinuxOps()
        ops.lock()
        if action == 'apply':
            if ops.is_disabled():
                print('Home overrides disabled; no changes')
                return 0
            if args.boot and not ops.claim_boot():
                print('Home boot attempt already recorded; no retry')
                return 0
            ops.wait_ready(45.0 if args.boot else 8.0)
            ops.trusted_file(ROOT + '/clear-home-checkpoint.py', CHECKPOINT_HELPER_SHA256)
            ops.trusted_file(ROOT + '/checkpoint-backup.json')
            result = Controller(ops).apply(staging=args.staging)
        else:
            # Set persistent disable before readiness checks, even if LS2/CRIU is unavailable.
            ops.disable()
            ops.wait_ready(8.0)
            result = Controller(ops).rollback(background=args.background)
        print(result)
        return 0
    except (Exception, KeyboardInterrupt) as exc:
        print('Native Home failed (inspect journal before retry): ' + str(exc), file=sys.stderr)
        return 1
    finally:
        if ops is not None: ops.unlock()
        signal.signal(signal.SIGTERM, old_handler)
