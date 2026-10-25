//! Bounded text protocol and asynchronous process boundary for Lua layout policy.
extern crate alloc;

use alloc::{string::String, vec::Vec};

const MAX_OUTPUT: usize = 16 * 1024;
const MAX_DIMENSION: u32 = 16_384;
const MAX_WINDOWS: usize = 64;
const MAX_DURATION_MS: u32 = 2000;

/// Records a submitted topology before attempting runner startup. Failures do
/// not clear it: only a changed input or explicit settings reload retries.
pub struct Requests {
    snapshot: Option<(Vec<u32>, u32, u32, usize)>,
}

impl Requests {
    pub const fn new() -> Self {
        Self { snapshot: None }
    }
    pub fn reset(&mut self) {
        self.snapshot = None;
    }
    pub fn begin(&mut self, ids: &[u32], width: u32, height: u32, workspace: usize) -> bool {
        if self.snapshot.as_ref().is_some_and(|(old, w, h, ws)| {
            old.as_slice() == ids && *w == width && *h == height && *ws == workspace
        }) {
            return false;
        }
        self.snapshot = Some((ids.to_vec(), width, height, workspace));
        true
    }
    pub fn ids(&self) -> &[u32] {
        self.snapshot
            .as_ref()
            .map_or(&[], |(ids, _, _, _)| ids.as_slice())
    }
}

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

