//! Bounded text protocol and asynchronous process boundary for Lua layout policy.
extern crate alloc;

use alloc::vec::Vec;

const MAX_OUTPUT: usize = 16 * 1024;
const MAX_DIMENSION: u32 = 16_384;
const MAX_WINDOWS: usize = 64;
const MAX_DURATION_MS: u32 = 2000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Policy {
    pub rects: Vec<Rect>,
    pub duration_ms: u32,
    pub ease_out: bool,
}

fn number<T: core::str::FromStr>(word: Option<&str>) -> Result<T, &'static str> {
    word.ok_or("missing field")?
        .parse()
        .map_err(|_| "invalid number")
}

/// Accept exactly one complete response for the geometry requested by the host.
/// No partial policy is exposed when a late line or trailing output is invalid.
fn parse(output: &[u8], width: u32, height: u32, count: usize) -> Result<Policy, &'static str> {
    if output.len() > MAX_OUTPUT
        || width == 0
        || height == 0
        || width > MAX_DIMENSION
        || height > MAX_DIMENSION
        || count > MAX_WINDOWS
    {
        return Err("invalid response size or geometry");
    }
    let text = core::str::from_utf8(output).map_err(|_| "non-UTF8 output")?;
    if !text.ends_with('\n') {
        return Err("incomplete response");
    }
    let mut lines = text.lines();
    let mut header = lines
        .next()
        .ok_or("missing header")?
        .split_ascii_whitespace();
    if header.next() != Some("DWM_LUA")
        || header.next() != Some("1")
        || number::<u32>(header.next())? != width
        || number::<u32>(header.next())? != height
        || number::<usize>(header.next())? != count
        || header.next().is_some()
    {
        return Err("unexpected response header");
    }
    let mut rects = Vec::new();
    for index in 0..count {
        let mut fields = lines
            .next()
            .ok_or("missing rectangle")?
            .split_ascii_whitespace();
        if fields.next() != Some("RECT") || number::<usize>(fields.next())? != index + 1 {
            return Err("non-sequential rectangle index");
        }
        let rect = Rect {
            x: number(fields.next())?,
            y: number(fields.next())?,
            w: number(fields.next())?,
            h: number(fields.next())?,
        };
        if fields.next().is_some()
            || rect.x < 0
            || rect.y < 0
            || rect.w <= 0
            || rect.h <= 0
            || rect.x.checked_add(rect.w).ok_or("rectangle overflow")? > width as i32
            || rect.y.checked_add(rect.h).ok_or("rectangle overflow")? > height as i32
        {
            return Err("rectangle outside requested geometry");
        }
        rects.push(rect);
    }
    let mut animation = lines
        .next()
        .ok_or("missing animation")?
        .split_ascii_whitespace();
    if animation.next() != Some("ANIMATION") {
        return Err("missing animation");
    }
    let duration_ms = number::<u32>(animation.next())?;
    if duration_ms > MAX_DURATION_MS {
        return Err("animation duration exceeds contract");
    }
    let ease_out = match animation.next() {
        Some("linear") => false,
        Some("ease_out") => true,
        _ => return Err("unsupported animation curve"),
    };
    if animation.next().is_some() || lines.next() != Some("END") || lines.next().is_some() {
        return Err("trailing or incomplete response");
    }
    Ok(Policy {
        rects,
        duration_ms,
        ease_out,
    })
}

#[cfg(not(test))]
mod runtime {
    use super::*;
    use crate::settings;
    use alloc::format;

    pub enum Poll {
        Pending,
        Ready(Policy),
        Failed(&'static str),
    }

    struct Child {
        pid: u32,
        pty: u32,
        deadline: u64,
        width: u32,
        height: u32,
        count: usize,
        output: Vec<u8>,
        exited: bool,
    }

    pub struct Engine {
        child: Option<Child>,
    }

    fn clock() -> Result<(u64, u64), &'static str> {
        let mut stats = libdunit::SystemStats::default();
        if libdunit::get_system_stats(&mut stats) < 0
            || stats.uptime_available == 0
            || stats.uptime_hz == 0
        {
            return Err("monotonic clock unavailable");
        }
        Ok((stats.uptime_ticks, stats.uptime_hz))
    }

