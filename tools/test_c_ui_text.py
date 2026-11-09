#!/usr/bin/env python3
"""Controlled native C Input/Textarea, using the SDK editor and exact pixels."""
import argparse
from test_c_ui import ROOT, prepare_reference, run_native, verify

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--accel',choices=('tcg','kvm','auto'),default='tcg')
    args=parser.parse_args()
    prepare_reference()
    run_native(args.accel,ROOT/'tools/c_ui_text_markers.json')
    verify(560,460,'c-ui-text',('initial','edited','reset'))

if __name__=='__main__':main()
