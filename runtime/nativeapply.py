#!/usr/bin/env python3
"""Apply fixed native Home overrides; --help is safe on the authoring host."""
import sys
sys.dont_write_bytecode = True
from native_home import cli

if __name__ == '__main__':
    sys.exit(cli('apply'))
