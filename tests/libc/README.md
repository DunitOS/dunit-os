# Upstream libc-test baseline

See `../../docs/libc-test.md` for provenance, integration and validation limits.
`upstream/` contains byte-identical original files at the manifest commit.
`manifest.json` accounts for every upstream `src/*/*.c` path, including deferred
cases. `runner.c` is the only Dunit runtime adapter; upstream cases are unchanged.
