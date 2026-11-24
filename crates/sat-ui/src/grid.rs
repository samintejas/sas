//! The source grid view: a track's source modules on a canvas, cables from
//! their outputs, and the clip dock where every path ends (one inlet per
//! clip). It takes the place of the arrange view; the bottom panel stays.

use sat_doc::sat_time::PPQ;
use sat_doc::*;
use sat_modules::euclid::{self, Params};
use sat_modules::{PortKind, SOURCES};
use sat_theme::{Rgba, Theme};
use vello::kurbo::{Point, Rect, Vec2};

use crate::icons::{self, Icon};
use crate::paint::{Align, Painter, TextStyle};

/// Height of the breadcrumb bar.
pub const BAR_H: f64 = 40.0;
/// Width of the clip dock.
pub const DOCK_W: f64 = 320.0;
/// Width of a module card.
pub const CARD_W: f64 = 172.0;
/// Height of a card's header.
pub const HEADER_H: f64 = 26.0;
/// Height of a port row.
pub const PORT_H: f64 = 22.0;
/// Height of a card's pattern preview.
pub const PREVIEW_H: f64 = 34.0;
/// Height of a parameter row.
pub const PARAM_H: f64 = 20.0;
/// Height of a dock row.
pub const ROW_H: f64 = 46.0;
/// How close (px) a click must be to a port.
pub const PORT_GRAB: f64 = 9.0;
/// Canvas margin before module position (0, 0).
pub const MARGIN: f64 = 24.0;
/// Parameters shown on a euclid card.
pub const EUCLID_ROWS: [&str; 5] = ["steps", "hits", "rot", "acc", "note"];

/// What the grid view shows besides the document.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GridView {
    /// Canvas pan, in px.
    pub scroll: Vec2,
    /// The selected module.
    pub selected: Option<ModuleId>,
    /// The add-module menu is open.
    pub menu: bool,
    /// A cable being dragged: from (an output) to (the pointer).
    pub wire: Option<(Point, Point)>,
}

