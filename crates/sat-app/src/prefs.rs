//! Settings kept between runs: the interface scale, how snapping
//! and drawing start, and the docked layout. A plain `key = value` file in
//! the user's config folder, so it is easy to read and to reset by hand.

use std::path::PathBuf;

use sat_doc::sat_time::Ticks;
use sat_ui::area::{EditorType, Slot};
use sat_ui::dock::Dock;

/// The settings.
#[derive(Debug, Clone, PartialEq)]
pub struct Prefs {
    /// The interface scale.
    pub ui_scale: f64,
    /// The arrangement's snap, `None` off.
    pub snap: Option<Ticks>,
    /// The length a drawn note starts at.
    pub note_len: Ticks,
    /// The docked layout.
    pub dock: Dock,
}

fn path() -> Option<PathBuf> {
    Some(dirs::config_dir()?.join("sat").join("settings"))
}

fn side(name: &str) -> Option<EditorType> {
    EditorType::choices(Slot::Left).iter().copied().find(|t| t.label() == name)
}

impl Prefs {
    /// Reads the settings over `defaults`; a missing or odd line keeps its default.
    pub fn load(defaults: Prefs) -> Prefs {
        let text = path().and_then(|p| std::fs::read_to_string(p).ok()).unwrap_or_default();
        Prefs::parse(&text, defaults)
    }

    /// Parses `text` over `defaults`.
    pub fn parse(text: &str, mut p: Prefs) -> Prefs {
        for line in text.lines() {
            let Some((k, v)) = line.split_once('=') else { continue };
            let (k, v) = (k.trim(), v.trim());
            let num = v.parse::<f64>().ok();
            let flag = match v {
                "true" => Some(true),
                "false" => Some(false),
                _ => None,
            };
            match k {
                "ui_scale" => p.ui_scale = num.map_or(p.ui_scale, |s| s.clamp(0.7, 1.4)),
                "snap" => p.snap = if v == "off" { None } else { v.parse().ok().map(Ticks).or(p.snap) },
                "note_len" => p.note_len = v.parse().ok().filter(|t| *t > 0).map_or(p.note_len, Ticks),
                "left_open" => p.dock.left_open = flag.unwrap_or(p.dock.left_open),
                "panel_open" => p.dock.panel_open = flag.unwrap_or(p.dock.panel_open),
                "left_w" => p.dock.left_w = num.map_or(p.dock.left_w, |w| w.clamp(160.0, 900.0)),
                "roll_h" => p.dock.roll_h = num.map_or(p.dock.roll_h, |h| h.clamp(120.0, 1400.0)),
                "left_shows" => p.dock.left_shows = side(v).unwrap_or(p.dock.left_shows),
                _ => {}
            }
        }
        p
    }

    /// The settings as text.
    pub fn to_text(&self) -> String {
        let d = &self.dock;
        format!(
            "# sat settings\nui_scale = {:.2}\nsnap = {}\nnote_len = {}\n\
             left_open = {}\nleft_w = {:.0}\nleft_shows = {}\n\
             panel_open = {}\nroll_h = {:.0}\n",
            self.ui_scale,
            self.snap.map_or("off".to_string(), |t| t.0.to_string()),
            self.note_len.0,
            d.left_open,
            d.left_w,
            d.left_shows.label(),
            d.panel_open,
            d.roll_h,
        )
    }

    /// Writes the settings out (quietly: settings are a convenience).
    pub fn save(&self) {
        let Some(p) = path() else { return };
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(p, self.to_text());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn defaults() -> Prefs {
        Prefs { ui_scale: 0.9, snap: Some(Ticks(3840)), note_len: Ticks(240), dock: Dock::default() }
    }

    #[test]
    fn round_trips_and_ignores_junk() {
        let mut p = defaults();
        p.ui_scale = 1.1;
        p.snap = None;
        p.dock.left_shows = EditorType::Browser;
        p.dock.left_w = 300.0;
        p.dock.panel_open = false;
        let back = Prefs::parse(&p.to_text(), defaults());
        assert_eq!(back, p);
        let odd = Prefs::parse("ui_scale = lots\nwhat = 3\nleft_shows = mixer\nnonsense", defaults());
        assert_eq!(odd, defaults());
    }
}
