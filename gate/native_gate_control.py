"""Root-only, reversible native Home gate recovery (Python >= 3.8).

Importing this module performs no device IO. Only recorded owned key-filter
entries may be removed; Home mounts, preload policy, processes and files stay
untouched.
"""
import argparse
import json
import os
import posixpath
import re
import stat
import subprocess
import sys
import types

ROOT = '/var/lib/lg-native'
STATE_NAME = 'native-home-gate-state.json'
OWNER = 'lg-native-home-gate-v1'
KEY = 'com.webos.surfacemanager.keyFilters'


def require(ok, message):
    if not ok:
        raise ValueError(message)


def validate_entry(entry):
    require(isinstance(entry, dict) and set(entry) == {'file', 'handler'} and
            entry.get('handler') == 'handleNativeHomeGate' and
            isinstance(entry.get('file'), str) and re.fullmatch(
                re.escape(ROOT) + r'/native-home-gate-loader-[a-f0-9]{64}\.js', entry['file']),
            'Invalid owned gate entry')
    return entry


def validate_state(state):
    require(isinstance(state, dict) and type(state.get('version')) is int and
            state['version'] == 1 and state.get('owner') == OWNER,
            'Unknown gate state owner/version')
    entry = validate_entry(state.get('entry'))
    if 'ownedEntries' in state:
        entries = state['ownedEntries']
        require(isinstance(entries, list) and 1 <= len(entries) <= 4,
                'Expected one to four owned gate entries')
        for owned in entries:
            validate_entry(owned)
        require(entry in entries, 'Active gate entry missing from ownedEntries')
        require(len({(e['file'], e['handler']) for e in entries}) == len(entries),
                'Duplicate owned gate entry')
    if 'files' in state:
        require(isinstance(state['files'], dict), 'Gate files must be an object')
        for path, digest in state['files'].items():
            require(isinstance(path, str) and path.startswith('/') and not path.startswith('//') and
                    posixpath.normpath(path) == path and '\\' not in path and
                    all(33 <= ord(c) < 127 for c in path), 'Noncanonical gate file path')
            require(isinstance(digest, str) and re.fullmatch('[a-f0-9]{64}', digest),
                    'Invalid gate file SHA256')
    return state


def remove_owned_filter(filters, state):
    """Return a fresh list without EXACT matches; preserve foreign values/order."""
    state = validate_state(state)
    entries = state.get('ownedEntries', [state['entry']])
    require(isinstance(filters, list), 'keyFilters must be an array')
    return [item for item in filters if item not in entries]


def checked_call(call, method, payload):
    response = call('com.webos.service.config/' + method, payload)
    require(isinstance(response, dict) and response.get('returnValue') is True,
            'Config LS2 call failed: ' + method)
    return response


def get_filters(call):
    response = checked_call(call, 'getConfigs', {'configNames': [KEY]})
    configs = response.get('configs')
    require(isinstance(configs, dict) and isinstance(configs.get(KEY), list),
            'Missing or invalid keyFilters response')
    return configs[KEY]


def disable_gate(state, call):
    """Remove exact entries in ownedEntries (legacy fallback: [state.entry]).

    call(method, payload) is the injectable LS2 boundary.

    disabled=True means verified absent (including an already absent entry).
    No state means no LS2 calls and disabled=False. Never restore a saved list.
    """
    if state is None:
        return dict(disabled=False, changed=False)
    validate_state(state)
    before = get_filters(call)
    after = remove_owned_filter(before, state)
    changed = after != before
    if changed:
        checked_call(call, 'setConfigs', {'configs': {KEY: after}})
        require(json.dumps(get_filters(call), sort_keys=True, separators=(',', ':'), allow_nan=False) ==
                json.dumps(after, sort_keys=True, separators=(',', ':'), allow_nan=False),
                'Gate removal readback mismatch')
    return dict(disabled=True, changed=changed)