/// A module card's geometry.
#[derive(Debug, Clone, PartialEq)]
pub struct Card {
    /// The module.
    pub module: ModuleId,
    /// The whole card.
    pub rect: Rect,
    /// The header (drag to move).
    pub header: Rect,
    /// Output ports: key and where the dot is.
    pub outs: Vec<(&'static str, Point)>,
    /// Input ports: key and where the dot is.
    pub ins: Vec<(&'static str, Point)>,
    /// The pattern preview.
    pub preview: Rect,
    /// Parameter rows: key (if draggable) and rect.
    pub params: Vec<(Option<&'static str>, Rect)>,
}

/// A clip's row in the dock.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DockRow {
    /// The clip.
    pub clip: ClipId,
    /// The row.
    pub rect: Rect,
    /// The inlet dot.
    pub inlet: Point,
}

/// A cable's ends on screen.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Wire {
    /// The cable.
    pub cable: CableId,
    /// Output end.
    pub from: Point,
    /// Input end.
    pub to: Point,
    /// What it carries.
    pub kind: PortKind,
}

/// What is under a point in the grid view.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GridHit {
    /// Back to the arrange view.
    Back,
    /// The add-module button.
    AddModule,
    /// An item of the add-module menu, by kind.
    MenuItem(&'static str),
    /// A module's header.
    Header(ModuleId),
    /// A module's output port.
    Out(ModuleId, &'static str),
    /// A module parameter row.
    Param(ModuleId, &'static str),
    /// Elsewhere on a module.
    Module(ModuleId),
    /// A clip's inlet.
    Inlet(ClipId),
    /// A clip's dock row.
    Row(ClipId),
    /// The add-clip button.
    AddClip,
    /// Empty canvas.
    Canvas,
    /// Nothing.
    Nothing,
}

/// The grid view's geometry.
#[derive(Debug, Clone)]
pub struct GridLayout {
    /// The track whose source grid is shown.
    pub track: TrackId,
    /// The grid.
    pub grid: GridId,
    /// Breadcrumb bar.
    pub bar: Rect,
    /// Back button.
    pub back: Rect,
    /// Add-module button.
    pub add_module: Rect,
    /// Add-module menu items, when open.
    pub menu: Vec<(&'static str, Rect)>,
    /// Module canvas.
    pub canvas: Rect,
    /// Clip dock.
    pub dock: Rect,
    /// Add-clip button.
    pub add_clip: Rect,
    /// Module cards, in drawing order.
    pub cards: Vec<Card>,
    /// Dock rows.
    pub rows: Vec<DockRow>,
    /// Cables.
    pub wires: Vec<Wire>,
}

fn rows_of(m: &Module) -> Vec<Option<&'static str>> {
    match m.kind.as_str() {
        sat_modules::EUCLID => EUCLID_ROWS.iter().map(|k| Some(*k)).collect(),
        _ => vec![None],
    }
}

/// Where module position `pos` is on screen.
pub fn to_screen(canvas: Rect, scroll: Vec2, pos: (f32, f32)) -> Point {
    Point::new(canvas.x0 + MARGIN + pos.0 as f64 - scroll.x, canvas.y0 + MARGIN + pos.1 as f64 - scroll.y)
}

/// The module position at screen point `pt`.
pub fn to_pos(canvas: Rect, scroll: Vec2, pt: Point) -> (f32, f32) {
    ((pt.x - canvas.x0 - MARGIN + scroll.x) as f32, (pt.y - canvas.y0 - MARGIN + scroll.y) as f32)
}

/// A card's height for module `m`.
pub fn card_h(m: &Module) -> f64 {
    let ports = sat_modules::ports(&m.kind).len().max(1) as f64;
    HEADER_H + PORT_H * ports + PREVIEW_H + PARAM_H * rows_of(m).len() as f64 + 10.0
}

/// A free spot for a new module: at the left edge, below the lowest card
/// (cables run to the dock on the right, so new cards stay out of their way).
pub fn free_pos(doc: &Document, grid: GridId) -> (f32, f32) {
    let y = doc.modules_on(grid).map(|m| m.pos.1 + card_h(m) as f32 + 24.0).fold(0.0f32, f32::max);
    (0.0, y)
}

fn card(m: &Module, canvas: Rect, scroll: Vec2) -> Card {
    let o = to_screen(canvas, scroll, m.pos);
    let rect = Rect::new(o.x, o.y, o.x + CARD_W, o.y + card_h(m));
    let header = Rect::new(rect.x0, rect.y0, rect.x1, rect.y0 + HEADER_H);
    let mut y = header.y1;
    let (mut outs, mut ins) = (Vec::new(), Vec::new());
    let ports = sat_modules::ports(&m.kind);
    for p in ports {
        let cy = y + PORT_H / 2.0;
        if p.input {
            ins.push((p.key, Point::new(rect.x0, cy)));
        } else {
            outs.push((p.key, Point::new(rect.x1, cy)));
        }
        y += PORT_H;
    }
    if ports.is_empty() {
        y += PORT_H;
    }
    let preview = Rect::new(rect.x0 + 8.0, y + 2.0, rect.x1 - 8.0, y + PREVIEW_H - 2.0);
    y += PREVIEW_H;
    let params = rows_of(m)
        .into_iter()
        .map(|k| {
            let r = Rect::new(rect.x0 + 8.0, y + 1.0, rect.x1 - 8.0, y + PARAM_H - 1.0);
            y += PARAM_H;
            (k, r)
        })
        .collect();
    Card { module: m.id, rect, header, outs, ins, preview, params }
}

/// Lays out the source grid of `track` inside `r`.
pub fn layout(doc: &Document, track: TrackId, r: Rect, view: &GridView) -> Option<GridLayout> {
    let grid = doc.tracks.get(&track)?.source_grid?;
    let bar = Rect::new(r.x0, r.y0, r.x1, r.y0 + BAR_H);
    let cy = bar.center().y;
    let back = Rect::new(r.x0 + 12.0, cy - 13.0, r.x0 + 104.0, cy + 13.0);
    let add_module = Rect::new(r.x1 - 96.0, cy - 13.0, r.x1 - 12.0, cy + 13.0);
    let menu = if view.menu {
        SOURCES
            .iter()
            .enumerate()
            .map(|(i, (k, _))| {
                let y = add_module.y1 + 6.0 + i as f64 * 28.0;
                (*k, Rect::new(add_module.x1 - 160.0, y, add_module.x1, y + 28.0))
            })
            .collect()
    } else {
        Vec::new()
    };
    let canvas = Rect::new(r.x0, bar.y1, r.x1 - DOCK_W, r.y1);
    let dock = Rect::new(canvas.x1, bar.y1, r.x1, r.y1);
    let add_clip = Rect::new(dock.x1 - 34.0, dock.y0 + 10.0, dock.x1 - 10.0, dock.y0 + 34.0);
    let cards: Vec<Card> = doc.modules_on(grid).map(|m| card(m, canvas, view.scroll)).collect();
    let rows: Vec<DockRow> = doc
        .clips_on(track)
        .enumerate()
        .map(|(i, c)| {
            let y = dock.y0 + 44.0 + i as f64 * (ROW_H + 6.0);
            let rect = Rect::new(dock.x0 + 18.0, y, dock.x1 - 10.0, y + ROW_H);
            DockRow { clip: c.id, rect, inlet: Point::new(dock.x0 + 2.0, rect.center().y) }
        })
        .collect();
    let out_at = |pr: &PortRef| -> Option<(Point, PortKind)> {
        let c = cards.iter().find(|c| c.module == pr.module)?;
        let p = c.outs.iter().find(|(k, _)| *k == pr.port).map(|(_, p)| *p).or(c.outs.first().map(|(_, p)| *p))?;
        let kind = sat_modules::ports(&doc.modules[&pr.module].kind)
            .iter()
            .find(|s| s.key == pr.port)
            .map_or(PortKind::Notes, |s| s.kind);
        Some((p, kind))
    };
    let wires = doc
        .cables_on(grid)
        .filter_map(|k| {
            let (from, kind) = out_at(&k.from)?;
            let to = match &k.to {
                Endpoint::Inlet(c) => rows.iter().find(|r| r.clip == *c)?.inlet,
                Endpoint::Port(p) => {
                    cards.iter().find(|c| c.module == p.module)?.ins.iter().find(|(key, _)| *key == p.port)?.1
                }
            };
            Some(Wire { cable: k.id, from, to, kind })
        })
        .collect();
    Some(GridLayout { track, grid, bar, back, add_module, menu, canvas, dock, add_clip, cards, rows, wires })
}

impl GridLayout {
    /// What is under `pt`.
    pub fn hit(&self, pt: Point) -> GridHit {
        if let Some((k, _)) = self.menu.iter().find(|(_, r)| r.contains(pt)) {
            return GridHit::MenuItem(k);
        }
        if self.back.contains(pt) {
            return GridHit::Back;
        }
        if self.add_module.contains(pt) {
            return GridHit::AddModule;
        }
        if self.add_clip.contains(pt) {
            return GridHit::AddClip;
        }
        if let Some(r) = self.rows.iter().find(|r| (r.inlet - pt).hypot() <= PORT_GRAB) {
            return GridHit::Inlet(r.clip);
        }
        if let Some(r) = self.rows.iter().find(|r| r.rect.contains(pt)) {
            return GridHit::Row(r.clip);
        }
        if self.dock.contains(pt) || self.bar.contains(pt) {
            return GridHit::Nothing;
        }
        if !self.canvas.contains(pt) {
            return GridHit::Nothing;
        }
        for c in self.cards.iter().rev() {
            if let Some((k, _)) = c.outs.iter().find(|(_, p)| (*p - pt).hypot() <= PORT_GRAB) {
                return GridHit::Out(c.module, k);
            }
            if !c.rect.contains(pt) {
                continue;
            }
            if c.header.contains(pt) {
                return GridHit::Header(c.module);
            }
            if let Some((Some(k), _)) = c.params.iter().find(|(_, r)| r.contains(pt)) {
                return GridHit::Param(c.module, k);
            }
            return GridHit::Module(c.module);
        }
        GridHit::Canvas
    }

    /// The card of `module`.
    pub fn card(&self, module: ModuleId) -> Option<&Card> {
        self.cards.iter().find(|c| c.module == module)
    }

    /// The inlet nearest `pt` within grabbing distance (for dropping a cable).
    pub fn inlet_near(&self, pt: Point) -> Option<ClipId> {
        self.rows
            .iter()
            .filter(|r| (r.inlet - pt).hypot() <= PORT_GRAB * 2.0 || r.rect.contains(pt))
            .min_by(|a, b| (a.inlet - pt).hypot().total_cmp(&(b.inlet - pt).hypot()))
            .map(|r| r.clip)
    }
}

/// The colour of a cable or port.
pub fn port_color(th: &Theme, k: PortKind) -> Rgba {
    match k {
        PortKind::Notes => th.c.accent_warning,
        PortKind::Audio => Rgba::hex(0x3b8bd2),
        PortKind::Mod => Rgba::hex(0x8a78f0),
    }
}

fn port_dot(p: &mut Painter<'_>, th: &Theme, at: Point, col: Rgba, filled: bool) {
    p.circle(at, 5.0, if filled { col } else { th.c.bg_base });
    p.circle_stroke(at, 5.0, 1.5, col);
}

fn chip(p: &mut Painter<'_>, th: &Theme, r: Rect, label: &str, icon: Option<Icon>) {
    let c = &th.c;
    p.rrect(r, 5.0, c.bg_control);
    p.rrect_stroke(r.inset(-0.5), 5.0, 1.0, c.border_strong.alpha(0.6));
    let mut x = r.x0 + 10.0;
    if let Some(ic) = icon {
        icons::draw(p, ic, Rect::new(x, r.center().y - 6.0, x + 12.0, r.center().y + 6.0), c.text_primary);
        x += 18.0;
    }
    p.text(x, r.center().y, label, &TextStyle::mono(12.0, c.text_strong), Align::Left);
}

/// The clip a card previews: the selected clip if the module feeds it,
/// otherwise the first clip it feeds.
fn preview_clip(doc: &Document, m: ModuleId, selected: Option<ClipId>) -> Option<ClipId> {
    let feeds: Vec<ClipId> = doc
        .cables
        .values()
        .filter(|k| k.from.module == m)
        .filter_map(|k| if let Endpoint::Inlet(c) = k.to { Some(c) } else { None })
        .collect();
    selected.filter(|s| feeds.contains(s)).or(feeds.first().copied())
}

/// Draws the source grid of `track` in `r` and returns its layout.
pub fn draw(
    p: &mut Painter<'_>,
    th: &Theme,
    doc: &Document,
    track: TrackId,
    view: &GridView,
    selected_clip: Option<ClipId>,
    r: Rect,
) -> Option<GridLayout> {
    let l = layout(doc, track, r, view)?;
    let c = &th.c;
    let tr = &doc.tracks[&track];
    let tcol = th.track(tr.color.0);
    // canvas
    p.rect(l.canvas, Rgba::hex(0x0b0b0b));
    let step = 24.0;
    let (ox, oy) = (l.canvas.x0 - view.scroll.x.rem_euclid(step), l.canvas.y0 - view.scroll.y.rem_euclid(step));
    let mut y = oy;
    while y < l.canvas.y1 {
        let mut x = ox;
        while x < l.canvas.x1 {
            if x >= l.canvas.x0 && y >= l.canvas.y0 {
                p.rect(Rect::new(x, y, x + 1.0, y + 1.0), Rgba::new(255, 255, 255, 18));
            }
            x += step;
        }
        y += step;
    }
    // hint, under everything
    p.text(
        l.canvas.x0 + 16.0,
        l.canvas.y1 - 16.0,
        "drag from an out port to a clip's inlet to wire it · drag a wire off an inlet to unplug it",
        &TextStyle::mono(11.0, c.text_secondary),
        Align::Left,
    );
    // dock background, then cables under the cards
    p.rect(l.dock, c.bg_surface);
    p.vline(l.dock.x0, l.dock.y0, l.dock.y1, c.border_default);
    p.clip(Rect::new(l.canvas.x0, l.canvas.y0, l.dock.x0 + 10.0, l.canvas.y1), |p| {
        for w in &l.wires {
            p.cable(w.from, w.to, 2.0, port_color(th, w.kind), w.kind == PortKind::Mod);
        }
        if let Some((a, b)) = view.wire {
            p.cable(a, b, 2.0, c.accent_warning.alpha(0.8), true);
        }
    });
    p.clip(l.canvas, |p| {
        for card in &l.cards {
            let m = &doc.modules[&card.module];
            let sel = view.selected == Some(m.id);
            let r = card.rect;
            p.rrect(r, 6.0, c.bg_surface);
            p.rrect_stroke(
                r.inset(-0.5),
                6.0,
                if sel { 1.5 } else { 1.0 },
                if sel { c.accent_primary } else { c.border_default },
            );
            p.hline(r.x0 + 1.0, r.x1 - 1.0, card.header.y1, c.border_subtle);
            let hy = card.header.center().y;
            p.rrect(Rect::new(r.x0 + 10.0, hy - 5.0, r.x0 + 20.0, hy + 5.0), 2.0, c.accent_warning);
            p.text(r.x0 + 28.0, hy, &m.name, &TextStyle::mono(12.0, c.text_max).medium(), Align::Left);
            let kind = sat_modules::label_of(&m.kind);
            if kind != m.name {
                p.text(r.x1 - 10.0, hy, kind, &TextStyle::mono(10.5, c.text_secondary), Align::Right);
            }
            for (k, at) in &card.outs {
                p.text(at.x - 12.0, at.y, k, &TextStyle::mono(11.0, c.text_secondary), Align::Right);
            }
            // preview
            let pv = card.preview;
            p.rrect(pv, 3.0, Rgba::hex(0x101010));
            let pc = preview_clip(doc, m.id, selected_clip);
            match pc.and_then(|id| doc.clips.get(&id)) {
                Some(clip) => {
                    let notes = sat_modules::effective_notes(doc, m.id, clip.id);
                    let (lo, hi) = notes.iter().fold((127u8, 0u8), |(a, b), n| (a.min(n.pitch), b.max(n.pitch)));
                    let span = (hi.saturating_sub(lo)).max(4) as f64;
                    let len = clip.length.0.max(1) as f64;
                    for n in &notes {
                        let x0 = pv.x0 + 3.0 + n.start.0 as f64 / len * (pv.width() - 6.0);
                        let x1 = (pv.x0 + 3.0 + (n.start.0 + n.len.0) as f64 / len * (pv.width() - 6.0)).max(x0 + 2.0);
                        let y = pv.y1 - 5.0 - (n.pitch.saturating_sub(lo)) as f64 / span * (pv.height() - 10.0);
                        p.rect(Rect::new(x0, y - 1.0, x1 - 0.5, y + 1.0), c.accent_warning.alpha(0.9));
                    }
                }
                None if m.kind == sat_modules::EUCLID => {
                    let cyc = Params::of(m).cycle();
                    let w = (pv.width() - 8.0) / cyc.len().max(1) as f64;
                    for (i, (hit, acc)) in cyc.iter().enumerate() {
                        let at = Point::new(pv.x0 + 4.0 + (i as f64 + 0.5) * w, pv.center().y);
                        if *hit {
                            p.circle(at, (w * 0.35).min(3.5), if *acc { c.text_max } else { c.accent_warning });
                        } else {
                            p.circle_stroke(at, (w * 0.3).min(3.0), 1.0, c.text_secondary.alpha(0.5));
                        }
                    }
                }
                None => {
                    p.text(
                        pv.center().x,
                        pv.center().y,
                        "not wired",
                        &TextStyle::mono(10.5, c.text_secondary),
                        Align::Center,
                    );
                }
            }
            // parameters
            for (k, pr) in &card.params {
                p.rrect(*pr, 3.0, c.bg_field);
                match k {
                    Some(key) => {
                        let spec = euclid::spec(key).unwrap();
                        let v = spec.clamp(m.params.get(*key).copied().unwrap_or(spec.default));
                        let fill = Rect::new(pr.x0, pr.y0, pr.x0 + pr.width() * spec.pos(v), pr.y1);
                        p.rrect(fill, 3.0, Rgba::new(255, 255, 255, 14));
                        p.text(
                            pr.x0 + 6.0,
                            pr.center().y,
                            spec.label,
                            &TextStyle::mono(11.0, c.text_secondary),
                            Align::Left,
                        );
                        p.text(
                            pr.x1 - 6.0,
                            pr.center().y,
                            &spec.show(v),
                            &TextStyle::mono(11.0, c.text_strong),
                            Align::Right,
                        );
                    }
                    None => {
                        let n = doc.cables.values().filter(|k| k.from.module == m.id).count();
                        p.text(
                            pr.x0 + 6.0,
                            pr.center().y,
                            "clips",
                            &TextStyle::mono(11.0, c.text_secondary),
                            Align::Left,
                        );
                        p.text(
                            pr.x1 - 6.0,
                            pr.center().y,
                            &n.to_string(),
                            &TextStyle::mono(11.0, c.text_strong),
                            Align::Right,
                        );
                    }
                }
            }
        }
        if l.cards.is_empty() {
            let at = l.canvas.center();
            p.text(
                at.x,
                at.y,
                "no modules · + module adds one",
                &TextStyle::mono(12.0, c.text_secondary),
                Align::Center,
            );
        }
    });
    // dock
    let dy = l.dock.y0 + 22.0;
    let w = p.text(l.dock.x0 + 18.0, dy, "clips", &TextStyle::mono(12.0, c.text_max).medium(), Align::Left);
    p.text(l.dock.x0 + 26.0 + w, dy, "patterns live here", &TextStyle::mono(11.0, c.text_secondary), Align::Left);
    p.rrect(l.add_clip, 4.0, c.bg_control);
    p.rrect_stroke(l.add_clip.inset(-0.5), 4.0, 1.0, c.border_default);
    icons::draw(p, Icon::Plus, l.add_clip.inset(-6.0), c.text_primary);
    for (i, row) in l.rows.iter().enumerate() {
        let clip = &doc.clips[&row.clip];
        let sel = selected_clip == Some(clip.id);
        let col = th.track(clip.color.0);
        p.rrect(row.rect, 5.0, if sel { c.accent_primary_subtle } else { c.bg_field });
        if sel {
            p.rrect_stroke(row.rect.inset(-0.5), 5.0, 1.0, c.accent_primary.alpha(0.7));
        }
        let badge =
            Rect::new(row.rect.x0 + 8.0, row.rect.center().y - 9.0, row.rect.x0 + 26.0, row.rect.center().y + 9.0);
        p.rrect(badge, 3.0, col);
        let letter = ((b'a' + (i % 26) as u8) as char).to_string();
        p.text(
            badge.center().x,
            badge.center().y,
            &letter,
            &TextStyle::mono(11.0, c.on_accent_primary).medium(),
            Align::Center,
        );
        let tx = badge.x1 + 10.0;
        p.text(tx, row.rect.y0 + 15.0, &clip.name, &TextStyle::mono(12.0, c.text_max), Align::Left);
        let bars = clip.length.0 as f64 / (PPQ * 4) as f64;
        let info = format!("pattern {letter} · {bars} bar{}", if bars == 1.0 { "" } else { "s" });
        p.text(tx, row.rect.y1 - 13.0, &info, &TextStyle::mono(10.5, c.text_secondary), Align::Left);
        let feed = doc.inlet_cable(clip.id).and_then(|k| doc.modules.get(&k.from.module));
        let (src, scol) = match feed {
            Some(m) => (m.name.as_str(), c.text_secondary),
            None => ("unwired", c.accent_warning),
        };
        p.text(row.rect.x1 - 10.0, row.rect.center().y, src, &TextStyle::mono(11.0, scol), Align::Right);
        port_dot(p, th, row.inlet, if feed.is_some() { c.accent_warning } else { c.text_secondary }, feed.is_some());
    }
    let hy = l.dock.y1 - 30.0;
    for (i, line) in
        ["one clip per inlet: a wire from a module's", "notes out into an inlet feeds that clip"].iter().enumerate()
    {
        p.text(l.dock.x0 + 18.0, hy + i as f64 * 15.0, line, &TextStyle::mono(10.5, c.text_secondary), Align::Left);
    }
    // output dots over the cards
    p.clip(l.canvas, |p| {
        for card in &l.cards {
            for (_, at) in &card.outs {
                port_dot(p, th, *at, c.accent_warning, l.wires.iter().any(|w| w.from == *at));
            }
        }
    });
    // breadcrumb bar
    p.rect(l.bar, c.bg_base);
    p.hline(l.bar.x0, l.bar.x1, l.bar.y1 - 1.0, c.border_default);
    let cy = l.bar.center().y;
    chip(p, th, l.back, "arrange", Some(Icon::ChevronLeft));
    let mut x = l.back.x1 + 16.0;
    p.rrect(Rect::new(x, cy - 5.0, x + 10.0, cy + 5.0), 2.0, tcol);
    x += 18.0;
    x += p.text(x, cy, &tr.name, &TextStyle::mono(12.0, c.text_strong), Align::Left) + 8.0;
    x += p.text(x, cy, "/", &TextStyle::mono(12.0, c.text_secondary), Align::Left) + 8.0;
    x += p.text(x, cy, "source grid", &TextStyle::mono(12.0, c.text_max).medium(), Align::Left) + 16.0;
    if let Some(clip) = selected_clip.and_then(|id| doc.clips.get(&id)).filter(|cl| cl.track == track) {
        let label = format!("clip · {}", clip.name);
        let w = p.fonts.measure(&label, &TextStyle::mono(12.0, c.text_strong)) as f64;
        chip(p, th, Rect::new(x, cy - 13.0, x + w + 20.0, cy + 13.0), &label, None);
    }
    let mut lx = l.add_module.x0 - 16.0;
    for (label, kind) in [("mod", PortKind::Mod), ("audio", PortKind::Audio), ("notes", PortKind::Notes)] {
        let w = p.text(lx, cy, label, &TextStyle::mono(11.0, c.text_secondary), Align::Right);
        let x1 = lx - w - 6.0;
        let col = port_color(th, kind);
        if kind == PortKind::Mod {
            p.cable(Point::new(x1 - 18.0, cy), Point::new(x1, cy), 2.0, col, true);
        } else {
            p.line(Point::new(x1 - 18.0, cy), Point::new(x1, cy), 2.0, col);
        }
        lx = x1 - 32.0;
    }
    chip(p, th, l.add_module, "module", Some(Icon::Plus));
    if !l.menu.is_empty() {
        let first = l.menu[0].1;
        let all = Rect::new(first.x0 - 4.0, first.y0 - 4.0, first.x1 + 4.0, l.menu.last().unwrap().1.y1 + 4.0);
        p.rrect(all, 6.0, c.bg_surface);
        p.rrect_stroke(all.inset(-0.5), 6.0, 1.0, c.border_strong);
        for (k, r) in &l.menu {
            p.text(
                r.x0 + 10.0,
                r.center().y,
                sat_modules::label_of(k),
                &TextStyle::mono(12.0, c.text_strong),
                Align::Left,
            );
            p.text(r.x1 - 10.0, r.center().y, "source", &TextStyle::mono(10.5, c.text_secondary), Align::Right);
        }
    }
    Some(l)
}