/// Events are policies evaluated between topology changes, never per render frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Event {
    pub name: &'static str,
    pub window_id: u32,
    pub target: u32,
}
impl Event {
    pub fn valid(self) -> bool {
        matches!(
            self.name,
            "window_open"
                | "window_close"
                | "window_focus"
                | "window_minimize"
                | "window_restore"
                | "workspace_change"
                | "panel_click"
                | "reload"
                | "window_maximize"
                | "panel_hover"
                | "launcher_open"
                | "launcher_close"
                | "widget_update"
                | "widget_click"
                | "theme_change"
        ) && self.window_id <= i32::MAX as u32
            && self.target <= i32::MAX as u32
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    Fade,
    Slide,
    Scale,
    Spring,
    Genie,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Property {
    Opacity,
    X,
    Y,
    Width,
    Height,
    Scale,
    TranslationX,
    TranslationY,
    Blur,
    Shadow,
    CornerRadius,
    Color,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Animation {
    pub window_id: u32,
    pub target: u32,
    pub effect: Effect,
    pub duration_ms: u32,
    pub property: Option<Property>,
    pub from: i64,
    pub to: i64,
    pub curve: [i32; 33],
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WidgetUpdate {
    pub target: u32,
    pub text: String,
    pub color: Option<u32>,
}
fn widget_record(line: &str) -> Result<WidgetUpdate, &'static str> {
    let mut fields = line.split_ascii_whitespace();
    if fields.next() != Some("WIDGET") {
        return Err("invalid widget record");
    }
    let target = number::<u32>(fields.next())?;
    let color = number::<i64>(fields.next())?;
    if !(1..=512).contains(&target) || !(-1..=u32::MAX as i64).contains(&color) {
        return Err("invalid widget target or color");
    }
    let hex = fields.next().ok_or("missing widget text")?;
    if fields.next().is_some() {
        return Err("trailing widget fields");
    }
    let mut bytes = Vec::new();
    if hex != "-" {
        if hex.is_empty() || hex.len() > 512 || hex.len() % 2 != 0 {
            return Err("invalid widget text encoding");
        }
        for pair in hex.as_bytes().chunks_exact(2) {
            let digit = |byte: u8| -> Result<u8, &'static str> {
                match byte {
                    b'0'..=b'9' => Ok(byte - b'0'),
                    b'a'..=b'f' => Ok(byte - b'a' + 10),
                    _ => Err("invalid hex digit"),
                }
            };
            let byte = digit(pair[0])? * 16 + digit(pair[1])?;
            if byte == 0 {
                return Err("NUL in widget text");
            }
            bytes.push(byte);
        }
    }
    let text = String::from_utf8(bytes).map_err(|_| "invalid widget UTF8")?;
    Ok(WidgetUpdate {
        target,
        text,
        color: if color == -1 {
            None
        } else {
            Some(color as u32)
        },
    })
}

fn event_bit(name:&str)->u32{
    ["window_open","window_close","window_focus","window_minimize","window_restore","workspace_change","panel_click","reload","window_maximize","panel_hover","launcher_open","launcher_close","widget_update","widget_click","theme_change"].iter().position(|&event|event==name).map_or(0,|i|1<<i)
}
struct EventResponse {
    animations: Vec<Animation>,
    commands: Vec<String>,
    widgets: Vec<WidgetUpdate>,
    subscriptions:Option<u32>,
}
#[cfg(test)]
fn parse_event(output: &[u8], event: Event) -> Result<Vec<Animation>, &'static str> {
    parse_event_response(output, event).map(|response| response.animations)
}
fn valid_command(action: &str) -> bool {
    if action.is_empty() || action.len() > 128 {
        return false;
    }
    match action {
        "launcher" | "close" | "maximize" | "minimize" | "retile" => true,
        _ => {
            action.strip_prefix("launch:").is_some_and(|id| {
                !id.is_empty()
                    && id
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
            }) || action.strip_prefix("workspace:").is_some_and(|number| {
                !number.is_empty() && number.bytes().all(|b| b.is_ascii_digit())
            })
        }
    }
}
fn parse_event_response(output: &[u8], event: Event) -> Result<EventResponse, &'static str> {
    if output.len() > MAX_OUTPUT || !event.valid() {
        return Err("invalid event response size");
    }
    let text = core::str::from_utf8(output).map_err(|_| "non-UTF8 output")?;
    if !text.ends_with('\n') {
        return Err("incomplete event response");
    }
    let mut lines = text.lines();
    let mut header = lines
        .next()
        .ok_or("missing event header")?
        .split_ascii_whitespace();
    if header.next() != Some("DWM_EVENT")
        || header.next() != Some("2")
        || header.next() != Some(event.name)
        || number::<u32>(header.next())? != event.window_id
    {
        return Err("unexpected event response");
    }
    let count = number::<usize>(header.next())?;
    if count > 32 || header.next().is_some() {
        return Err("invalid action count");
    }
    let mut result = Vec::new();
    for _ in 0..count {
        let mut fields = lines
            .next()
            .ok_or("missing animation action")?
            .split_ascii_whitespace();
        if fields.next() != Some("ANIMATE") {
            return Err("unexpected event action");
        }
        let window_id = number::<u32>(fields.next())?;
        let target = number::<u32>(fields.next())?;
        if window_id > i32::MAX as u32 || target > i32::MAX as u32 {
            return Err("invalid action target");
        }
        let effect = match fields.next() {
            Some("fade") => Effect::Fade,
            Some("slide") => Effect::Slide,
            Some("scale") => Effect::Scale,
            Some("spring") => Effect::Spring,
            Some("genie") => Effect::Genie,
            _ => return Err("unsupported effect"),
        };
        let duration_ms = number::<u32>(fields.next())?;
        if duration_ms > MAX_DURATION_MS {
            return Err("invalid action duration");
        }
        let property = match fields.next() {
            Some("none") => None,
            Some("opacity") => Some(Property::Opacity),
            Some("x") => Some(Property::X),
            Some("y") => Some(Property::Y),
            Some("width") => Some(Property::Width),
            Some("height") => Some(Property::Height),
            Some("scale") => Some(Property::Scale),
            Some("translation_x") => Some(Property::TranslationX),
            Some("translation_y") => Some(Property::TranslationY),
            Some("blur") => Some(Property::Blur),
            Some("shadow") => Some(Property::Shadow),
            Some("corner_radius") => Some(Property::CornerRadius),
            Some("color") => Some(Property::Color),
            _ => return Err("unsupported animation property"),
        };
        let from = number::<i64>(fields.next())?;
        let to = number::<i64>(fields.next())?;
        let (minimum, maximum) = match property {
            Some(Property::Opacity) => (0, 65535),
            Some(Property::Scale) => (0, 16 * 65535),
            Some(Property::X | Property::Y | Property::TranslationX | Property::TranslationY) => {
                (-16384, 16384)
            }
            Some(Property::Width | Property::Height) => (1, 16384),
            Some(Property::Blur | Property::Shadow | Property::CornerRadius) => (0, 1024),
            Some(Property::Color) => (0, u32::MAX as i64),
            None => (0, 0),
        };
        if !(minimum..=maximum).contains(&from) || !(minimum..=maximum).contains(&to) {
            return Err("invalid scalar endpoints");
        }
        if property.is_some() && effect != Effect::Fade {
            return Err("invalid scalar effect");
        }
        if window_id == 0 && (property.is_none() || !(1..=2048).contains(&target)) {
            return Err("component animation requires scalar property and target");
        }
        let mut curve = [0; 33];
        for sample in &mut curve {
            *sample = number(fields.next())?;
        }
        if curve[0] != 0
            || curve[32] != 65535
            || curve.iter().any(|v| *v < -65535 || *v > 131070)
            || fields.next().is_some()
        {
            return Err("invalid action curve");
        }
        result.push(Animation {
            window_id,
            target,
            effect,
            duration_ms,
            curve,
            property,
            from,
            to,
        });
    }
    let mut commands = Vec::new();
    let mut widgets = Vec::new();
    let mut subscriptions=None;
    loop {
        match lines.next() {
            Some("END") => break,
            Some(line) if line.starts_with("SUBSCRIPTIONS ")=>{
                if subscriptions.is_some(){return Err("duplicate subscriptions");}
                let value=line[14..].parse::<u32>().map_err(|_|"invalid subscriptions")?;
                if value>=1<<15{return Err("invalid subscriptions");}subscriptions=Some(value);
            }
            Some(line) if line.starts_with("WIDGET ") => {
                if widgets.len() >= 16 {
                    return Err("too many widget updates");
                }
                widgets.push(widget_record(line)?);
            }
            Some(line) => {
                let action = line
                    .strip_prefix("COMMAND ")
                    .ok_or("unexpected event record")?;
                if commands.len() >= 32 || !valid_command(action) {
                    return Err("invalid desktop command");
                }
                commands.push(String::from(action));
            }
            None => return Err("incomplete event response"),
        }
    }
    if lines.next().is_some() {
        return Err("trailing event response");
    }
    Ok(EventResponse {
        animations: result,
        commands,
        widgets,
        subscriptions,
    })
}

