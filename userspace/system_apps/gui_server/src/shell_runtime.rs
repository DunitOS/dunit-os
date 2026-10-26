//! Retained, configurable shell. Parsing, style resolution and layout happen at
//! configuration boundaries; painting and input share the cached scene.
use super::*;
use alloc::string::String;
use dunit_style::{cascade::NodeStyle, Cascade, Style};
use dunit_ui::{
    attrs::{Attrs, Edges, Sizing},
    tree::{Kind as UiKind, NodeId, TreeBuilder},
};
use settings::shell::{Align, AppsSource, ComponentKind as Kind, Direction, ShellConfig};

#[path = "shell_geometry.rs"]
mod geometry;
use geometry::{group_cell, item_rect, Rect};
#[path = "shell_text.rs"]
mod text_cache;
use text_cache::{text, GlyphAtlas};
#[path="shell_resources.rs"]
mod resources;
#[path="shell_layout.rs"]
mod layout;
use layout::layout_panels;

#[derive(Clone)]
pub enum Action {
    Launcher(Vec<usize>, Rect, Edge),
    Workspace(usize),
    QuickSettings(Rect, Edge),
    App(usize),
    Group(usize, Rect),
    Command(String),
}
struct Cell {
    component: usize,
    horizontal: bool,
    gap:i32,
    rect: Rect,
    base_rect: Rect,
    parent:Option<usize>,
    normal: Style,
    hover: Style,
    focused: Style,
    transitions:[Vec<dunit_style::Transition>;3],
}
struct Panel {
    index: usize,
    base_rect:Rect,
    opacity:u32,
    rect: Rect,
    cells: Vec<Cell>,
    style: Style,
}
pub struct Shell {
    pub revision:u64,
    pub config: ShellConfig,
    panels: Vec<Panel>,
    pub insets: Insets,
    popup: Option<(usize, Rect)>,
    stats: libdunit::SystemStats,
    sampled_at: u64,
    app_indices: Vec<Vec<usize>>,
    text_cache: Vec<String>,
    text_sampled_at: Vec<u64>,
    widget_sampled_at: Vec<u64>,
    stats_interval: u64,
    glyphs: GlyphAtlas,
    workspace_labels: Vec<String>,
    cascade: Cascade,
    viewport: (usize, usize),
    component_icons: Vec<Option<dunit_image::Image>>,
    running_apps: Vec<usize>,
    running_scratch: Vec<usize>,
    motion:crate::shell_motion::Tracks,
    animation_backing:Vec<u32>,
    animation_now:u64,
    motion_rects:Vec<Rect>,
    motion_opacity:Vec<u32>,
    style_states:Vec<u8>,
}

fn color(style: &Style, key: &str, fallback: u32) -> u32 {
    style
        .color(key)
        .map(|c| (c.a as u32) << 24 | (c.r as u32) << 16 | (c.g as u32) << 8 | c.b as u32)
        .unwrap_or(fallback)
}
fn number(style: &Style, key: &str, fallback: i32) -> i32 {
    style
        .number(key)
        .filter(|n| n.is_finite())
        .map(|n| n.clamp(0.0, 16384.0) as i32)
        .unwrap_or(fallback)
}
fn style(cascade: &Cascade, tag: &str, id: &str, classes: &str, state: Option<&str>) -> Style {
    let classes: Vec<&str> = classes.split_whitespace().collect();
    let states = state.map(|s| [s]);
    cascade.resolve(&NodeStyle {
        element: tag,
        id: Some(id),
        classes: &classes,
        states: states.as_ref().map_or(&[][..], |s| &s[..]),
    })
}
fn ui_align(a: Align) -> dunit_ui::Align {
    match a {
        Align::Start => dunit_ui::Align::Start,
        Align::Center => dunit_ui::Align::Center,
        Align::End => dunit_ui::Align::End,
    }
}
fn entries(c: &settings::shell::ComponentConfig, apps: &Applications) -> Vec<usize> {
    c.entries
        .iter()
        .filter_map(|id| apps.apps.iter().position(|a| a.id == *id))
        .collect()
}
fn prepare_atlas(
    config: &ShellConfig,
    panels: &[Panel],
    font: Option<&Font>,
    apps: &Applications,
    ly: &Layout,
) -> Result<GlyphAtlas, String> {
    let mut atlas = GlyphAtlas::default();
    let Some(font) = font else {
        return Ok(atlas);
    };
    for panel in panels {
        for cell in &panel.cells {
            let c = &config.components[cell.component];
            for style in [&cell.normal, &cell.hover, &cell.focused] {
                let px = number(
                    style,
                    "font-size",
                    if c.font_px == 0 {
                        ly.panel_font_px
                    } else {
                        c.font_px as i32
                    },
                )
                .clamp(6, 128);
                atlas.add(font, px, "?DunitWindowSettings0123456789: %RAMproc")?;
                atlas.add(font, px, &c.label)?;
                atlas.add(font, px, &c.format)?;
                if matches!(
                    c.kind,
                    Kind::Apps | Kind::AppGroup | Kind::Launcher | Kind::Title
                ) {
                    for app in &apps.apps {
                        atlas.add(font, px, &app.label)?;
                        atlas.add(font, px, &app.name)?;
                    }
                }
            }
        }
    }
    // Group popups inherit the general panel text size.
    let px = ly.panel_font_px.clamp(6, 128);
    atlas.add(font, px, "?")?;
    for app in &apps.apps {
        atlas.add(font, px, &app.label)?;
    }
    atlas.finish();
    Ok(atlas)
}
impl Shell {

