//! Native theme resources. Parse every candidate before replacing live state.
use crate::{
    fs,
    io::Read,
    ui::{Font, Theme},
    Error, Result,
};
use alloc::{string::String, vec};

const BASE: &str = "/assets/dwm/sdk/base.dss";
pub(crate) const POLL: crate::time::Duration = crate::time::Duration::from_millis(500);
pub(crate) struct Failure {
    pub error: Error,
    pub detail: String,
}

fn bytes(path: &str, limit: usize) -> Result<alloc::vec::Vec<u8>> {
    if !path.starts_with('/') || path.split('/').any(|p| p == "..") {
        return Err(Error(libdunit::EINVAL));
    }
    let mut file = fs::File::open(path)?;
    let count = usize::try_from(file.metadata()?.len).map_err(|_| Error(libdunit::EFBIG))?;
    if count > limit {
        return Err(Error(libdunit::EFBIG));
    }
    let mut data = vec![0; count];
    file.read_exact(&mut data)?;
    Ok(data)
}
fn text(path: &str) -> Result<String> {
    String::from_utf8(bytes(path, 64 * 1024)?).map_err(|_| Error(libdunit::EINVAL))
}
#[derive(PartialEq)]
struct Source {
    font: String,
    profile: String,
    base: String,
    animations: bool,
}
impl Source {
    fn read() -> Result<Self> {
        let document = dwm_settings::load_document().map_err(|_| Error(libdunit::EIO))?;
        let config = dwm_settings::validate_installed_document(&document)
            .map_err(|_| Error(libdunit::EINVAL))?;
        if !config.valid {
            return Err(Error(libdunit::EINVAL));
        }
        let desktop = config.settings.desktop;
        Ok(Self {
            font: desktop.font.as_str().into(),
            profile: text(desktop.sdk_theme.as_str())?,
            base: text(BASE)?,
            animations: config.settings.effects.anim,
        })
    }
    fn theme(&self, overlay: &str) -> core::result::Result<Theme, Failure> {
        Theme::parse(&self.profile, &self.base, overlay).map_err(|error| Failure {
            error: Error(libdunit::EINVAL),
            detail: alloc::format!("{:?}", error),
        })
    }
    fn font(&self) -> Result<Font> {
        Font::parse(bytes(&self.font, 4 * 1024 * 1024)?).map_err(|_| Error(libdunit::EINVAL))
    }
}
pub(crate) struct Watcher {
    source: Source,
    overlay: String,
}
impl Watcher {
    pub fn animations_enabled(&self) -> bool {
        self.source.animations
    }
    pub fn open(overlay: &str) -> Result<(Self, Theme, Font)> {
        let source = Source::read()?;
        let theme = source.theme(overlay).map_err(|failure| {
            libdunit::println(&alloc::format!("[SDKTHEME] {}", failure.detail));
            failure.error
        })?;
        let font = source.font()?;
        Ok((
            Self {
                source,
                overlay: overlay.into(),
            },
            theme,
            font,
        ))
    }
    /// A rejected candidate leaves both the current renderer and source intact.
    pub fn candidate(&self) -> core::result::Result<Option<(Self, Theme, Option<Font>)>, Failure> {
        let source = Source::read().map_err(|error| Failure {
            error,
            detail: alloc::format!("could not load configured SDK theme: {}", error),
        })?;
        if source == self.source {
            return Ok(None);
        }
        let theme = source.theme(&self.overlay)?;
        let font = if source.font != self.source.font {
            Some(source.font().map_err(|error| Failure {
                error,
                detail: alloc::format!("invalid or unreadable font {}: {}", source.font, error),
            })?)
        } else {
            None
        };
        Ok(Some((
            Self {
                source,
                overlay: self.overlay.clone(),
            },
            theme,
            font,
        )))
    }
}
