"""One-shot persistent native key-filter ownership; no daemon or boot hooks."""
import copy
import re

ROOT = '/var/lib/lg-native'
OWNER = 'lg-native-remote-controls-v1'
STATE = ROOT + '/remote-controls-state.json'
KEY = 'com.webos.surfacemanager.keyFilters'
LIMIT = 1024 * 1024
FILE_RE = re.compile(r'^/var/lib/lg-native/remote-(core|controller|loader)-([0-9a-f]{64})\.(js|qml)$')


class Error(Exception):
    pass


def require(ok, message):
    if not ok:
        raise Error(message)


def valid_entry(e):
    require(type(e) is dict and set(e) == {'file', 'handler'}, 'Invalid owned entry')
    require(e['handler'] == 'handleRemoteControls' and type(e['file']) is str,
            'Invalid owned handler')
    match = FILE_RE.fullmatch(e['file'])
    require(match is not None and match.group(1) == 'loader' and match.group(3) == 'js',
            'Invalid loader path')


def validate_package(p):
    require(type(p) is dict and set(p) == {'owner', 'version', 'mode', 'entry', 'files'},
            'Invalid package shape')
    require(p['owner'] == OWNER and type(p['version']) is int and p['version'] == 1
            and p['mode'] == 'final', 'Only final version 1 packages accepted')
    valid_entry(p['entry'])
    require(type(p['files']) is dict and len(p['files']) == 3, 'Expected three files')
    kinds = set()
    for path, digest in p['files'].items():
        match = FILE_RE.fullmatch(path) if type(path) is str else None
        require(match is not None, 'Invalid immutable path')
        kind, actual, ext = match.groups()
        require(ext == ('qml' if kind == 'controller' else 'js') and actual == digest,
                'Filename digest mismatch')
        kinds.add(kind)
    require(kinds == {'core', 'controller', 'loader'} and p['entry']['file'] in p['files'],
            'Missing artifact or entry')
    return copy.deepcopy(p)


def validate_chain(chain):
    require(type(chain) is list and all(type(e) is dict for e in chain), 'Invalid filter chain')


def remove_owned(chain, owned):
    validate_chain(chain)
    for e in owned:
        valid_entry(e)
    return copy.deepcopy([e for e in chain if e not in owned])


def build_chain(chain, entry, owned):
    valid_entry(entry)
    result = remove_owned(chain, owned)
    require(not any(e.get('handler') == 'handleRemoteControls' for e in result),
            'Unowned remote handler present')
    anchors = [i for i, e in enumerate(result) if e.get('handler') == 'handleCPHotkeys']
    require(len(anchors) == 1, 'Expected one handleCPHotkeys anchor')
    result.insert(anchors[0], copy.deepcopy(entry))
    return result


def read_state(io):
    state = io.read_json(STATE, optional=True)
    if state is None:
        return None
    require(type(state) is dict and set(state) == {'owner', 'version', 'package', 'owned'},
            'Invalid ownership state')
    require(state['owner'] == OWNER and type(state['version']) is int
            and state['version'] == 1, 'Invalid ownership identity')
    validate_package(state['package'])
    require(type(state['owned']) is list and 0 < len(state['owned']) <= 128,
            'Invalid owned entries')
    for entry in state['owned']:
        valid_entry(entry)
    require(state['package']['entry'] in state['owned'], 'Current entry not journaled')
    return state


def verify_files(io, package):
    import hashlib
    for path, digest in package['files'].items():
        require(hashlib.sha256(io.read_private(path)).hexdigest() == digest,
                'Artifact integrity failure')


def write_chain(io, before, desired):
    if before == desired:
        return False
    require(io.get_filters() == before, 'Concurrent configuration change; retry')
    io.set_filters(desired)
    require(io.get_filters() == desired, 'Configuration readback mismatch; ownership retained')
    return True


def install(io, package_path):
    package = validate_package(io.read_json(package_path))
    verify_files(io, package)
    old = read_state(io)
    owned = copy.deepcopy(old['owned']) if old else []
    before = io.get_filters()
    desired = build_chain(before, package['entry'], owned)
    if package['entry'] not in owned:
        owned.append(copy.deepcopy(package['entry']))
    require(len(owned) <= 128, 'Ownership history full')
    state = {'owner': OWNER, 'version': 1, 'package': package, 'owned': owned}
    if state != old:
        io.atomic_json(STATE, state)
        require(io.read_json(STATE) == state, 'Ownership readback mismatch')
    changed = write_chain(io, before, desired)
    return {'ok': True, 'changed': changed, 'filters': len(desired)}


def disable(io):
    state = read_state(io)
    if state is None:
        return {'ok': True, 'changed': False, 'reason': 'no ownership state'}
    before = io.get_filters()
    desired = remove_owned(before, state['owned'])
    changed = write_chain(io, before, desired)
    return {'ok': True, 'changed': changed, 'filters': len(desired)}