def secure_parents(path):
    require(isinstance(path, str) and path.startswith('/') and not path.startswith('//') and
            posixpath.normpath(path) == path and '\\' not in path and
            all(33 <= ord(c) < 127 for c in path), 'Noncanonical trusted path')
    parent = posixpath.dirname(path)
    while True:
        info = os.lstat(parent)
        require(stat.S_ISDIR(info.st_mode) and info.st_uid == 0 and not info.st_mode & 0o022,
                'Unsafe parent directory: ' + parent)
        if parent == '/':
            break
        parent = posixpath.dirname(parent)


def trusted_bytes(path, optional=False):
    """Read one trusted inode, not a check-then-reopen pathname. No writes."""
    secure_parents(path)
    try:
        before = os.lstat(path)
    except FileNotFoundError:
        if optional:
            return None
        raise
    def validate(info):
        require(stat.S_ISREG(info.st_mode) and info.st_uid == 0 and info.st_nlink == 1 and
                not info.st_mode & 0o022 and info.st_size <= 1024 * 1024,
                'Unsafe trusted file: ' + path)
    validate(before)
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    try:
        after = os.fstat(fd)
        validate(after)
        require((before.st_dev, before.st_ino) == (after.st_dev, after.st_ino),
                'Trusted file changed during open')
        with os.fdopen(fd, 'rb') as stream:
            fd = None
            raw = stream.read(1024 * 1024 + 1)
        require(len(raw) <= 1024 * 1024, 'Oversized trusted file')
        return raw
    finally:
        if fd is not None:
            os.close(fd)


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        require(key not in result, 'Duplicate JSON key: ' + key)
        result[key] = value
    return result


def load_state(root=ROOT):
    # An uninstalled private root is a no-op, not a reason to touch config.
    # Validate its ancestors first: a dangling symlink is NOT an absent root.
    secure_parents(root)
    try:
        directory = os.lstat(root)
    except FileNotFoundError:
        return None
    require(stat.S_ISDIR(directory.st_mode) and directory.st_uid == 0 and
            not directory.st_mode & 0o022, 'Unsafe gate state root')
    raw = trusted_bytes(root + '/' + STATE_NAME, optional=True)
    if raw is None:
        return None
    return validate_state(json.loads(raw.decode('utf-8'), object_pairs_hook=unique_object))


def load_native(root=ROOT):
    """Execute validated bytes from the private root, never import by CWD/name."""
    path = root + '/native_home.py'
    raw = trusted_bytes(path)
    module = types.ModuleType('_lg_native_home_gate_runtime')
    module.__file__ = path
    exec(compile(raw, path, 'exec'), module.__dict__)
    return module


def native_ready(ops=None, native=None):
    """Read-only mount health proof; raises on incomplete or foreign evidence.

    No foreground/preload LS2 calls, lock creation, waits or transactions. The
    production LinuxOps supplies secure reads and the composite current boot ID.
    """
    if native is None:
        native = load_native()
    if ops is None:
        ops = native.LinuxOps()
    require(not ops.is_disabled(), 'Native Home is disabled')
    manifest = native.validate_manifest(ops.load_manifest())
    require(len(manifest) == 8 and {e['target'] for e in manifest} == native.ALLOWED,
            'Expected all eight native Home overlays')
    raw_journal = ops.load_journal()
    require(isinstance(raw_journal, dict), 'Modern native Home journal required')
    controller = native.Controller(ops)
    controller.journal = native.validate_journal(raw_journal)
    journal = controller.journal
    preload = journal.get('preload')
    require(isinstance(preload, dict) and preload.get('pending') is False and
            preload.get('bootId') == ops.boot_id, 'Native preload recovery incomplete or stale')
    require(len(journal['mounts']) == 8, 'Expected eight journal-owned overlays')
    by_target = {e['target']: e for e in manifest}
    require({e['target'] for e in journal['mounts']} == set(by_target),
            'Journal/manifest target mismatch')
    for entry in journal['mounts']:
        expected = by_target[entry['target']]
        require(entry.get('state') == 'mounted' and entry.get('bootId') == ops.boot_id and
                all(type(entry.get(k)) is int for k in ('mountId', 'sourceDev', 'sourceIno')),
                'Missing or stale current-boot mount identity')
        require(entry.get('inputSource') == expected['source'] and
                all(entry[k] == expected[k] for k in ('originalHash', 'newHash')),
                'Journal/manifest source or hash mismatch')
        native.validate_file(ops.inspect(expected['source']), expected['source'], expected['newHash'])
        # This is LIVE mount/inode/hash proof, not journal text claiming ownership.
        require(controller.is_owned(entry), 'Native Home mount is not owned')
        source = ops.inspect(entry['source'])
        target = ops.inspect(entry['target'])
        require(all(entry.get(k) == source[k] == target[k] for k in ('uid', 'gid', 'mode')),
                'Native Home mount metadata mismatch')
        exact = [m for m in ops.mounts() if m['target'] == entry['target']]
        require(len(exact) == 1 and 'ro' in exact[0]['options'], 'Native Home mount is not readonly')
    return True