    impl Engine {
        pub const fn new() -> Self {
            Self { child: None }
        }

        pub fn start(
            &mut self,
            config: &settings::Scripts,
            width: u32,
            height: u32,
            count: usize,
        ) -> Result<(), &'static str> {
            self.cancel();
            if !config.enabled {
                return Err("scripts disabled");
            }
            if width == 0
                || height == 0
                || width > MAX_DIMENSION
                || height > MAX_DIMENSION
                || count > MAX_WINDOWS
            {
                return Err("invalid requested geometry");
            }
            let (now, hz) = clock()?;
            let ticks = (config.timeout_ms.clamp(50, 5000) as u64)
                .saturating_mul(hz)
                .saturating_add(999)
                / 1000;
            let pty = libdunit::pty_create();
            if pty < 0 {
                return Err("PTY creation failed");
            }
            let script = format!("DWM_LUA_SCRIPT={}", config.layout.as_str());
            let width_env = format!("DWM_LUA_WIDTH={}", width);
            let height_env = format!("DWM_LUA_HEIGHT={}", height);
            let count_env = format!("DWM_LUA_WINDOWS={}", count);
            let pid = libdunit::pty_spawn_env(
                config.runner.as_str(),
                pty as u32,
                &[&script, &width_env, &height_env, &count_env],
            );
            if pid <= 0 {
                libdunit::pty_close(pty as u32);
                return Err("Lua runner spawn failed");
            }
            self.child = Some(Child {
                pid: pid as u32,
                pty: pty as u32,
                deadline: now.saturating_add(ticks.max(1)),
                width,
                height,
                count,
                output: Vec::new(),
                exited: false,
            });
            Ok(())
        }

        pub fn cancel(&mut self) {
            if let Some(child) = self.child.take() {
                if !child.exited {
                    libdunit::kill(child.pid);
                    let mut status = libdunit::WaitStatus::empty();
                    libdunit::wait(child.pid, &mut status);
                }
                libdunit::pty_close(child.pty);
            }
        }

        fn fail(&mut self, reason: &'static str) -> Poll {
            self.cancel();
            Poll::Failed(reason)
        }

        pub fn poll(&mut self) -> Poll {
            let Some(child) = self.child.as_ref() else {
                return Poll::Pending;
            };
            let now = match clock() {
                Ok((now, _)) => now,
                Err(reason) => return self.fail(reason),
            };
            let child = self.child.as_mut().unwrap();
            let mut drained = false;
            // Bound syscall work per compositor frame as well as total storage.
            for _ in 0..4 {
                let mut buffer = [0u8; 1024];
                let read = libdunit::pty_read(child.pty, &mut buffer);
                if read == 0 || read == libdunit::EPIPE {
                    drained = true;
                    break;
                }
                if read < 0 {
                    return self.fail("Lua runner output read failed");
                }
                let read = read as usize;
                if read > buffer.len() || child.output.len().saturating_add(read) > MAX_OUTPUT {
                    return self.fail("Lua runner output exceeds 16 KiB");
                }
                child.output.extend_from_slice(&buffer[..read]);
            }
            if child.exited && drained {
                let result = parse(&child.output, child.width, child.height, child.count);
                self.cancel();
                return match result {
                    Ok(policy) => Poll::Ready(policy),
                    Err(reason) => Poll::Failed(reason),
                };
            }
            if !child.exited {
                let mut status = libdunit::WaitStatus::empty();
                let waited = libdunit::wait(child.pid, &mut status);
                if waited > 0 {
                    child.exited = true;
                    if !status.exited() || status.code != 0 {
                        return self.fail("Lua runner exited unsuccessfully");
                    }
                    // Drain again on the next frame: the child could have
                    // written between the last pty_read and its exit/reap.
                } else if waited != libdunit::EAGAIN {
                    return self.fail("Lua runner wait failed");
                }
            }
            // Observe completion before enforcing a running process's deadline.
            // A busy compositor may next poll after the child has already exited;
            // in that case the bounded PTY drain must still deliver its result.
            if !child.exited && now >= child.deadline {
                return self.fail("Lua runner timed out");
            }
            Poll::Pending
        }
    }

    impl Drop for Engine {
        fn drop(&mut self) {
            self.cancel();
        }
    }
}

