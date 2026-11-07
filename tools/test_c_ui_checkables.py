#!/usr/bin/env python3
"""Native C-controlled checkables, using the ordinary Rust SDK and QEMU gate."""
import argparse
from test_c_ui import ROOT, prepare_reference, run_native, verify

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--accel',choices=('tcg','kvm','auto'),default='tcg')
    args=parser.parse_args()
    prepare_reference()
    run_native(args.accel,ROOT/'tools/c_ui_checkables_markers.json')
    verify(400,240,'c-ui-checkables',('initial','checked','updated'))

if __name__=='__main__':main()