    fn refresh_running(&mut self, wins: &[Win], apps: &Applications) {
        self.running_scratch.clear();
        for win in wins
            .iter()
            .filter(|w| w.alive && (w.app as usize) < apps.apps.len())
        {
            let app = win.app as usize;
            if !self.running_scratch.contains(&app) {
                self.running_scratch.push(app);
            }
        }
        if self.running_scratch == self.running_apps {
            return;
        }
        core::mem::swap(&mut self.running_apps, &mut self.running_scratch);
        let mut effective = self.config.clone();
        for (index, c) in effective.components.iter_mut().enumerate() {
            if c.kind != Kind::Apps || c.source == AppsSource::Pinned {
                continue;
            }
            let pinned = entries(&self.config.components[index], apps);
            self.app_indices[index] = c.source.resolve_entries(&pinned, &self.running_apps);
            let indices = &self.app_indices[index];
            c.entries = indices
                .iter()
                .map(|&app| apps.apps[app].id.clone())
                .collect();
        }
        match layout_panels(
            &effective,
            apps,
            self.viewport.0,
            self.viewport.1,
            &self.cascade,
        ) {
            Ok((panels, insets)) => {
                self.panels = panels;
                self.insets = insets;
            }
            Err(error) => libdunit::println(&alloc::format!(
                "gui_server: running-app layout rejected {}",
                error
            )),
        }
    }
    /// Warm every reachable text glyph at reload time. Painting only reads the
    /// atlas; malformed configurations cannot cause unbounded frame allocations.