#[cfg(not(test))]
pub use runtime::{Engine, Poll};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn complete_policy_with_crlf_and_zero_windows() {
        let p = parse(
            b"DWM_LUA 1 800 600 1\r\nRECT 1 0 0 800 600\r\nANIMATION 120 ease_out\r\nEND\r\n",
            800,
            600,
            1,
        )
        .unwrap();
        assert_eq!(
            p.rects,
            [Rect {
                x: 0,
                y: 0,
                w: 800,
                h: 600
            }]
        );
        assert_eq!((p.duration_ms, p.ease_out), (120, true));
        assert!(parse(
            b"DWM_LUA 1 800 600 0\nANIMATION 0 linear\nEND\n",
            800,
            600,
            0
        )
        .is_ok());
    }

    #[test]
    fn rejects_partial_trailing_wrong_geometry_and_duplicate_indices() {
        for text in [
            "DWM_LUA 1 800 600 1\nRECT 1 0 0 800 600\nANIMATION 0 linear\nEND",
            "DWM_LUA 1 800 600 1\nRECT 1 0 0 800 600\nANIMATION 0 linear\nEND\nnoise\n",
            "DWM_LUA 1 801 600 1\nRECT 1 0 0 800 600\nANIMATION 0 linear\nEND\n",
            "DWM_LUA 1 800 600 1\nRECT 0 0 0 800 600\nANIMATION 0 linear\nEND\n",
            "DWM_LUA 1 800 600 1\nRECT 1 -1 0 800 600\nANIMATION 0 linear\nEND\n",
            "DWM_LUA 1 800 600 1\nRECT 1 0 0 0 600\nANIMATION 0 linear\nEND\n",
            "DWM_LUA 1 800 600 1\nRECT 1 1 0 800 600\nANIMATION 0 linear\nEND\n",
            "DWM_LUA 1 800 600 1\nRECT 1 1 0 2147483647 600\nANIMATION 0 linear\nEND\n",
        ] {
            assert!(parse(text.as_bytes(), 800, 600, 1).is_err(), "{text}");
        }
        assert!(parse(b"DWM_LUA 1 800 600 2\nRECT 1 0 0 400 600\nRECT 1 400 0 400 600\nANIMATION 0 linear\nEND\n", 800, 600, 2).is_err());
    }

    #[test]
    fn rejects_oversized_output() {
        assert!(parse(&alloc::vec![b' '; MAX_OUTPUT + 1], 800, 600, 1).is_err());
    }

    #[test]
    fn enforces_runner_contract_limits() {
        let max = b"DWM_LUA 1 16384 16384 1\nRECT 1 0 0 16384 16384\nANIMATION 2000 linear\nEND\n";
        assert!(parse(max, 16384, 16384, 1).is_ok());
        let over_duration = b"DWM_LUA 1 800 600 0\nANIMATION 2001 linear\nEND\n";
        assert!(parse(over_duration, 800, 600, 0).is_err());
        for (width, height) in [(16385, 600), (800, 16385), (0, 600), (800, 0)] {
            let text = alloc::format!("DWM_LUA 1 {width} {height} 0\nANIMATION 0 linear\nEND\n");
            assert!(parse(text.as_bytes(), width, height, 0).is_err());
        }
        let mut text = alloc::format!("DWM_LUA 1 800 600 64\n");
        for index in 1..=64 {
            text.push_str(&alloc::format!("RECT {index} 0 0 1 1\n"));
        }
        text.push_str("ANIMATION 0 linear\nEND\n");
        assert!(parse(text.as_bytes(), 800, 600, 64).is_ok());
        let text = text
            .replacen("600 64", "600 65", 1)
            .replace("ANIMATION", "RECT 65 0 0 1 1\nANIMATION");
        assert!(parse(text.as_bytes(), 800, 600, 65).is_err());
    }
}
