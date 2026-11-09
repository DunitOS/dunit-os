#!/usr/bin/env python3
"""Native C disabled controls: skipped focus, re-enable, and exact SDK pixels."""
import argparse
from test_c_ui import ROOT, prepare_reference, run_native, verify

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--accel',choices=('tcg','kvm','auto'),default='tcg')
    args=parser.parse_args()
    prepare_reference()
    run_native(args.accel,ROOT/'tools/c_ui_disabled_markers.json')
    verify(400,240,'c-ui-disabled',('initial','enabled','updated'))

if __name__=='__main__':main()