    /// Return due component tokens once per configured interval. The observer
    /// can submit these events to Lua without polling built-in painters.
    pub fn valid_animation_target(&self,target:u32)->bool{(513..=515).contains(&target)||self.panels.iter().any(|p|target==1024+p.index as u32||p.cells.iter().any(|c|c.component+1==target as usize))}
    pub fn apply_response(&mut self,actions:&[lua_policy::Animation],updates:&[lua_policy::WidgetUpdate],now:u64,font:Option<&Font>,apps:&Applications,ly:&Layout)->Result<(),String>{
        let mut motion=self.motion.clone();
        for action in actions.iter().filter(|a|a.window_id==0){if !self.valid_animation_target(action.target){return Err("unknown component token".into());}if !(513..=515).contains(&action.target){motion.submit(action,now).map_err(String::from)?;}}
        self.apply_widget_updates(updates,font,apps,ly)?;self.motion=motion;
        if !actions.is_empty()||!updates.is_empty(){self.revision=self.revision.wrapping_add(1);}Ok(())
    }
    pub fn advance(&mut self,now:u64){
        self.animation_now=now;
        for panel in &mut self.panels{
            let visual=self.motion.sample(1024+panel.index as u32,panel.base_rect,now);panel.rect=geometry::intersect(visual.rect,(0,0,self.viewport.0 as i32,self.viewport.1 as i32));panel.opacity=visual.opacity;
            for index in 0..panel.cells.len(){
                let cell=&panel.cells[index];let visual=self.motion.sample((cell.component+1)as u32,cell.base_rect,now);let mut r=visual.rect;let mut opacity=visual.opacity;
                if cell.parent.is_none(){let b=panel.base_rect;let v=panel.rect;if b.2>0&&b.3>0{r=(v.0+((r.0-b.0)as i64*v.2 as i64/b.2 as i64)as i32,v.1+((r.1-b.1)as i64*v.3 as i64/b.3 as i64)as i32,(r.2 as i64*v.2 as i64/b.2 as i64)as i32,(r.3 as i64*v.3 as i64/b.3 as i64)as i32);}}
                if let Some(parent)=cell.parent {if let Some(p)=panel.cells.iter().find(|c|c.component==parent){let b=p.base_rect;let v=self.motion_rects[parent];
                    if b.2>0&&b.3>0{r=(v.0+((r.0-b.0)as i64*v.2 as i64/b.2 as i64)as i32,v.1+((r.1-b.1)as i64*v.3 as i64/b.3 as i64)as i32,
                        (r.2 as i64*v.2 as i64/b.2 as i64)as i32,(r.3 as i64*v.3 as i64/b.3 as i64)as i32);r=geometry::intersect(r,v);}
                    opacity=opacity*self.motion_opacity[parent]/255;
                }}
                let component=cell.component;let r=geometry::intersect(r,panel.rect);self.motion_rects[component]=r;self.motion_opacity[component]=opacity;panel.cells[index].rect=r;
            }
        }
    }
    /// Cached shell pixels are valid only before the next sampled value or
    /// transition can change. Dynamic analog clocks conservatively redraw.
    pub fn reusable(&self,now:u64)->bool {
        self.popup.is_none()&&!self.motion.animating(now)&&self.sampled_at!=u64::MAX
            &&(!self.config.components.iter().any(|c|matches!(c.kind,Kind::Tray|Kind::SystemMonitor|Kind::Clock))
                ||now.saturating_sub(self.sampled_at)<self.stats_interval)
            &&self.config.components.iter().enumerate().all(|(i,c)|match c.kind {
                Kind::AnalogClock=>false,
                Kind::Clock|Kind::Tray|Kind::SystemMonitor=>self.text_sampled_at[i]!=u64::MAX
                    &&now.saturating_sub(self.text_sampled_at[i])<c.interval_ms as u64,
                _=>true,
            })
    }
    pub fn damage_blockers(&self,now:u64,fx:&Effects)->Vec<crate::damage_plan::Rect> {
        self.panels.iter().map(|p| {
            let mut margin=fx.shadow.max(number(&p.style,"shadow",fx.shadow).clamp(0,64))
                .max(self.motion.sample(1024+p.index as u32,p.base_rect,now).shadow);
            for c in &p.cells {
                for style in [&c.normal,&c.hover,&c.focused] {margin=margin.max(number(style,"shadow",0).clamp(0,64));}
                margin=margin.max(self.motion.sample((c.component+1)as u32,c.base_rect,now).shadow);
            }
            margin=margin.max(0).saturating_add(fx.blur_radius.max(0)).saturating_add(2);
            crate::damage_plan::Rect{x:p.rect.0-margin,y:p.rect.1-margin,
                w:(p.rect.2+margin*2).max(0)as u32,h:(p.rect.3+margin*2).max(0)as u32}
        }).collect()
    }
    pub fn due_widget_updates(&mut self, now: u64) -> Vec<u32> {
        let mut due = Vec::new();
        for panel in &self.panels {
            for cell in &panel.cells {
                let index = cell.component;
                let c = &self.config.components[index];
                if !matches!(
                    c.kind,
                    Kind::Indicator
                        | Kind::Button
                        | Kind::Clock
                        | Kind::AnalogClock
                        | Kind::SystemMonitor
                        | Kind::Tray
                ) {
                    continue;
                }
                let sampled = self.widget_sampled_at[index];
                if sampled == u64::MAX || now.saturating_sub(sampled) >= c.interval_ms as u64 {
                    due.push((index + 1) as u32);
                    self.widget_sampled_at[index] = now;
                }
            }
        }
        due
    }
    /// Apply a complete event response atomically. Glyphs and geometry are
    /// prepared before replacing any labels, colors or live cached resources.

