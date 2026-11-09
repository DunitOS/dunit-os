#!/usr/bin/env python3
"""Native C ranges, signed editing, passive progress and mixed text callbacks."""
import argparse
from test_c_ui import ROOT, prepare_reference, run_native, verify

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--accel',choices=('tcg','kvm','auto'),default='tcg')
    args=parser.parse_args()
    prepare_reference()
    run_native(args.accel,ROOT/'tools/c_ui_numbers_markers.json')
    verify(560,520,'c-ui-numbers',('initial','changed','reset'))

if __name__=='__main__':main()
