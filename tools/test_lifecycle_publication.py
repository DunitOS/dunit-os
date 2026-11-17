#!/usr/bin/env python3
"""Actual exit finalization/leases/registry: cleanup precedes wait readiness."""
import json
import os
from pathlib import Path
import subprocess
import tempfile
from dunit_cli import ROOT


if __name__ == '__main__':
    source = (ROOT / 'tools/test_lifecycle_publication_host.rs.in').read_text()
    for name, path in [('SYNC', 'kernel/src/sync.rs'),
                       ('REGISTRY', 'kernel/src/process/registry.rs'),
                       ('EXECUTION', 'kernel/src/process/execution.rs'),
                       ('FINALIZATION', 'kernel/src/process/finalization.rs')]:
        source = source.replace('@' + name + '@', json.dumps(str(ROOT / path)))
    lifecycle = (ROOT / 'kernel/src/process/lifecycle.rs').read_text()
    start = lifecycle.index('pub(crate) fn child_exit_snapshot(')
    end = lifecycle.index('\npub fn wait_for_child(', start)
    source = source.replace('@CHILD_SNAPSHOT@', lifecycle[start:end])
    with tempfile.TemporaryDirectory() as directory:
        path = Path(directory) / 'publication.rs'
        path.write_text(source)
        binary = path.with_suffix('')
        subprocess.run([os.environ.get('RUSTC', 'rustc'), '--edition=2021', '-O',
                        str(path), '-o', str(binary)], check=True)
        subprocess.run([str(binary)], check=True, timeout=60)

    resources = (ROOT / 'kernel/src/process/resources.rs').read_text()
    start = resources.index('    pub(super) fn release_tls_allocation(')
    end = resources.index('    pub(super) fn take_fd_resources(', start)
    cleanup = resources[start:end]
    process = (ROOT / 'kernel/src/process/mod.rs').read_text()
    start = process.index('fn vm_range_end(')
    end = process.index('\nimpl Process {', start)
    source = (ROOT / 'tools/test_tls_cleanup_host.rs.in').read_text()
    source = source.replace('@RANGE@', process[start:end]).replace('@TLS_CLEANUP@', cleanup)
    # This fixture lives at crate root; the method remains private to the test.
    source = source.replace('pub(super) fn release_tls_allocation', 'fn release_tls_allocation')
    with tempfile.TemporaryDirectory() as directory:
        path = Path(directory) / 'tls.rs'
        path.write_text(source)
        binary = path.with_suffix('')
        subprocess.run([os.environ.get('RUSTC', 'rustc'), '--edition=2021', '-O',
                        str(path), '-o', str(binary)], check=True)
        subprocess.run([str(binary)], check=True, timeout=30)