    /// Report the committed scene, shared by renderer and hit testing, for
    /// reload diagnostics and end-to-end automation observations.
    pub fn describe(&self) {
        for panel in &self.panels {
            let p = &self.config.panels[panel.index];
            let r = panel.rect;
            libdunit::println(&alloc::format!(
                "gui_server: shell panel={} edge={} rect={},{},{},{} reserve={} autohide={}",
                p.id,
                p.edge.as_str(),
                r.0,
                r.1,
                r.2,
                r.3,
                p.reserve as u8,
                p.autohide as u8
            ));
            for cell in &panel.cells {
                let c = &self.config.components[cell.component];
                let r = cell.rect;
                libdunit::println(&alloc::format!(
                    "gui_server: shell component={} kind={} rect={},{},{},{}",
                    c.id,
                    c.kind.as_str(),
                    r.0,
                    r.1,
                    r.2,
                    r.3
                ));
            }
        }
    }
    fn visible(&self, p: &Panel, mx: i32, my: i32) -> bool {
        let cfg = &self.config.panels[p.index];
        !cfg.autohide
            || match cfg.edge {
                Edge::Top => my <= p.rect.1 + p.rect.3,
                Edge::Bottom => my >= p.rect.1,
                Edge::Left => mx <= p.rect.0 + p.rect.2,
                Edge::Right => mx >= p.rect.0,
            }
    }
    pub fn contains(&self, x: i32, y: i32) -> bool {
        self.panels
            .iter()
            .any(|p| self.visible(p, x, y) && in_rect(p.rect, x, y))
            || self.popup.is_some()
    }
    pub fn first_quicksettings_anchor(&self) -> Option<(Rect, Edge)> {
        self.panels.iter().find_map(|panel| {
            panel
                .cells
                .iter()
                .find(|cell| self.config.components[cell.component].kind == Kind::QuickSettings)
                .map(|cell| (cell.rect, self.config.panels[panel.index].edge))
        })
    }
    /// Stable component token for observer events, independent of layout changes.
    pub fn event_target(&self, x: i32, y: i32) -> Option<(u32, bool)> {
        if let Some((component, rect)) = self.popup {
            if in_rect(rect, x, y) {
                return Some(((component + 1) as u32, false));
            }
        }
        self.panels
            .iter()
            .rev()
            .filter(|p| self.visible(p, x, y))
            .find_map(|panel| {
                panel
                    .cells
                    .iter()
                    .rev()
                    .find(|cell| in_rect(cell.rect, x, y))
                    .map(|cell| {
                        let kind = self.config.components[cell.component].kind;
                        (
                            (cell.component + 1) as u32,
                            matches!(
                                kind,
                                Kind::Clock
                                    | Kind::AnalogClock
                                    | Kind::SystemMonitor
                                    | Kind::Tray
                                    | Kind::Indicator
                                    | Kind::Button
                            ),
                        )
                    })
            })
    }
    pub fn app_target(&self, app: usize, _apps: &Applications) -> Option<Rect> {
        for p in &self.panels {
            for cell in &p.cells {
                let c = &self.config.components[cell.component];
                if matches!(c.kind, Kind::Apps | Kind::AppGroup) {
                    let ids = &self.app_indices[cell.component];
                    if let Some(i) = ids.iter().position(|&id| id == app) {
                        return Some(if c.kind == Kind::AppGroup {
                            cell.rect
                        } else {
                            item_rect(cell.rect, cell.horizontal, i, ids.len(), cell.gap)
                        });
                    }
                }
            }
        }
        None
    }
    pub fn click(&mut self, x: i32, y: i32, apps: &Applications) -> Option<Action> {
        if let Some((component, r)) = self.popup.take() {
            let c = &self.config.components[component];
            let ids = &self.app_indices[component];
            for (i, &app) in ids.iter().enumerate().rev() {
                if in_rect(group_cell(r, i, ids.len(), &c.presentation), x, y) {
                    return Some(Action::App(app));
                }
            }
            return None;
        }
        for p in self.panels.iter().rev() {
            if self.visible(p, x, y) && in_rect(p.rect, x, y) {
                for cell in p.cells.iter().rev().filter(|cell| in_rect(cell.rect, x, y)) {
                    let c = &self.config.components[cell.component];
                    let ids = &self.app_indices[cell.component];
                    if !c.action.is_empty(){return Some(Action::Command(c.action.clone()));}
                    let action=match c.kind {
                        Kind::Launcher => Some(Action::Launcher(
                            if ids.is_empty() {
                                apps.launcher.clone()
                            } else {
                                ids.clone()
                            },
                            cell.rect,
                            self.config.panels[p.index].edge,
                        )),
                        Kind::QuickSettings => Some(Action::QuickSettings(
                            cell.rect,
                            self.config.panels[p.index].edge,
                        )),
                        Kind::Workspaces => (0..apps.workspaces)
                            .find(|&i| {
                                in_rect(
                                    item_rect(
                                        cell.rect,
                                        cell.horizontal,
                                        i,
                                        apps.workspaces,
                                        cell.gap,
                                    ),
                                    x,
                                    y,
                                )
                            })
                            .map(Action::Workspace),
                        Kind::Apps => ids
                            .iter()
                            .enumerate()
                            .find(|(i, _)| {
                                in_rect(
                                    item_rect(cell.rect, cell.horizontal, *i, ids.len(), cell.gap),
                                    x,
                                    y,
                                )
                            })
                            .map(|(_, &id)| Action::App(id)),
                        Kind::AppGroup => Some(Action::Group(cell.component, cell.rect)),
                        _ => None,
                    };
                    if action.is_some(){return action;}
                }
            }
        }
        None
    }
    pub fn open_group(&mut self, index: usize, anchor: Rect, bw: usize, bh: usize) {
        let c = &self.config.components[index];
        libdunit::println(&alloc::format!(
            "gui_server: shell group={} open entries={} presentation={}",
            c.id,
            self.app_indices[index].len(),
            c.presentation
        ));
        self.popup = Some((
            index,
            geometry::popup(
                anchor,
                bw as i32,
                bh as i32,
                self.app_indices[index].len(),
                &c.presentation,
            ),
        ));
    }

