#![cfg_attr(not(test), no_std)]
#![cfg_attr(not(test), no_main)]

//! Read-only DunitFS checker CLI. The kernel owns filesystem checking; the SDK
//! owns entry, native error conversion and complete stdout transfers. Exit codes
//! remain 0=clean, 1=degraded, 2=unrecoverable, 3=missing partition/native failure.

extern crate alloc;

use alloc::format;
use dunit::fsck::Report;
use dunit::io::Write;

#[cfg_attr(not(test), dunit::main)]
fn main() -> dunit::Result<()> {
    let code = match run(&mut dunit::io::stdout(), dunit::fsck::check()) {
        Ok(code) => code,
        // A report that could not be delivered must not claim a clean check.
        Err(_) => 3,
    };
    dunit::process::exit(code)
}

fn yes_no(value: u32) -> &'static str {
    if value != 0 {
        "yes"
    } else {
        "no"
    }
}

fn line(output: &mut impl Write, text: alloc::string::String) -> dunit::Result<()> {
    // Keep each short report record together on the native console route.
    output.write_all(text.as_bytes())
}

fn run(output: &mut impl Write, report: dunit::Result<Report>) -> dunit::Result<i32> {
    let report = match report {
        Ok(report) => report,
        Err(error) => {
            line(
                output,
                format!(
                    "[fsck.dunit] error: no Dunit partition or syscall failure (rc={})\n",
                    error.raw_os_error(),
                ),
            )?;
            return Ok(3);
        }
    };
    let verdict = match report.verdict {
        0 => "clean",
        1 => "degraded",
        2 => "unrecoverable",
        _ => "unknown",
    };
    line(
        output,
        format!(
            "[fsck.dunit] verdict={} generation={}\n",
            verdict, report.generation
        ),
    )?;
    line(
        output,
        format!(
            "[fsck.dunit] superblocks: primary={} backup={} slot_crc={}\n",
            yes_no(report.primary_ok),
            yes_no(report.backup_ok),
            yes_no(report.slot_crc_ok),
        ),
    )?;
    line(
        output,
        format!(
            "[fsck.dunit] nodes: {}/{} ok\n",
            report.nodes_ok, report.nodes_total
        ),
    )?;
    line(output, format!(
        "[fsck.dunit] issues: {} corrupt, {} duplicate path(s), {} extent error(s), {} bitmap error(s)\n",
        report.nodes_corrupt, report.duplicate_paths, report.extent_errors, report.bitmap_errors,
    ))?;
    let result = match report.verdict {
        0 => "filesystem is consistent",
        1 => "filesystem mountable read-only; corrupt nodes will be dropped",
        _ => "filesystem unrecoverable; no valid metadata slot",
    };
    line(output, format!("[fsck.dunit] result: {}\n", result))?;
    Ok(report.verdict as i32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::{string::String, vec::Vec};

    #[derive(Default)]
    struct Output(Vec<u8>);
    impl Write for Output {
        fn write(&mut self, bytes: &[u8]) -> dunit::Result<usize> {
            // Force partial writes to verify the CLI delivers the entire report.
            let count = bytes.len().min(7);
            self.0.extend_from_slice(&bytes[..count]);
            Ok(count)
        }
    }

    #[test]
    fn clean_report_preserves_every_record_and_completes_partial_writes() {
        let report = Report {
            primary_ok: 1,
            backup_ok: 2,
            slot_crc_ok: 0,
            generation: u64::MAX,
            nodes_total: 9,
            nodes_ok: 8,
            nodes_corrupt: 1,
            duplicate_paths: 2,
            extent_errors: 3,
            bitmap_errors: 4,
            verdict: 0,
        };
        let mut output = Output::default();
        assert_eq!(run(&mut output, Ok(report)), Ok(0));
        assert_eq!(String::from_utf8(output.0).unwrap(), concat!(
            "[fsck.dunit] verdict=clean generation=18446744073709551615\n",
            "[fsck.dunit] superblocks: primary=yes backup=yes slot_crc=no\n",
            "[fsck.dunit] nodes: 8/9 ok\n",
            "[fsck.dunit] issues: 1 corrupt, 2 duplicate path(s), 3 extent error(s), 4 bitmap error(s)\n",
            "[fsck.dunit] result: filesystem is consistent\n",
        ));
    }

    #[test]
    fn verdicts_preserve_diagnostics_and_exit_codes() {
        for (code, name, result) in [
            (
                1,
                "degraded",
                "filesystem mountable read-only; corrupt nodes will be dropped",
            ),
            (
                2,
                "unrecoverable",
                "filesystem unrecoverable; no valid metadata slot",
            ),
            (
                99,
                "unknown",
                "filesystem unrecoverable; no valid metadata slot",
            ),
        ] {
            let mut output = Output::default();
            assert_eq!(
                run(
                    &mut output,
                    Ok(Report {
                        verdict: code,
                        ..Report::default()
                    })
                ),
                Ok(code as i32)
            );
            let text = String::from_utf8(output.0).unwrap();
            assert!(text.starts_with(&format!("[fsck.dunit] verdict={} generation=0\n", name)));
            assert!(text.ends_with(&format!("[fsck.dunit] result: {}\n", result)));
        }
    }

    #[test]
    fn native_failure_keeps_numeric_error_and_exit_three() {
        let mut output = Output::default();
        assert_eq!(
            run(&mut output, Err(dunit::Error(dunit::errno::ENOENT))),
            Ok(3)
        );
        assert_eq!(
            String::from_utf8(output.0).unwrap(),
            "[fsck.dunit] error: no Dunit partition or syscall failure (rc=-2)\n"
        );
    }

    #[test]
    fn broken_output_cannot_return_a_clean_verdict() {
        struct Broken;
        impl Write for Broken {
            fn write(&mut self, _: &[u8]) -> dunit::Result<usize> {
                Ok(0)
            }
        }
        assert_eq!(
            run(&mut Broken, Ok(Report::default())),
            Err(dunit::Error(dunit::errno::EIO))
        );
    }
}
