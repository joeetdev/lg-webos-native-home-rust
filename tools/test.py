"""Run all offline suites. No SSH, Luna calls or device deployment."""
import os
import pathlib
import shutil
import subprocess
import sys

ROOT=pathlib.Path(__file__).resolve().parents[1]
def commands():
    node=shutil.which('node') or 'node'
    py=[sys.executable,'-B','-m','unittest','discover','-s','.']
    return [
        ('runtime',ROOT/'runtime',py+['-p','test_*.py']),
        ('gate-python',ROOT/'gate',py+['-p','tests_*.py']),
        ('gate-packing',ROOT/'gate',[sys.executable,'-B','tests-pack-gate.py']),
        ('gate-javascript',ROOT/'gate',[node,'--test','--test-reporter=dot','tests-gate-proof.cjs','tests-gate-surface.cjs','tests-gate-loader.cjs']),
        ('remote-python',ROOT/'remote',py+['-p','tests_*.py']),
        ('remote-javascript',ROOT/'remote',[node,'--test','--test-reporter=dot','tests-remote-core.cjs','tests-remote-controller.cjs','tests-remote-loader.cjs']),
        ('home',ROOT/'home',[sys.executable,'-B','-m','unittest','discover','-s','tests','-p','test*.py']),
        ('publication-tools',ROOT/'tools',py+['-p','test_*.py']),
    ]
def run_all(run=subprocess.run):
    for name,cwd,cmd in commands():
        print('\n== '+name+' ==',flush=True)
        if not cwd.is_dir() or not list(cwd.rglob('*test*.py')) and not name.endswith('javascript'):
            print('Required test suite is missing: '+name);return 2
        result=run(cmd,cwd=cwd,env=dict(os.environ,PYTHONDONTWRITEBYTECODE='1'))
        if result.returncode:return result.returncode
    return 0
if __name__=='__main__':sys.exit(run_all())
