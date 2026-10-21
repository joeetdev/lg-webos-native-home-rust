"""Local-only verifier: never constructs LinuxOps outside patched unit tests."""
import ast
import contextlib
import hashlib
import io
import json
from pathlib import Path
import platform
import sys
import unittest

HERE = Path(__file__).resolve().parent
RUNTIME = ('native_home.py', 'nativeapply.py', 'rollback.py', 'clear-home-checkpoint.py', '55-lg-native')


def main():
    grammar = []
    for name in RUNTIME:
        if name.endswith('.py'):
            ast.parse((HERE / name).read_text(encoding='utf-8'), filename=name, feature_version=(3, 8))
            grammar.append(name)
    suite = unittest.defaultTestLoader.discover(str(HERE), pattern='test_*.py')
    output = io.StringIO()
    with contextlib.redirect_stdout(output), contextlib.redirect_stderr(output):
        result = unittest.TextTestRunner(stream=output, verbosity=2).run(suite)
    report = dict(success=result.wasSuccessful(), testsRun=result.testsRun,
                  failures=len(result.failures), errors=len(result.errors), skipped=len(result.skipped),
                  localPython=sys.version, host=platform.system(), python38GrammarChecked=grammar,
                  deviceExecution=False, installedHook=False,
                  runtimeSHA256={name: hashlib.sha256((HERE / name).read_bytes()).hexdigest() for name in RUNTIME})
    (HERE / 'verification.log').write_text(output.getvalue(), encoding='utf-8')
    (HERE / 'verification.json').write_text(json.dumps(report, indent=2) + '\n', encoding='utf-8')
    print(json.dumps(report, indent=2))
    return 0 if result.wasSuccessful() else 1


if __name__ == '__main__':
    sys.exit(main())