def recover_if_unready(state, call, checker=None):
    """A failed/unknown health check MUST NOT prevent gate removal.

    Invalid gate ownership or failed LS2 readback still raise: do not invent
    ownership, remove another generation, or report unverified recovery.
    """
    if state is not None:
        validate_state(state)
    if checker is None:
        checker = native_ready
    reason = None
    try:
        ready = checker() is True
    except Exception as exc:
        ready = False
        reason = '%s: %s' % (type(exc).__name__, exc)
    if ready:
        return dict(ready=True, disabled=False, changed=False)
    result = dict(ready=False, **disable_gate(state, call))
    if reason is not None:
        result['reason'] = reason
    return result


def luna_call(method, payload):
    """Only the two config methods are available; no shell, stdin or retries."""
    require(method in ('com.webos.service.config/getConfigs',
                       'com.webos.service.config/setConfigs'), 'Unapproved Luna method')
    completed = subprocess.run(
        ['luna-send', '-n', '1', 'luna://' + method, json.dumps(payload, separators=(',', ':'))],
        stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
        universal_newlines=True, timeout=10, shell=False,
        env=dict(os.environ, PATH='/usr/sbin:/usr/bin:/sbin:/bin'))
    require(completed.returncode == 0, 'Luna command failed: ' + completed.stderr)
    response = json.loads(completed.stdout, object_pairs_hook=unique_object)
    require(isinstance(response, dict) and response.get('returnValue') is True,
            'Luna returned failure: ' + repr(response))
    return response


def cli(argv=None, call=None, checker=None):
    parser = argparse.ArgumentParser(description='Root-only native Home gate safety control')
    action = parser.add_mutually_exclusive_group(required=True)
    action.add_argument('--disable', action='store_true', help='remove only the exact owned gate filter')
    action.add_argument('--recover-if-unready', action='store_true', help='remove gate unless all eight overlays are healthy')
    args = parser.parse_args(argv)
    try:
        require(sys.platform.startswith('linux') and os.geteuid() == 0,
                'Requires root on Linux; not a host installer')
        state = load_state()
        if state is None:
            result = dict(disabled=False, changed=False, state='absent')
            if args.recover_if_unready:
                result['ready'] = False
        elif args.disable:
            # Independent of native_home import, journal, mounts and UI health.
            result = disable_gate(state, call if call is not None else luna_call)
        else:
            result = recover_if_unready(state, call if call is not None else luna_call, checker)
        print(json.dumps(result, sort_keys=True))
        return 0
    except Exception as exc:
        print(json.dumps(dict(error='%s: %s' % (type(exc).__name__, exc)), sort_keys=True))
        return 1


if __name__ == '__main__':
    sys.exit(cli())