fn coalescible(event: &Event) -> bool {
    matches!(event.name, "widget_update" | "panel_hover")
}
fn enqueue(pending: &mut Vec<Event>, event: Event) -> Result<(), &'static str> {
    if !event.valid() {
        return Err("invalid Lua event");
    }
    if coalescible(&event) {
        if let Some(old) = pending
            .iter_mut()
            .find(|old| old.name == event.name && old.target == event.target)
        {
            *old = event;
            return Ok(());
        }
    } else if pending.last().is_some_and(|old| *old == event) {
        return Ok(());
    }
    if pending.len() >= 32 {
        if !coalescible(&event) {
            if let Some(index) = pending.iter().position(coalescible) {
                pending.remove(index);
            } else {
                return Err("Lua event queue full");
            }
        } else {
            return Err("Lua event queue full");
        }
    }
    if coalescible(&event) {
        pending.push(event);
    } else {
        let index = pending
            .iter()
            .position(coalescible)
            .unwrap_or(pending.len());
        pending.insert(index, event);
    }
    Ok(())
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

    enum RawPoll {
        Pending,
        Ready(Vec<u8>, u32, u32, usize),
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
            self.start_request(config, config.layout.as_str(), width, height, count, &[])
        }

        fn start_request(
            &mut self,
            config: &settings::Scripts,
            path: &str,
            width: u32,
            height: u32,
            count: usize,
            extra: &[&str],
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
            let script = format!("DWM_LUA_SCRIPT={}", path);
            let width_env = format!("DWM_LUA_WIDTH={}", width);
            let height_env = format!("DWM_LUA_HEIGHT={}", height);
            let count_env = format!("DWM_LUA_WINDOWS={}", count);
            let mut env: Vec<&str> = alloc::vec![&script, &width_env, &height_env, &count_env];
            env.extend_from_slice(extra);
            let pid = libdunit::pty_spawn_env(config.runner.as_str(), pty as u32, &env);
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

        fn fail(&mut self, reason: &'static str) -> RawPoll {
            self.cancel();
            RawPoll::Failed(reason)
        }

        pub fn poll(&mut self) -> Poll {
            match self.poll_raw() {
                RawPoll::Pending => Poll::Pending,
                RawPoll::Failed(reason) => Poll::Failed(reason),
                RawPoll::Ready(output, w, h, count) => match parse(&output, w, h, count) {
                    Ok(policy) => Poll::Ready(policy),
                    Err(reason) => Poll::Failed(reason),
                },
            }
        }
        fn poll_raw(&mut self) -> RawPoll {
            let Some(_) = self.child.as_ref() else {
                return RawPoll::Pending;
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
                let result = RawPoll::Ready(
                    core::mem::take(&mut child.output),
                    child.width,
                    child.height,
                    child.count,
                );
                self.cancel();
                return result;
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
            RawPoll::Pending
        }
    }

    pub enum EventPoll {
        Pending,
        Ready(Vec<Animation>, Vec<String>, Vec<WidgetUpdate>),
        Failed(&'static str),
    }
    /// Bounded FIFO; repeated pending events with the same kind/window coalesce.
    /// Overflow is reported so callers can retain a native policy fallback.
    pub struct EventEngine {
        engine: Engine,
        active: Option<Event>,
        pending: Vec<Event>,
        subscriptions:Option<u32>,
        component_ids:Vec<String>,
        component_panels:Vec<u32>,
        app_ids:Vec<String>,
    }
    impl EventEngine {
        pub const fn new() -> Self {
            Self {
                engine: Engine::new(),
                active: None,
                pending: Vec::new(),
                subscriptions:None,
                component_ids:Vec::new(),component_panels:Vec::new(),app_ids:Vec::new(),
            }
        }
        pub fn set_context(&mut self,shell:&settings::shell::ShellConfig,apps:&settings::Applications){
            self.component_ids=shell.components.iter().map(|c|c.id.clone()).collect();self.app_ids=apps.apps.iter().map(|a|a.id.clone()).collect();
            self.component_panels=alloc::vec![0;shell.components.len()];
            for(i,panel)in shell.panels.iter().enumerate(){let mut pending=panel.children.clone();while let Some(id)=pending.pop(){if let Some(index)=shell.components.iter().position(|c|c.id==id){self.component_panels[index]=1024+i as u32;pending.extend(shell.components[index].children.iter().cloned());}}}
        }
        pub fn cancel(&mut self) {
            self.engine.cancel();
            self.active = None;
            self.pending.clear();
            self.subscriptions=None;
        }
        pub fn submit(&mut self, event: Event) -> Result<(), &'static str> {
            if self.subscriptions.is_some_and(|mask|mask&event_bit(event.name)==0){return Ok(());}enqueue(&mut self.pending,event)
        }

        fn fail(&mut self, reason: &'static str) -> EventPoll {
            self.engine.cancel();
            self.active = None;
            self.subscriptions=None;
            EventPoll::Failed(reason)
        }
        pub fn poll(&mut self, config: &settings::Scripts, path: &str) -> EventPoll {
            if !config.enabled || path.is_empty() {
                self.cancel();
                return EventPoll::Pending;
            }
            if self.engine.child.is_none() {
                if self.pending.is_empty() {
                    return EventPoll::Pending;
                }
                if let Err(reason) =
                    self.engine
                        .start_request(config, path, 1, 1, 0, &["DWM_LUA_DAEMON=1","DWM_LUA_SUBSCRIPTIONS=1"])
                {
                    self.pending.remove(0);
                    return self.fail(reason);
                }
            }
            let (now, hz) = match clock() {
                Ok(clock) => clock,
                Err(reason) => return self.fail(reason),
            };
            let child = self.engine.child.as_ref().unwrap();
            let mut status = libdunit::WaitStatus::empty();
            let waited = libdunit::wait(child.pid, &mut status);
            if waited > 0 {
                self.engine.child.as_mut().unwrap().exited = true;
                return self.fail("Lua event worker exited");
            }
            if waited != libdunit::EAGAIN {
                return self.fail("Lua event worker wait failed");
            }
            if self.active.is_none() {
                if self.pending.is_empty() {
                    return EventPoll::Pending;
                }
                let event = self.pending.remove(0);
                let ids=if event.window_id==0{&self.component_ids}else{&self.app_ids};
                let identity=event.target.checked_sub(1).and_then(|i|ids.get(i as usize));
                let input=if let Some(id)=identity{if event.window_id==0{let panel=self.component_panels.get(event.target as usize-1).copied().unwrap_or(0);if panel!=0{format!("EVENT {} {} {} {} {}\n",event.name,event.window_id,event.target,id,panel)}else{format!("EVENT {} {} {} {}\n",event.name,event.window_id,event.target,id)}}else{format!("EVENT {} {} {} {}\n",event.name,event.window_id,event.target,id)}}else{format!("EVENT {} {} {}\n",event.name,event.window_id,event.target)};
                let child = self.engine.child.as_mut().unwrap();
                if libdunit::pty_write(child.pty, input.as_bytes()) != input.len() as isize {
                    return self.fail("Lua event worker request write failed");
                }
                let ticks = (config.timeout_ms.clamp(50, 5000) as u64)
                    .saturating_mul(hz)
                    .saturating_add(999)
                    / 1000;
                child.deadline = now.saturating_add(ticks.max(1));
                self.active = Some(event);
            }
            let child = self.engine.child.as_mut().unwrap();
            // A worker can emit only one batch for the sole outstanding request.
            // Bound both bytes retained and syscalls spent in a compositor frame.
            for _ in 0..4 {
                let mut buffer = [0u8; 1024];
                let read = libdunit::pty_read(child.pty, &mut buffer);
                if read == 0 {
                    break;
                }
                if read < 0 {
                    return self.fail("Lua event worker output read failed");
                }
                let read = read as usize;
                if read > buffer.len() || child.output.len().saturating_add(read) > MAX_OUTPUT {
                    return self.fail("Lua event worker output exceeds limit");
                }
                child
                    .output
                    .extend(buffer[..read].iter().copied().filter(|b| *b != b'\r'));
            }
            if child.output.ends_with(b"\nEND\n") {
                let output = core::mem::take(&mut child.output);
                let event = self.active.take().unwrap();
                return match parse_event_response(&output, event) {
                    Ok(response) => {
                        if let Some(mask)=response.subscriptions{self.subscriptions=Some(mask);self.pending.retain(|event|mask&event_bit(event.name)!=0);}
                        EventPoll::Ready(response.animations, response.commands, response.widgets)
                    }
                    Err(reason) => self.fail(reason),
                };
            }
            if now >= child.deadline {
                return self.fail("Lua event worker timed out");
            }
            EventPoll::Pending
        }
    }

    pub enum PreflightPoll {
        Pending,
        Ready,
        Failed(&'static str),
    }
    /// Candidate script workers are separate from committed desktop workers.
    /// No returned commands or animation descriptors are executed by this probe.
    pub struct Preflight {
        layout: Engine,
        events: EventEngine,
        config: settings::Scripts,
        layout_done: bool,
        events_done: bool,
    }
    impl Preflight {
        pub fn new(
            config: &settings::Scripts,
            width: u32,
            height: u32,
            count: usize,
        ) -> Result<Self, &'static str> {
            let mut probe = Self {
                layout: Engine::new(),
                events: EventEngine::new(),
                config: *config,
                layout_done: !config.enabled,
                events_done: !config.enabled || config.events.as_str().is_empty(),
            };
            if !probe.layout_done {
                probe.layout.start(config, width, height, count)?;
            }
            if !probe.events_done {
                probe.events.submit(Event {
                    name: "reload",
                    window_id: 0,
                    target: 0,
                })?;
            }
            Ok(probe)
        }
        pub fn poll(&mut self) -> PreflightPoll {
            if !self.layout_done {
                match self.layout.poll() {
                    Poll::Pending => (),
                    Poll::Ready(_) => self.layout_done = true,
                    Poll::Failed(reason) => {
                        self.events.cancel();
                        return PreflightPoll::Failed(reason);
                    }
                }
            }
            if !self.events_done {
                match self.events.poll(&self.config, self.config.events.as_str()) {
                    EventPoll::Pending => (),
                    EventPoll::Ready(_, _, _) => {
                        self.events_done = true;
                        self.events.cancel();
                    }
                    EventPoll::Failed(reason) => {
                        self.layout.cancel();
                        return PreflightPoll::Failed(reason);
                    }
                }
            }
            if self.layout_done && self.events_done {
                PreflightPoll::Ready
            } else {
                PreflightPoll::Pending
            }
        }
    }

    impl Drop for Engine {
        fn drop(&mut self) {
            self.cancel();
        }
    }
}

