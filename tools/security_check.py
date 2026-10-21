"""Publication hygiene check. Scans Git-visible files; never prints matched secrets."""
import pathlib
import re
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parents[1]
PATTERNS = (
    ('private-key', re.compile(r'-----BEGIN (?:OPENSSH|RSA|EC|DSA|PGP|ENCRYPTED)? ?PRIVATE KEY')),
    ('github-token', re.compile(r'\bgh[pousr]_[A-Za-z0-9]{20,}\b')),
    ('github-pat', re.compile(r'github_pat_[A-Za-z0-9_]{20,}')),
    ('private-network', re.compile(r'\b(?:192\.168\.\d{1,3}\.\d{1,3}|10\.\d{1,3}\.\d{1,3}\.\d{1,3}|172\.(?:1[6-9]|2\d|3[01])\.\d{1,3}\.\d{1,3})\b')),
    ('personal-path', re.compile(r'[A-Za-z]:[\\/]+Users[\\/]+|/Users\/|/home/[^/\s]+/\.ssh')),
    ('mac-address', re.compile(r'\b(?:[0-9a-fA-F]{2}:){5}[0-9a-fA-F]{2}\b')),
    ('proprietary-source', re.compile('Confidential computer '+'software')),
)

def inspect_content(text):
    return [name for name, pattern in PATTERNS if pattern.search(text)]

def inspect_name(name):
    p = pathlib.PurePosixPath(name)
    if p.name in ('.env', 'id_ed25519', 'id_rsa', 'known_hosts') or '.ssh' in p.parts:
        return ['credential-file']
    if p.suffix.lower() in ('.key', '.pem', '.zip', '.tar', '.gz', '.ipk', '.so', '.pyc'):
        return ['private-or-firmware-artifact']
    return []

def main():
    result = subprocess.run(['git', 'ls-files', '-z', '--cached', '--others', '--exclude-standard'],
                            cwd=ROOT, stdout=subprocess.PIPE, check=True)
    names = sorted(set(n.decode('utf-8') for n in result.stdout.split(b'\0') if n))
    errors = []
    for name in names:
        errors.extend((name, issue) for issue in inspect_name(name))
        p = ROOT / name
        if p.is_symlink():
            errors.append((name, 'symlink')); continue
        if not p.is_file():
            errors.append((name, 'missing-file')); continue
        if p.suffix.lower() == '.png' and name.startswith('docs/images/'):
            continue
        try:
            text = p.read_text(encoding='utf-8')
        except UnicodeError:
            errors.append((name, 'unexpected-binary')); continue
        errors.extend((name, issue) for issue in inspect_content(text))
    for name, issue in errors:
        print(name + ': ' + issue)
    print('Publication check: {} Git-visible files, {} findings'.format(len(names), len(errors)))
    return 1 if errors else 0

if __name__ == '__main__':
    sys.exit(main())
