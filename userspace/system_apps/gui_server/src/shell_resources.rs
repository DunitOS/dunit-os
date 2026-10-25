//! Configuration/event resource preparation, outside frame sampling.
use super::*;
impl Shell {
    pub fn prepare(
        config: ShellConfig,
        apps: &Applications,
        bw: usize,
        bh: usize,
    ) -> Result<Self, String> {
        let mut cascade = Cascade::new();
        if !config.stylesheet.is_empty() {
            let source = libdunit::read_binary(&config.stylesheet, 64 * 1024)
                .ok_or_else(|| alloc::format!("{}: cannot read stylesheet", config.stylesheet))?;
            let source =
                core::str::from_utf8(&source).map_err(|_| String::from("DSS must be UTF-8"))?;
            let sheet = dunit_style::parse(source)
                .map_err(|e| alloc::format!("{}: {:?}", config.stylesheet, e))?;
            cascade.push(sheet);
        }
        let mut initial = config.clone();
        for c in &mut initial.components {
            if c.kind == Kind::Apps && c.source == AppsSource::Running {
                c.entries.clear();
            }
        }
        let (panels, insets) = layout_panels(&initial, apps, bw, bh, &cascade)?;
        let mut icon_bytes = 0usize;
        let component_icons = config
            .components
            .iter()
            .enumerate()
            .map(|(index, c)| {
                if c.icon.is_empty()
                    || !panels
                        .iter()
                        .any(|p| p.cells.iter().any(|cell| cell.component == index))
                {
                    return Ok(None);
                }
                let size = c.size.max(c.width).max(c.height).clamp(16, 256) as u32;
                let icon = dunit_image::load(&c.icon, size, size)
                    .map_err(|e| alloc::format!("component {} icon {}: {:?}", c.id, c.icon, e))?;
                icon_bytes += icon.pixels.len() * 4;
                if icon_bytes > 16 * 1024 * 1024 {
                    return Err(String::from("component icons exceed resource budget"));
                }
                Ok(Some(icon))
            })
            .collect::<Result<Vec<_>, String>>()?;
        let app_indices = config
            .components
            .iter()
            .map(|c| {
                if c.kind == Kind::Apps {
                    c.source.resolve_entries(&entries(c, apps), &[])
                } else {
                    entries(c, apps)
                }
            })
            .collect();
        let text_cache = config.components.iter().map(|_| String::new()).collect();
        let text_sampled_at = config.components.iter().map(|_| u64::MAX).collect();
        let widget_sampled_at = config.components.iter().map(|_| u64::MAX).collect();
        let stats_interval = config
            .components
            .iter()
            .filter(|c| {
                c.enabled
                    && matches!(
                        c.kind,
                        Kind::Clock
                            | Kind::AnalogClock
                            | Kind::SystemMonitor
                            | Kind::Tray
                            | Kind::Indicator
                            | Kind::Button
                    )
            })
            .map(|c| c.interval_ms as u64)
            .min()
            .unwrap_or(1000);
        let count = config.components.len();
        Ok(Self {
            config,
            panels,
            insets,
            popup: None,
            stats: libdunit::SystemStats::default(),
            sampled_at: u64::MAX,
            app_indices,
            text_cache,
            text_sampled_at,
            widget_sampled_at,
            stats_interval,
            glyphs: GlyphAtlas::default(),
            workspace_labels: (1..=apps.workspaces)
                .map(|i| alloc::format!("{}", i))
                .collect(),
            cascade,
            viewport: (bw, bh),
            component_icons,
            running_apps: Vec::with_capacity(apps.apps.len()),
            running_scratch: Vec::with_capacity(apps.apps.len()),
            motion: crate::shell_motion::Tracks::default(),
            animation_backing: Vec::new(),
            animation_now: 0,
            motion_rects: alloc::vec![(0,0,0,0);count],
            motion_opacity: alloc::vec![255;count],
            style_states: alloc::vec![0;count],
        })
    }
    pub fn prepare_text(
        &mut self,
        font: Option<&Font>,
        apps: &Applications,
        ly: &Layout,
    ) -> Result<(), String> {
        self.glyphs = prepare_atlas(&self.config, &self.panels, font, apps, ly)?;
        Ok(())
    }
    pub fn apply_widget_updates(
        &mut self,
        updates: &[lua_policy::WidgetUpdate],
        font: Option<&Font>,
        apps: &Applications,
        ly: &Layout,
    ) -> Result<(), String> {
        if updates.is_empty() {
            return Ok(());
        }
        let candidate = self
            .config
            .with_widget_updates(
                updates
                    .iter()
                    .map(|update| (update.target, update.text.as_str(), update.color)),
            )
            .map_err(String::from)?;
        let mut effective = candidate.clone();
        for (index, component) in effective.components.iter_mut().enumerate() {
            if component.kind == Kind::Apps && component.source != AppsSource::Pinned {
                component.entries = self.app_indices[index]
                    .iter()
                    .map(|&i| apps.apps[i].id.clone())
                    .collect();
            }
        }
        let (panels, insets) = layout_panels(
            &effective,
            apps,
            self.viewport.0,
            self.viewport.1,
            &self.cascade,
        )?;
        let mut atlas = self.glyphs.clone();
        if let Some(font) = font {
            for update in updates {
                let index = update.target as usize - 1;
                for cell in panels
                    .iter()
                    .flat_map(|p| p.cells.iter())
                    .filter(|c| c.component == index)
                {
                    for style in [&cell.normal, &cell.hover, &cell.focused] {
                        let c = &candidate.components[index];
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
                        atlas.add(font, px, &update.text)?;
                    }
                }
            }
            atlas.finish();
        }
        self.config = candidate;
        self.panels = panels;
        self.insets = insets;
        self.glyphs = atlas;
        Ok(())
    }
}
