#!/usr/bin/env python3
"""Release owned presentation gate before existing native rollback."""
import sys
sys.dont_write_bytecode=True
if __name__=='__main__':
    if '--help' not in sys.argv:
        from native_gate_control import disable_gate,load_state,luna_call
        disable_gate(load_state(),luna_call)
    from native_home import cli
    sys.exit(cli('rollback'))