    pub fn paint(
        &mut self,
        back: &mut [u32],
        bw: usize,
        bh: usize,
        font: Option<&Font>,
        icons: &[Option<dunit_image::Image>],
        logo:Option<&dunit_image::Image>,
        apps: &Applications,
        wins: &[Win],
        focused: Option<usize>,
        ws: usize,
        mx: i32,
        my: i32,
        now: u64,
        theme: &Theme,
        ly: &Layout,
        fx: &Effects,
        a: &mut Vec<u32>,
        b: &mut Vec<u32>,
    ) {
        self.refresh_running(wins, apps);
        if self.sampled_at == u64::MAX || now.saturating_sub(self.sampled_at) >= self.stats_interval
        {
            libdunit::get_system_stats(&mut self.stats);
            self.sampled_at = now;
        }
        let sampled_ms = if self.stats.uptime_hz > 0 {
            self.stats.uptime_ticks.saturating_mul(1000) / self.stats.uptime_hz
        } else {
            0
        };
        let clock_ms = sampled_ms.saturating_add(now.saturating_sub(self.sampled_at));
        let secs = clock_ms / 1000;
        for (index, c) in self.config.components.iter().enumerate() {
            if self.text_sampled_at[index] != u64::MAX
                && now.saturating_sub(self.text_sampled_at[index]) < c.interval_ms as u64
            {
                continue;
            }
            match c.kind {
                Kind::Clock => self.text_cache[index] = clock_text(&c.format, secs),
                Kind::Tray=>self.text_cache[index]=alloc::format!("RAM {}%  {} proc  {:02}:{:02}",self.stats.memory_used_percent(),self.stats.process_running,(secs/60)%100,secs%60),
                Kind::SystemMonitor => {
                    self.text_cache[index] = alloc::format!(
                        "RAM {}%  {} proc",
                        self.stats.memory_used_percent(),
                        self.stats.process_running
                    )
                }
                _ => continue,
            }
            self.text_sampled_at[index] = now;
        }
        for panel in &self.panels {
            if self.visible(panel, mx, my) {
                let r = panel.rect;
                let mut effects = *fx;
                effects.corner_radius = number(&panel.style, "corner-radius", fx.corner_radius);
                effects.blur = panel.style.keyword("blur").map_or(fx.blur, |v| v == "true");
                effects.panel_alpha = number(&panel.style, "opacity", fx.panel_alpha).clamp(0, 255);
                effects.blur_radius=number(&panel.style,"blur-radius",fx.blur_radius).clamp(0,32);
                effects.shadow=number(&panel.style,"shadow",fx.shadow).clamp(0,64);
                effects.shadow_alpha=number(&panel.style,"shadow-opacity",fx.shadow_alpha).clamp(0,255);
                effects.gradient=fx.gradient||panel.style.color("gradient-top").is_some()||panel.style.color("gradient-bottom").is_some();
                let panel_visual=self.motion.sample(1024+panel.index as u32,panel.base_rect,now);
                effects.panel_alpha=effects.panel_alpha*panel.opacity as i32/255;
                effects.corner_radius=panel_visual.radius.unwrap_or(effects.corner_radius);
                if self.motion.has(1024+panel.index as u32,lua_policy::Property::Blur){effects.blur=panel_visual.blur>0;effects.blur_radius=panel_visual.blur;}
                if self.motion.has(1024+panel.index as u32,lua_policy::Property::Shadow){effects.shadow=panel_visual.shadow;}
                let bg=panel_visual.color.unwrap_or_else(||color(&panel.style,"background",theme.panel));
                let gradient=(color(&panel.style,"gradient-top",0xff000000|shade(bg,12)),color(&panel.style,"gradient-bottom",0xff000000|shade(bg,-10)));
                if effects.shadow>0{draw_soft_shadow(back,bw,bh,r.0,r.1,r.2,r.3,effects.corner_radius,effects.shadow,effects.shadow_alpha);}
                let need = (r.2 * r.3).max(1) as usize;
                if a.len() < need {
                    a.resize(need, 0);
                    b.resize(need, 0);
                }
                panel_background(back,bw,bh,r,bg,gradient,&effects,a,b);
                for cell in &panel.cells {
                    let c = &self.config.components[cell.component];
                    let r = cell.rect;
                    if r.2 <= 0 || r.3 <= 0 {
                        continue;
                    }
                    let hot = in_rect(r, mx, my);
                    let focused_app = focused.and_then(|i| wins.get(i)).map(|w| w.app as usize);
                    let active = matches!(c.kind, Kind::Apps | Kind::AppGroup)
                        && focused_app
                            .map_or(false, |app| self.app_indices[cell.component].contains(&app));
                    let style = if hot {
                        &cell.hover
                    } else if active {
                        &cell.focused
                    } else {
                        &cell.normal
                    };
                    let state=if hot{1}else if active{2}else{0};
                    if self.style_states[cell.component]!=state{
                        let old=match self.style_states[cell.component]{1=>&cell.hover,2=>&cell.focused,_=>&cell.normal};
                        if fx.anim{self.motion.transition((cell.component+1)as u32,cell.base_rect,old,style,&cell.transitions[state as usize],now,theme.panel_text);}
                        self.style_states[cell.component]=state;
                    }
                    let mut visual=self.motion.sample((cell.component+1)as u32,cell.base_rect,self.animation_now);visual.opacity=self.motion_opacity[cell.component]*panel.opacity/255;
                    if !self.motion.has((cell.component+1)as u32,lua_policy::Property::Opacity){visual.opacity=visual.opacity*number(style,"opacity",255).clamp(0,255)as u32/255;}
                    let capture=geometry::intersect(r,(0,0,bw as i32,bh as i32));
                    if visual.opacity<255{self.animation_backing.clear();for y in capture.1..capture.1+capture.3{let start=y as usize*bw+capture.0 as usize;self.animation_backing.extend_from_slice(&back[start..start+capture.2 as usize]);}}
                    if !self.motion.has((cell.component+1)as u32,lua_policy::Property::Blur){visual.blur=number(style,"blur-radius",0).clamp(0,32);}
                    if !self.motion.has((cell.component+1)as u32,lua_policy::Property::Shadow){visual.shadow=number(style,"shadow",0).clamp(0,64);}
                    if visual.blur>0{let need=(r.2*r.3).max(1)as usize;if a.len()<need{a.resize(need,0);b.resize(need,0);}blur_region(back,bw,bh,r.0,r.1,r.2,r.3,visual.blur,1,a,b);}
                    if visual.shadow>0{draw_soft_shadow(back,bw,bh,r.0,r.1,r.2,r.3,visual.radius.unwrap_or(fx.corner_radius),visual.shadow,fx.shadow_alpha);}
                    let background=visual.background.or_else(||style.color("background").map(|c|(c.a as u32)<<24|(c.r as u32)<<16|(c.g as u32)<<8|c.b as u32));
                    if let Some(col)=background {
                        fill_rrect(
                            back,
                            bw,
                            bh,
                            r.0,
                            r.1,
                            r.2,
                            r.3,
                            visual.radius.unwrap_or_else(||number(style, "corner-radius", fx.corner_radius)),
                            RR_ALL,
                            col,
                        );
                    }
                    let fg=visual.color.unwrap_or_else(||color(style,"color",c.color.unwrap_or(theme.panel_text)));
                    let px = number(
                        style,
                        "font-size",
                        if c.font_px == 0 {
                            ly.panel_font_px
                        } else {
                            c.font_px as i32
                        },
                    )
                    .clamp(6, 128);
                    if let Some(icon) = &self.component_icons[cell.component] {
                        paint_image(back,bw,bh,r,icon);
                    }else{match c.kind {
                        Kind::Apps => {
                            let ids = &self.app_indices[cell.component];
                            for (i, &app) in ids.iter().enumerate() {
                                let ir = item_rect(r, cell.horizontal, i, ids.len(), cell.gap);
                                paint_app(
                                    &self.glyphs,
                                    back,
                                    bw,
                                    bh,
                                    ir,
                                    app,
                                    font,
                                    icons,
                                    apps,
                                    fg,
                                    px,
                                );
                                if wins.iter().any(|w| w.alive && w.app as usize == app) {
                                    let active = focused.map(|i| wins[i].app as usize) == Some(app);
                                    fill_rrect(
                                        back,
                                        bw,
                                        bh,
                                        ir.0 + ir.2 / 2 - 3,
                                        ir.1 + ir.3 - 3,
                                        6,
                                        3,
                                        1,
                                        RR_ALL,
                                        if active { theme.launcher } else { fg },
                                    );
                                }
                            }
                        }
                        Kind::Workspaces => {
                            for i in 0..apps.workspaces {
                                let ir = item_rect(r, cell.horizontal, i, apps.workspaces, c.gap);
                                if i == ws {
                                    fill_rrect(
                                        back,
                                        bw,
                                        bh,
                                        ir.0,
                                        ir.1,
                                        ir.2,
                                        ir.3,
                                        fx.corner_radius,
                                        RR_ALL,
                                        theme.taskbtn_focused,
                                    );
                                }
                                text(
                                    &self.glyphs,
                                    back,
                                    bw,
                                    bh,
                                    font,
                                    ir,
                                    &self.workspace_labels[i],
                                    px,
                                    fg,
                                );
                            }
                        }
                        Kind::QuickSettings => text(
                            &self.glyphs,
                            back,
                            bw,
                            bh,
                            font,
                            r,
                            if c.label.is_empty() {
                                "Settings"
                            } else {
                                &c.label
                            },
                            px,
                            fg,
                        ),
                        Kind::Launcher=>{
                            let mut label_rect=r;
                            if let Some(logo)=logo{let size=r.3.min(r.2).saturating_sub(8).clamp(0,24);paint_image(back,bw,bh,(r.0+4,r.1+(r.3-size)/2,size,size),logo);label_rect.0+=size+4;label_rect.2=(label_rect.2-size-4).max(0);}
                            text(&self.glyphs,back,bw,bh,font,label_rect,if c.label.is_empty(){"Dunit"}else{&c.label},px,fg);
                        },
                        Kind::AppGroup => {
                            let ids = &self.app_indices[cell.component];
                            for (i, &app) in ids.iter().take(4).enumerate() {
                                let half = r.2.min(r.3) / 2;
                                paint_app(
                                    &self.glyphs,
                                    back,
                                    bw,
                                    bh,
                                    (
                                        r.0 + (i as i32 % 2) * half,
                                        r.1 + (i as i32 / 2) * half,
                                        half,
                                        half,
                                    ),
                                    app,
                                    font,
                                    icons,
                                    apps,
                                    fg,
                                    px,
                                );
                            }
                        }
                        Kind::Clock => text(
                            &self.glyphs,
                            back,
                            bw,
                            bh,
                            font,
                            r,
                            &self.text_cache[cell.component],
                            px,
                            fg,
                        ),
                        Kind::AnalogClock => {
                            analog_millis(back, bw, bh, r, clock_ms, fg, theme.launcher)
                        }
                        Kind::Title => {
                            if let Some(i) = focused {
                                text(
                                    &self.glyphs,
                                    back,
                                    bw,
                                    bh,
                                    font,
                                    r,
                                    win_title(apps, wins[i].app),
                                    px,
                                    fg,
                                );
                            }
                        }
                        Kind::Tray | Kind::SystemMonitor => text(
                            &self.glyphs,
                            back,
                            bw,
                            bh,
                            font,
                            r,
                            &self.text_cache[cell.component],
                            px,
                            fg,
                        ),
                        Kind::Button | Kind::Indicator => {
                            text(&self.glyphs, back, bw, bh, font, r, &c.label, px, fg)
                        }
                        Kind::Container | Kind::Spacer => {}
                    }}
                    if visual.opacity<255{for y in 0..capture.3{for x in 0..capture.2{let dst=(capture.1+y)as usize*bw+(capture.0+x)as usize;let source=back[dst];let previous=self.animation_backing[y as usize*capture.2 as usize+x as usize];back[dst]=blend(previous,(visual.opacity<<24)|(source&0x00ff_ffff));}}}
                }
            }
        }
        if let Some((index, r)) = self.popup {
            let c = &self.config.components[index];
            fill_rrect(
                back,
                bw,
                bh,
                r.0,
                r.1,
                r.2,
                r.3,
                fx.corner_radius,
                RR_ALL,
                theme.menu,
            );
            let ids = &self.app_indices[index];
            let hovered = ids
                .iter()
                .enumerate()
                .rev()
                .find(|(i, _)| in_rect(group_cell(r, *i, ids.len(), &c.presentation), mx, my))
                .map(|(i, _)| i);
            for (i, &app) in ids.iter().enumerate() {
                let ir = group_cell(r, i, ids.len(), &c.presentation);
                if hovered == Some(i) {
                    fill_rrect(
                        back,
                        bw,
                        bh,
                        ir.0,
                        ir.1,
                        ir.2,
                        ir.3,
                        fx.corner_radius,
                        RR_ALL,
                        theme.menu_hover,
                    );
                }
                paint_app(
                    &self.glyphs,
                    back,
                    bw,
                    bh,
                    ir,
                    app,
                    font,
                    icons,
                    apps,
                    theme.panel_text,
                    ly.panel_font_px,
                );
            }
        }
    }
}
fn panel_background(
    back: &mut [u32],
    bw: usize,
    bh: usize,
    r: Rect,
    color: u32,
    gradient:(u32,u32),
    fx: &Effects,
    a: &mut [u32],
    b: &mut [u32],
) {
    let radius = fx.corner_radius.max(0).min(r.2 / 2).min(r.3 / 2);
    let (x, y, w, h) = clip_to_screen(r.0, r.1, r.2, r.3, bw, bh);
    if fx.blur && fx.blur_radius > 0 && fx.blur_iters > 0 && w * h <= a.len() && w * h <= b.len() {
        for row in 0..h {
            a[row * w..(row + 1) * w]
                .copy_from_slice(&back[(y + row) * bw + x..(y + row) * bw + x + w]);
        }
        for _ in 0..fx.blur_iters {
            box_h(a, b, w, h, fx.blur_radius as usize);
            box_v(b, a, w, h, fx.blur_radius as usize);
        }
        for row in 0..h {
            for col in 0..w {
                let px = (x + col) as i32;
                let py = (y + row) as i32;
                let coverage = rr_cov(px, py, r.0, r.1, r.2, r.3, radius, RR_ALL);
                if coverage > 0 {
                    blend_pixel(
                        back,
                        bw,
                        bh,
                        px,
                        py,
                        (coverage << 24) | (a[row * w + col] & 0xFFFFFF),
                    );
                }
            }
        }
    }
    let alpha = fx.panel_alpha.clamp(0, 255) * ((color >> 24) & 255) as i32 / 255;
    if fx.gradient {
        fill_rrect_grad(
            back, bw, bh, r.0, r.1, r.2, r.3, radius, RR_ALL, gradient.0, gradient.1, alpha,
        );
    } else {
        fill_rrect(
            back,
            bw,
            bh,
            r.0,
            r.1,
            r.2,
            r.3,
            radius,
            RR_ALL,
            ((alpha as u32) << 24) | (color & 0xFFFFFF),
        );
    }
}