def check(io):
    state = read_state(io)
    require(state is not None, 'No ownership state')
    verify_files(io, state['package'])
    live = io.get_filters()
    validate_chain(live)
    remote = [e for e in live if e.get('handler') == 'handleRemoteControls']
    require(remote == [state['package']['entry']], 'Expected exactly one current remote entry')
    require(build_chain(live, state['package']['entry'], state['owned']) == live,
            'Remote entry is not before handleCPHotkeys')
    return {'ok': True, 'filters': len(live)}


def validate_metadata(st, directory=False):
    import stat
    require(st.st_uid == 0, 'Expected root ownership')
    if directory:
        require(stat.S_ISDIR(st.st_mode) and not st.st_mode & 0o022,
                'Unsafe parent directory')
    else:
        require(stat.S_ISREG(st.st_mode) and not st.st_mode & 0o077,
                'Expected private regular file')
        require(0 < st.st_size <= LIMIT, 'File size outside limit')


class LinuxIO:
    def __init__(self):
        import os
        require(hasattr(os, 'geteuid') and os.geteuid() == 0, 'Linux root required')

    def parents(self, path):
        import os
        import posixpath
        require(type(path) is str and path.startswith('/') and
                posixpath.normpath(path) == path and not path.startswith('//'),
                'Expected canonical absolute path')
        parent = posixpath.dirname(path)
        while True:
            validate_metadata(os.lstat(parent), directory=True)
            if parent == '/':
                break
            parent = posixpath.dirname(parent)

    def read_private(self, path):
        import os
        self.parents(path)
        before = os.lstat(path)
        validate_metadata(before)
        fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
        with os.fdopen(fd, 'rb') as stream:
            opened = os.fstat(stream.fileno())
            validate_metadata(opened)
            require((before.st_dev, before.st_ino) == (opened.st_dev, opened.st_ino),
                    'File changed while opening')
            data = stream.read(LIMIT + 1)
            require(0 < len(data) <= LIMIT and len(data) == opened.st_size,
                    'File changed or exceeds size limit')
            return data

    def read_json(self, path, optional=False):
        import json
        try:
            data = self.read_private(path)
        except FileNotFoundError:
            if optional:
                return None
            raise
        def pairs(items):
            value = {}
            for key, item in items:
                require(key not in value, 'Duplicate JSON key')
                value[key] = item
            return value
        return json.loads(data.decode('utf-8'), object_pairs_hook=pairs)

    def atomic_json(self, path, value):
        import json
        import os
        import tempfile
        require(path == STATE, 'Only ownership state may be written')
        self.parents(path)
        try:
            self.read_private(path)
        except FileNotFoundError:
            pass
        data = json.dumps(value, sort_keys=True, separators=(',', ':')).encode('utf-8')
        require(0 < len(data) <= LIMIT, 'Ownership state too large')
        fd, temporary = tempfile.mkstemp(prefix='.remote-controls-', dir=ROOT)
        try:
            with os.fdopen(fd, 'wb') as stream:
                os.fchmod(stream.fileno(), 0o600)
                stream.write(data)
                stream.flush()
                os.fsync(stream.fileno())
            os.replace(temporary, path)
            directory = os.open(ROOT, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
            try:
                os.fsync(directory)
            finally:
                os.close(directory)
        finally:
            if os.path.exists(temporary):
                os.unlink(temporary)

    def luna(self, method, payload):
        import json
        import subprocess
        result = subprocess.run(['luna-send', '-n', '1',
                                 'luna://com.webos.service.config/' + method,
                                 json.dumps(payload, separators=(',', ':'))],
                                stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                                stderr=subprocess.DEVNULL, timeout=10, check=False)
        require(result.returncode == 0 and len(result.stdout) <= LIMIT, 'Config call failed')
        response = json.loads(result.stdout.decode('utf-8'))
        require(type(response) is dict and response.get('returnValue') is True,
                'Config service rejected request')
        return response

    def get_filters(self):
        configs = self.luna('getConfigs', {'configNames': [KEY]}).get('configs')
        require(type(configs) is dict and KEY in configs, 'Missing keyFilters response')
        validate_chain(configs[KEY])
        return configs[KEY]

    def set_filters(self, chain):
        validate_chain(chain)
        self.luna('setConfigs', {'configs': {KEY: chain}})


def main(argv=None, io=None):
    import argparse
    import json
    import subprocess
    parser = argparse.ArgumentParser(description=__doc__)
    group = parser.add_mutually_exclusive_group(required=True)
    group.add_argument('--install', metavar='PACKAGE')
    group.add_argument('--disable', action='store_true')
    group.add_argument('--check', action='store_true')
    args = parser.parse_args(argv)
    try:
        io = io if io is not None else LinuxIO()
        result = install(io, args.install) if args.install else disable(io) if args.disable else check(io)
        print(json.dumps(result, sort_keys=True))
        return 0
    except (Error, OSError, ValueError, TypeError, KeyError, subprocess.SubprocessError) as exc:
        # Never print service responses, subprocess output, paths or arbitrary exceptions.
        print(json.dumps({'ok': False, 'error': str(exc) if isinstance(exc, Error)
                          else 'I/O or data failure; ownership retained'}))
        return 1


if __name__ == '__main__':
    import sys
    sys.exit(main())
