import importlib.util
import pathlib
import unittest
from types import SimpleNamespace
P=pathlib.Path(__file__).with_name('test.py')
spec=importlib.util.spec_from_file_location('suite_runner',P)
m=importlib.util.module_from_spec(spec)
if P.exists():spec.loader.exec_module(m)
class RunnerTests(unittest.TestCase):
    def test_plan_contains_all_suites_and_only_offline_executables(self):
        plan=m.commands();self.assertEqual(len(plan),8)
        self.assertEqual({str(p[1].relative_to(m.ROOT)) for p in plan},{'runtime','gate','remote','home','tools'})
        for _,_,cmd in plan:self.assertNotIn('ssh',cmd);self.assertNotIn('luna-send',cmd)
    def test_nested_home_tests_are_discovered(self):
        home=next(cmd for name,_,cmd in m.commands() if name=='home')
        self.assertEqual(home[home.index('-s')+1],'tests')
    def test_stops_on_first_nonzero_result(self):
        calls=[]
        def run(cmd,**kwargs):calls.append(cmd);return SimpleNamespace(returncode=7)
        self.assertEqual(m.run_all(run=run),7);self.assertEqual(len(calls),1)
if __name__=='__main__':unittest.main()