fn paint_image(back: &mut [u32], bw: usize, bh: usize, r: Rect, icon: &dunit_image::Image) {
    if r.2 <= 0 || r.3 <= 0 {
        return;
    }
    let size = (r.2.min(r.3) - 6).max(1);
    blit_icon(
        back,
        bw,
        bh,
        &icon.pixels,
        icon.width,
        icon.height,
        r.0 + (r.2 - size) / 2,
        r.1 + (r.3 - size) / 2,
        size,
        size,
    );
}

fn paint_app(
    glyphs: &GlyphAtlas,
    back: &mut [u32],
    bw: usize,
    bh: usize,
    r: Rect,
    app: usize,
    font: Option<&Font>,
    icons: &[Option<dunit_image::Image>],
    apps: &Applications,
    color: u32,
    px: i32,
) {
    if r.2 <= 0 || r.3 <= 0 {
        return;
    }
    if let Some(icon) = icons.get(app).and_then(Option::as_ref) {
        paint_image(back, bw, bh, r, icon);
    } else {
        text(
            glyphs,
            back,
            bw,
            bh,
            font,
            r,
            apps.apps.get(app).map(|a| a.label.as_str()).unwrap_or("?"),
            px,
            color,
        );
    }
}
fn clock_text(format: &str, secs: u64) -> String {
    format
        .replace("%H", &alloc::format!("{:02}", (secs / 3600) % 24))
        .replace("%M", &alloc::format!("{:02}", (secs / 60) % 60))
        .replace("%S", &alloc::format!("{:02}", secs % 60))
}
// Fixed circle samples avoid floating-point transcendental work in the painter.
const CIRCLE: [(i32, i32); 12] = [
    (0, -1024),
    (512, -887),
    (887, -512),
    (1024, 0),
    (887, 512),
    (512, 887),
    (0, 1024),
    (-512, 887),
    (-887, 512),
    (-1024, 0),
    (-887, -512),
    (-512, -887),
];
fn hand(
    back: &mut [u32],
    bw: usize,
    bh: usize,
    cx: i32,
    cy: i32,
    radius: i32,
    phase: u64,
    period: u64,
    color: u32,
) {
    let (dx, dy) = geometry::hand_endpoint(radius, phase, period);
    line(back, bw, bh, cx, cy, cx + dx, cy + dy, color);
}
fn line(
    back: &mut [u32],
    bw: usize,
    bh: usize,
    mut x: i32,
    mut y: i32,
    x1: i32,
    y1: i32,
    color: u32,
) {
    let dx = (x1 - x).abs();
    let sx = if x < x1 { 1 } else { -1 };
    let dy = -(y1 - y).abs();
    let sy = if y < y1 { 1 } else { -1 };
    let mut error = dx + dy;
    loop {
        blend_pixel(back, bw, bh, x, y, color);
        if x == x1 && y == y1 {
            break;
        }
        let twice = 2 * error;
        if twice >= dy {
            error += dy;
            x += sx;
        }
        if twice <= dx {
            error += dx;
            y += sy;
        }
    }
}
pub fn analog(back: &mut [u32], bw: usize, bh: usize, r: Rect, secs: u64, fg: u32, accent: u32) {
    analog_millis(back, bw, bh, r, secs.saturating_mul(1000), fg, accent);
}
pub fn analog_millis(
    back: &mut [u32],
    bw: usize,
    bh: usize,
    r: Rect,
    millis: u64,
    fg: u32,
    accent: u32,
) {
    let radius = (r.2.min(r.3) / 2 - 3).max(1);
    let cx = r.0 + r.2 / 2;
    let cy = r.1 + r.3 / 2;
    for &(x, y) in &CIRCLE {
        fill_rect(
            back,
            bw,
            bh,
            cx + x * radius / 1024 - 1,
            cy + y * radius / 1024 - 1,
            2,
            2,
            fg,
        );
    }
    hand(back, bw, bh, cx, cy, radius * 5 / 10, millis, 43200000, fg);
    hand(back, bw, bh, cx, cy, radius * 8 / 10, millis, 3600000, fg);
    hand(back, bw, bh, cx, cy, radius * 9 / 10, millis, 60000, accent);
}