#[cfg(not(test))]
pub use runtime::{Engine, EventEngine, EventPoll, Poll, Preflight, PreflightPoll};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subscription_mask_is_bounded_and_has_unambiguous_event_bits(){
        assert_eq!(event_bit("reload"),128);assert_eq!(event_bit("widget_update"),4096);assert_eq!(event_bit("unknown"),0);
        let event=Event{name:"reload",window_id:0,target:0};
        let response=parse_event_response(b"DWM_EVENT 2 reload 0 0\nSUBSCRIPTIONS 128\nEND\n",event).unwrap();assert_eq!(response.subscriptions,Some(128));
        assert!(parse_event_response(b"DWM_EVENT 2 reload 0 0\nSUBSCRIPTIONS 32768\nEND\n",event).is_err());
        assert!(parse_event_response(b"DWM_EVENT 2 reload 0 0\nSUBSCRIPTIONS 1\nSUBSCRIPTIONS 2\nEND\n",event).is_err());
    }
    #[test]
    fn event_protocol_is_atomic_and_checks_curves_and_targets() {
        let event = Event {
            name: "window_minimize",
            window_id: 7,
            target: 9,
        };
        let samples = (0..33)
            .map(|i| alloc::format!(" {}", ((i * 65535 + 16) / 32)))
            .collect::<alloc::string::String>();
        let text = alloc::format!(
            "DWM_EVENT 2 window_minimize 7 1\nANIMATE 7 9 genie 420 none 0 0{samples}\nEND\n"
        );
        let actions = parse_event(text.as_bytes(), event).unwrap();
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].effect, Effect::Genie);
        assert_eq!(actions[0].curve[32], 65535);
        // Overshooting spring curves remain valid native descriptors.
        assert!(parse_event(text.replace("32768", "80000").as_bytes(), event).is_ok());
        let scalar = text.replace("genie 420 none 0 0", "fade 250 opacity 16384 65535");
        assert_eq!(
            parse_event(scalar.as_bytes(), event).unwrap()[0].property,
            Some(Property::Opacity)
        );
        assert!(parse_event(
            scalar.replace("opacity 16384", "opacity -1").as_bytes(),
            event
        )
        .is_err());
        for broken in [
            text.replace(" 7 1", " 8 1"),
            text.replace("genie", "unknown"),
            text.replace("420", "2001"),
            text.replace("65535", "131071"),
            text.replace(" 0 ", " 1 "),
            text.replace("END\n", "END\nnoise\n"),
            text.replace("ANIMATE 7", "ANIMATE 0"),
            text.replace("END\n", ""),
        ] {
            assert!(parse_event(broken.as_bytes(), event).is_err(), "{broken}");
        }
        assert!(parse_event(
            b"DWM_EVENT 2 reload 0 0\nEND\n",
            Event {
                name: "reload",
                window_id: 0,
                target: 0
            }
        )
        .is_ok());
    }

    #[test]
    fn native_scalar_properties_enforce_per_property_ranges() {
        let event = Event {
            name: "window_open",
            window_id: 7,
            target: 9,
        };
        let curve = (0..33)
            .map(|i| alloc::format!(" {}", (i * 65535 + 16) / 32))
            .collect::<alloc::string::String>();
        for (name, minimum, maximum) in [
            ("opacity", 0i64, 65535i64),
            ("scale", 0, 1048560),
            ("x", -16384, 16384),
            ("y", -16384, 16384),
            ("translation_x", -16384, 16384),
            ("translation_y", -16384, 16384),
            ("width", 1, 16384),
            ("height", 1, 16384),
            ("blur", 0, 1024),
            ("shadow", 0, 1024),
            ("corner_radius", 0, 1024),
            ("color", 0, 4294967295),
        ] {
            let valid = alloc::format!("DWM_EVENT 2 window_open 7 1\nANIMATE 7 9 fade 200 {name} {minimum} {maximum}{curve}\nEND\n");
            let action = parse_event(valid.as_bytes(), event).unwrap().remove(0);
            assert_eq!((action.from, action.to), (minimum, maximum));
            for invalid in [minimum - 1, maximum + 1] {
                let text = alloc::format!("DWM_EVENT 2 window_open 7 1\nANIMATE 7 9 fade 200 {name} {invalid} {maximum}{curve}\nEND\n");
                assert!(
                    parse_event(text.as_bytes(), event).is_err(),
                    "{name} {invalid}"
                );
            }
        }
    }

    #[test]
    fn desktop_commands_are_bounded_allowlisted_and_atomic() {
        let event = Event {
            name: "reload",
            window_id: 0,
            target: 0,
        };
        let output = b"DWM_EVENT 2 reload 0 0\nCOMMAND launcher\nCOMMAND launch:terminal\nCOMMAND workspace:2\nEND\n";
        let response = parse_event_response(output, event).unwrap();
        assert!(response.animations.is_empty());
        assert_eq!(
            response.commands,
            ["launcher", "launch:terminal", "workspace:2"]
        );
        for action in [
            "exec:/app/terminal",
            "launch:../evil",
            "launch:",
            "launch:terminal;poweroff",
            "workspace:-1",
            "launcher extra",
        ] {
            let text =
                alloc::format!("DWM_EVENT 2 reload 0 0\nCOMMAND launcher\nCOMMAND {action}\nEND\n");
            assert!(
                parse_event_response(text.as_bytes(), event).is_err(),
                "{action}"
            );
        }
        let text = alloc::format!(
            "DWM_EVENT 2 reload 0 0\n{}END\n",
            "COMMAND launcher\n".repeat(33)
        );
        assert!(parse_event_response(text.as_bytes(), event).is_err());
    }

    #[test]
    fn widget_updates_decode_utf8_atomically_and_validate_targets() {
        let event = Event {
            name: "widget_update",
            window_id: 0,
            target: 9,
        };
        let text = "DWM_EVENT 2 widget_update 0 0\nWIDGET 9 4289379276 435055203432250ae29c93\nWIDGET 9 -1 -\nEND\n";
        let response = parse_event_response(text.as_bytes(), event).unwrap();
        assert_eq!(
            response.widgets[0],
            WidgetUpdate {
                target: 9,
                color: Some(0xffaabbcc),
                text: String::from("CPU 42%\n✓")
            }
        );
        assert_eq!(response.widgets[1].text, "");
        assert_eq!(response.widgets[1].color, None);
        for record in [
            "WIDGET 0 -1 61",
            "WIDGET 513 -1 61",
            "WIDGET 1 -2 61",
            "WIDGET 1 4294967296 61",
            "WIDGET 1 -1 00",
            "WIDGET 1 -1 c080",
            "WIDGET 1 -1 eda080",
            "WIDGET 1 -1 f4908080",
            "WIDGET 1 -1 z0",
            "WIDGET 1 -1 1",
        ] {
            let bad =
                alloc::format!("DWM_EVENT 2 widget_update 0 0\nWIDGET 9 -1 61\n{record}\nEND\n");
            assert!(
                parse_event_response(bad.as_bytes(), event).is_err(),
                "{record}"
            );
        }
    }
    #[test]
    fn lifecycle_events_evict_and_precede_coalescible_updates() {
        let mut pending = Vec::new();
        for target in 1..=32 {
            enqueue(
                &mut pending,
                Event {
                    name: "widget_update",
                    window_id: 0,
                    target,
                },
            )
            .unwrap();
        }
        enqueue(
            &mut pending,
            Event {
                name: "widget_update",
                window_id: 0,
                target: 4,
            },
        )
        .unwrap();
        assert_eq!(pending.len(), 32);
        let close = Event {
            name: "window_close",
            window_id: 42,
            target: 0,
        };
        enqueue(&mut pending, close).unwrap();
        assert_eq!(pending.len(), 32);
        assert_eq!(pending[0], close);
        let restore = Event {
            name: "window_restore",
            window_id: 42,
            target: 0,
        };
        enqueue(&mut pending, restore).unwrap();
        assert_eq!(pending[1], restore);
    }

    #[test]
    fn component_scalar_animation_requires_valid_target_and_property() {
        let event = Event {
            name: "panel_hover",
            window_id: 0,
            target: 9,
        };
        let curve = (0..33)
            .map(|i| alloc::format!(" {}", (i * 65535 + 16) / 32))
            .collect::<alloc::string::String>();
        let text = alloc::format!(
            "DWM_EVENT 2 panel_hover 0 1\nANIMATE 0 9 fade 140 opacity 0 32768{curve}\nEND\n"
        );
        let action = parse_event(text.as_bytes(), event).unwrap().remove(0);
        assert_eq!((action.window_id, action.target), (0, 9));
        for target in [513, 514, 515, 1024, 1087] {
            let virtual_action = text.replace("0 9 fade", &alloc::format!("0 {target} fade"));
            assert_eq!(parse_event(virtual_action.as_bytes(), event).unwrap()[0].target, target);
        }
        for bad in [
            text.replace("0 9 fade", "0 0 fade"),
            text.replace("0 9 fade", "0 2049 fade"),
            text.replace("opacity 0 32768", "none 0 0"),
            text.replace("fade", "genie"),
        ] {
            assert!(parse_event(bad.as_bytes(), event).is_err());
        }
    }

    #[test]
    fn failed_requests_retry_only_after_changed_input_or_reload() {
        let mut requests = Requests::new();
        assert!(requests.begin(&[10, 20], 800, 600, 0));
        // Startup failure, timeout, or rejected output leaves the recorded
        // request intact. Repeated frames must not restart or notify again.
        for _ in 0..1000 {
            assert!(!requests.begin(&[10, 20], 800, 600, 0));
        }
        assert_eq!(requests.ids(), &[10, 20]);
        assert!(requests.begin(&[10, 20], 801, 600, 0));
        assert!(requests.begin(&[10, 20], 801, 601, 0));
        assert!(requests.begin(&[10, 20], 801, 601, 1));
        assert!(requests.begin(&[20, 10], 801, 601, 1));
        assert!(requests.begin(&[20], 801, 601, 1));
        requests.reset();
        assert!(requests.begin(&[20], 801, 601, 1));
        assert!(requests.begin(&[], 801, 601, 1));
        assert!(!requests.begin(&[], 801, 601, 1));
    }

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
