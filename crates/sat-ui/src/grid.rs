//! The grid view: a track's modules on a canvas with the cables between
//! them. A source grid also has the clip dock where every path ends (one
//! inlet per clip); an instrument grid wires module outputs to module inputs.
//! It takes the place of the arrange view; the bottom panel stays.

use sat_doc::sat_time::PPQ;
use sat_doc::*;
use sat_modules::euclid::Params;
use sat_modules::{PortKind, PortSpec};
use sat_theme::{Rgba, Theme};
use vello::kurbo::{Point, Rect, Vec2};

use crate::icons::{self, Icon};
use crate::paint::{Align, Painter, TextStyle};

/// Height of the breadcrumb bar.
pub const BAR_H: f64 = 40.0;
/// Width of the clip dock.
pub const DOCK_W: f64 = 320.0;
/// Width of the track list beside a grid.
pub const TRACKS_W: f64 = 176.0;
/// Height of a row in the track list.
pub const TRACK_ROW_H: f64 = 34.0;
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
/// Parameters shown on a turing card.
pub const TURING_ROWS: [&str; 5] = ["steps", "change", "low", "range", "scale"];

/// What the grid view shows besides the document.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GridView {
    /// Canvas pan, in px.
    pub scroll: Vec2,
    /// The selected module.
    pub selected: Option<ModuleId>,
    /// The add-module menu is open.
    pub menu: bool,
    /// A cable being dragged: from (an output) to (the pointer), and what it carries.
    pub wire: Option<(Point, Point, PortKind)>,
    /// What the back button returns to ("arrange" when empty).
    pub back_label: &'static str,
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
    /// The bypass switch, on effects.
    pub power: Option<Rect>,
    /// Output ports: key and where the dot is.
    pub outs: Vec<(&'static str, Point)>,
    /// Input ports: key and where the dot is.
    pub ins: Vec<(&'static str, Point)>,
    /// The pattern preview (source modules), or a sampler's waveform.
    pub preview: Option<Rect>,
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
    /// Back to the view the grid was opened from.
    Back,
    /// The previous track's grid of the same kind.
    PrevTrack,
    /// The next track's grid of the same kind.
    NextTrack,
    /// Another of the same track's grids.
    Kind(GridKind),
    /// A track in the track list: show its grid.
    Track(TrackId),
    /// A track's mute button in the track list.
    TrackMute(TrackId),
    /// A track's solo button in the track list.
    TrackSolo(TrackId),
    /// The add-module button.
    AddModule,
    /// An item of the add-module menu, by kind.
    MenuItem(&'static str),
    /// A module's header.
    Header(ModuleId),
    /// An effect's bypass switch.
    Bypass(ModuleId),
    /// A sampler's waveform (click to load a sound).
    Sound(ModuleId),
    /// A module's output port.
    Out(ModuleId, &'static str),
    /// A module's input port.
    In(ModuleId, &'static str),
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
    /// The track that owns the grid.
    pub track: TrackId,
    /// The grid.
    pub grid: GridId,
    /// Source, instrument or fx.
    pub kind: GridKind,
    /// Breadcrumb bar.
    pub bar: Rect,
    /// Back button.
    pub back: Rect,
    /// Previous-track button.
    pub prev: Rect,
    /// Next-track button.
    pub next: Rect,
    /// Tabs for the track's grids: source, instrument, fx (those it has).
    pub kinds: Vec<(GridKind, Rect)>,
    /// Add-module button.
    pub add_module: Rect,
    /// Add-module menu items, when open.
    pub menu: Vec<(&'static str, Rect)>,
    /// The track list beside the canvas.
    pub track_list: Rect,
    /// Its rows: the track, the row, its mute and solo buttons.
    pub tracks: Vec<TrackRow>,
    /// Module canvas.
    pub canvas: Rect,
    /// Clip dock (empty except on source grids).
    pub dock: Rect,
    /// Add-clip button (empty except on source grids).
    pub add_clip: Rect,
    /// Module cards, in drawing order.
    pub cards: Vec<Card>,
    /// Dock rows.
    pub rows: Vec<DockRow>,
    /// Cables.
    pub wires: Vec<Wire>,
}

fn is_source(kind: &str) -> bool {
    sat_modules::SOURCES.iter().any(|(k, _)| *k == kind)
}

fn is_sampler(kind: &str) -> bool {
    kind == sat_modules::instrument::SAMPLER
}

/// Height of a sampler's waveform.
pub const WAVE_H: f64 = 56.0;

fn rows_of(m: &Module) -> Vec<Option<&'static str>> {
    match m.kind.as_str() {
        sat_modules::EUCLID => EUCLID_ROWS.iter().map(|k| Some(*k)).collect(),
        sat_modules::TURING => TURING_ROWS.iter().map(|k| Some(*k)).collect(),
        k if is_source(k) => vec![None],
        k => sat_modules::params_of(k).iter().map(|s| Some(s.key)).collect(),
    }
}

/// Port rows of a card: inputs down the left, outputs down the right.
fn port_rows(ports: &[PortSpec]) -> usize {
    let ins = ports.iter().filter(|p| p.input).count();
    ins.max(ports.len() - ins).max(1)
}

/// A row of the track list beside a grid.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TrackRow {
    /// The track.
    pub track: TrackId,
    /// The row.
    pub rect: Rect,
    /// Its mute button.
    pub mute: Rect,
    /// Its solo button (not on master).
    pub solo: Option<Rect>,
    /// Nesting depth in the group tree.
    pub depth: usize,
}

/// The tracks the list beside a grid shows, in arrange order: tracks and
/// groups, then returns, then master.
fn listed_tracks(doc: &Document) -> Vec<TrackId> {
    let order = doc.tracks_in_order();
    let main = order.iter().filter(|t| matches!(doc.tracks[t].kind, TrackKind::Track | TrackKind::Group));
    let rets = order.iter().filter(|t| doc.tracks[t].kind == TrackKind::Return);
    main.chain(rets).copied().chain([doc.master]).collect()
}

/// The grids `track` has, in tab order: source, instrument, fx.
pub fn grid_kinds(doc: &Document, track: TrackId) -> Vec<(GridKind, GridId)> {
    let Some(t) = doc.tracks.get(&track) else { return Vec::new() };
    [
        (GridKind::Source, t.source_grid),
        (GridKind::Instrument, t.instrument_grid),
        (GridKind::Fx, t.fx_grids.front().copied()),
    ]
    .into_iter()
    .filter_map(|(k, g)| Some((k, g?)))
    .collect()
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
    let ports = port_rows(sat_modules::ports(&m.kind)) as f64;
    let preview = if is_source(&m.kind) {
        PREVIEW_H
    } else if is_sampler(&m.kind) {
        WAVE_H
    } else {
        4.0
    };
    HEADER_H + PORT_H * ports + preview + PARAM_H * rows_of(m).len() as f64 + 10.0
}

/// A free spot for a new module. On a source grid: at the left edge, below
/// the lowest card (cables run to the dock on the right, so new cards stay out
/// of their way). Elsewhere: along the top, right of the rightmost card.
pub fn free_pos(doc: &Document, grid: GridId) -> (f32, f32) {
    if doc.grids.get(&grid).is_some_and(|g| g.kind != GridKind::Source) {
        let x = doc.modules_on(grid).map(|m| m.pos.0 + CARD_W as f32 + 24.0).fold(0.0f32, f32::max);
        return (x, 0.0);
    }
    let y = doc.modules_on(grid).map(|m| m.pos.1 + card_h(m) as f32 + 24.0).fold(0.0f32, f32::max);
    (0.0, y)
}

fn card(m: &Module, canvas: Rect, scroll: Vec2) -> Card {
    let o = to_screen(canvas, scroll, m.pos);
    let rect = Rect::new(o.x, o.y, o.x + CARD_W, o.y + card_h(m));
    let header = Rect::new(rect.x0, rect.y0, rect.x1, rect.y0 + HEADER_H);
    let hy = header.center().y;
    let power =
        sat_modules::fx::bypassable(&m.kind).then(|| Rect::new(rect.x1 - 26.0, hy - 9.0, rect.x1 - 8.0, hy + 9.0));
    let ports = sat_modules::ports(&m.kind);
    let row_y = |i: usize| header.y1 + PORT_H * (i as f64 + 0.5);
    let ins: Vec<_> =
        ports.iter().filter(|p| p.input).enumerate().map(|(i, p)| (p.key, Point::new(rect.x0, row_y(i)))).collect();
    let outs: Vec<_> =
        ports.iter().filter(|p| !p.input).enumerate().map(|(i, p)| (p.key, Point::new(rect.x1, row_y(i)))).collect();
    let mut y = header.y1 + PORT_H * port_rows(ports) as f64;
    let preview = if is_source(&m.kind) || is_sampler(&m.kind) {
        let h = if is_sampler(&m.kind) { WAVE_H } else { PREVIEW_H };
        let r = Rect::new(rect.x0 + 8.0, y + 2.0, rect.x1 - 8.0, y + h - 2.0);
        y += h;
        Some(r)
    } else {
        y += 4.0;
        None
    };
    let params = rows_of(m)
        .into_iter()
        .map(|k| {
            let r = Rect::new(rect.x0 + 8.0, y + 1.0, rect.x1 - 8.0, y + PARAM_H - 1.0);
            y += PARAM_H;
            (k, r)
        })
        .collect();
    Card { module: m.id, rect, header, power, outs, ins, preview, params }
}

/// Lays out `grid` inside `r`.
pub fn layout(doc: &Document, grid: GridId, r: Rect, view: &GridView) -> Option<GridLayout> {
    let g = doc.grids.get(&grid)?;
    let (track, kind) = (g.track, g.kind);
    let source = kind == GridKind::Source;
    let bar = Rect::new(r.x0, r.y0, r.x1, r.y0 + BAR_H);
    let cy = bar.center().y;
    let back = Rect::new(r.x0 + 12.0, cy - 13.0, r.x0 + 104.0, cy + 13.0);
    let prev = Rect::new(back.x1 + 10.0, cy - 13.0, back.x1 + 34.0, cy + 13.0);
    let next = Rect::new(prev.x1 + 138.0, cy - 13.0, prev.x1 + 162.0, cy + 13.0);
    let mut kx = next.x1 + 14.0;
    let kinds = grid_kinds(doc, track)
        .into_iter()
        .map(|(k, _)| {
            let w = match k {
                GridKind::Source => 72.0,
                GridKind::Instrument => 100.0,
                GridKind::Fx => 44.0,
            };
            let r = Rect::new(kx, cy - 13.0, kx + w, cy + 13.0);
            kx += w;
            (k, r)
        })
        .collect();
    let add_module = Rect::new(r.x1 - 96.0, cy - 13.0, r.x1 - 12.0, cy + 13.0);
    let menu = if view.menu {
        sat_modules::catalogue(kind)
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
    let track_list = Rect::new(r.x0, bar.y1, r.x0 + TRACKS_W, r.y1);
    let tracks = listed_tracks(doc)
        .into_iter()
        .enumerate()
        .map(|(i, t)| {
            let y = track_list.y0 + 6.0 + i as f64 * TRACK_ROW_H;
            let rect = Rect::new(track_list.x0 + 4.0, y, track_list.x1 - 4.0, y + TRACK_ROW_H - 2.0);
            let cy = rect.center().y;
            let b = |x: f64| Rect::new(x, cy - 9.0, x + 18.0, cy + 9.0);
            let master = t == doc.master;
            let mut depth = 0;
            let mut p = doc.tracks[&t].parent;
            while let Some(q) = p {
                depth += 1;
                p = doc.tracks[&q].parent;
            }
            TrackRow {
                track: t,
                rect,
                mute: b(rect.x1 - (if master { 30.0 } else { 52.0 })),
                solo: (!master).then(|| b(rect.x1 - 30.0)),
                depth,
            }
        })
        .filter(|row| row.rect.y1 <= r.y1)
        .collect();
    let canvas = Rect::new(track_list.x1, bar.y1, if source { r.x1 - DOCK_W } else { r.x1 }, r.y1);
    let (dock, add_clip) = if source {
        let dock = Rect::new(canvas.x1, bar.y1, r.x1, r.y1);
        (dock, Rect::new(dock.x1 - 34.0, dock.y0 + 10.0, dock.x1 - 10.0, dock.y0 + 34.0))
    } else {
        (Rect::ZERO, Rect::ZERO)
    };
    let cards: Vec<Card> = doc.modules_on(grid).map(|m| card(m, canvas, view.scroll)).collect();
    let rows: Vec<DockRow> = doc
        .clips_on(track)
        .filter(|_| source)
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
    Some(GridLayout {
        track,
        grid,
        kind,
        bar,
        back,
        prev,
        next,
        kinds,
        track_list,
        tracks,
        add_module,
        menu,
        canvas,
        dock,
        add_clip,
        cards,
        rows,
        wires,
    })
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
        if self.prev.contains(pt) {
            return GridHit::PrevTrack;
        }
        if self.next.contains(pt) {
            return GridHit::NextTrack;
        }
        if let Some((k, _)) = self.kinds.iter().find(|(_, r)| r.contains(pt)) {
            return GridHit::Kind(*k);
        }
        if self.track_list.contains(pt) {
            let Some(row) = self.tracks.iter().find(|t| t.rect.contains(pt)) else { return GridHit::Nothing };
            if row.mute.contains(pt) {
                return GridHit::TrackMute(row.track);
            }
            if row.solo.is_some_and(|r| r.contains(pt)) {
                return GridHit::TrackSolo(row.track);
            }
            return GridHit::Track(row.track);
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
            if let Some((k, _)) = c.ins.iter().find(|(_, p)| (*p - pt).hypot() <= PORT_GRAB) {
                return GridHit::In(c.module, k);
            }
            if !c.rect.contains(pt) {
                continue;
            }
            if c.power.is_some_and(|r| r.contains(pt)) {
                return GridHit::Bypass(c.module);
            }
            if c.header.contains(pt) {
                return GridHit::Header(c.module);
            }
            if let Some((Some(k), _)) = c.params.iter().find(|(_, r)| r.contains(pt)) {
                return GridHit::Param(c.module, k);
            }
            if self.kind == GridKind::Instrument && c.preview.is_some_and(|r| r.contains(pt)) {
                return GridHit::Sound(c.module);
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

    /// The input port nearest `pt` within grabbing distance (for dropping a cable).
    pub fn input_near(&self, pt: Point) -> Option<(ModuleId, &'static str)> {
        self.cards
            .iter()
            .flat_map(|c| c.ins.iter().map(move |(k, p)| (c.module, *k, *p)))
            .map(|(m, k, p)| (m, k, (p - pt).hypot()))
            .filter(|(_, _, d)| *d <= PORT_GRAB * 2.0)
            .min_by(|a, b| a.2.total_cmp(&b.2))
            .map(|(m, k, _)| (m, k))
    }
}

/// What a module's port carries.
pub fn port_kind(doc: &Document, module: ModuleId, port: &str) -> PortKind {
    doc.modules
        .get(&module)
        .and_then(|m| sat_modules::ports(&m.kind).iter().find(|s| s.key == port))
        .map_or(PortKind::Notes, |s| s.kind)
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

/// The track list beside the canvas: the grid's track is highlighted, a
/// click shows another track's grid of the same kind.
fn draw_tracks(p: &mut Painter<'_>, th: &Theme, doc: &Document, l: &GridLayout, levels: &crate::mixer::Levels) {
    let c = &th.c;
    p.rect(l.track_list, c.bg_surface);
    p.vline(l.track_list.x1 - 0.5, l.track_list.y0, l.track_list.y1, c.border_default);
    for row in &l.tracks {
        let Some(t) = doc.tracks.get(&row.track) else { continue };
        let r = row.rect;
        let on = row.track == l.track;
        let has = grid_kinds(doc, row.track).iter().any(|(k, _)| *k == l.kind);
        p.rrect(r, 4.0, if on { c.bg_raised } else { c.bg_surface });
        if on {
            p.rrect_stroke(r.inset(-0.5), 4.0, 1.0, c.accent_primary_border);
        }
        let master = row.track == doc.master;
        let col = if master { Rgba::hex(0xdcdcdc) } else { th.track(t.color.0) };
        p.rect(Rect::new(r.x0, r.y0 + 4.0, r.x0 + 3.0, r.y1 - 4.0), col);
        let x = r.x0 + 10.0 + 8.0 * row.depth as f64;
        let name = if master { "master" } else { t.name.as_str() };
        let tc = if has { c.text_strong } else { c.text_secondary };
        p.text_fit(x, r.center().y, name, &TextStyle::sans(12.0, tc).medium(), row.mute.x0 - x - 14.0);
        let (lv, rv) = levels.get(&row.track).copied().unwrap_or((0.0, 0.0));
        let m = Rect::new(row.mute.x0 - 10.0, r.y0 + 6.0, row.mute.x0 - 4.0, r.y1 - 6.0);
        crate::controls::meter(p, m, crate::controls::meter_pos(lv), crate::controls::meter_pos(rv));
        crate::controls::toggle(
            p,
            th,
            row.mute,
            "m",
            t.mixer.mute.then_some(crate::controls::On::Tint(c.accent_warning)),
        );
        if let Some(sr) = row.solo {
            crate::controls::toggle(p, th, sr, "s", t.mixer.solo.then_some(crate::controls::On::Tint(c.accent_info)));
        }
    }
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

/// A sampler's waveform, with where notes start and where the loop starts.
fn draw_sound(p: &mut Painter<'_>, th: &Theme, doc: &Document, m: &Module, sounds: &sat_disk::Pool, pv: Rect) {
    let c = &th.c;
    p.rrect(pv, 3.0, Rgba::hex(0x101010));
    let name = sat_modules::instrument::sound_name(doc, m);
    let peaks = m.audio.and_then(|a| sounds.peaks(a, (pv.width() - 4.0).max(1.0) as usize));
    let small = TextStyle::mono(10.5, c.text_secondary);
    let (Some(name), Some(peaks)) = (name, peaks) else {
        let msg = if name.is_some() { "sound missing · click to load" } else { "click to load a wav" };
        p.text(pv.center().x, pv.center().y, msg, &small, Align::Center);
        return;
    };
    let wave = Rect::new(pv.x0 + 2.0, pv.y0 + 14.0, pv.x1 - 2.0, pv.y1 - 3.0);
    let (mid, half) = (wave.center().y, wave.height() / 2.0);
    let r = sat_modules::macros::resolved(doc, m);
    let at = |key: &str| r.params.get(key).copied().unwrap_or(0.0) as f64;
    let (start, looped) = (at("start"), at("mode").round() as i32 == 2);
    let col = th.track(doc.tracks.get(&doc.grids[&m.grid].track).map_or(0, |t| t.color.0));
    for (i, (lo, hi)) in peaks.iter().enumerate() {
        let x = wave.x0 + i as f64;
        let a = if (i as f64) < start * peaks.len() as f64 { 0.35 } else { 0.9 };
        let (y0, y1) = (mid - *hi as f64 * half, mid - *lo as f64 * half);
        p.rect(Rect::new(x, y0.min(mid - 0.5), x + 1.0, y1.max(mid + 0.5)), col.alpha(a));
    }
    let mark = |p: &mut Painter<'_>, f: f64, col: Rgba| {
        let x = wave.x0 + f * wave.width();
        p.rect(Rect::new(x - 0.5, wave.y0, x + 0.5, wave.y1), col);
    };
    mark(p, start, c.text_max);
    if looped {
        let lf = at("loop");
        mark(p, lf, c.accent_warning);
        let x = wave.x0 + lf * wave.width();
        p.rect(Rect::new(x, wave.y0, wave.x1, wave.y0 + 2.0), c.accent_warning.alpha(0.8));
    }
    p.text(pv.x0 + 5.0, pv.y0 + 8.0, name, &TextStyle::mono(10.5, c.text_strong), Align::Left);
    if let Some(f) = m.audio.and_then(|a| doc.audio.get(&a)) {
        let secs = f.frames as f64 / f.rate.max(1) as f64;
        p.text(pv.x1 - 5.0, pv.y0 + 8.0, &format!("{secs:.2}s"), &small, Align::Right);
    }
}

/// Draws `grid` in `r` and returns its layout.
#[allow(clippy::too_many_arguments)]
pub fn draw(
    p: &mut Painter<'_>,
    th: &Theme,
    doc: &Document,
    grid: GridId,
    view: &GridView,
    selected_clip: Option<ClipId>,
    sounds: &sat_disk::Pool,
    levels: &crate::mixer::Levels,
    r: Rect,
) -> Option<GridLayout> {
    let l = layout(doc, grid, r, view)?;
    draw_tracks(p, th, doc, &l, levels);
    let c = &th.c;
    let source = l.kind == GridKind::Source;
    let tr = &doc.tracks[&l.track];
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
        if source {
            "drag from an out port to a clip's inlet to wire it · drag a wire off an inlet to unplug it"
        } else if l.kind == GridKind::Fx {
            "+ module adds an effect before out · drag between ports to reroute · the power icon bypasses"
        } else {
            "drag from an out port to an in port of the same colour · drag a wire off an in port to unplug it"
        },
        &TextStyle::mono(11.0, c.text_secondary),
        Align::Left,
    );
    // dock background, then cables under the cards
    if source {
        p.rect(l.dock, c.bg_surface);
        p.vline(l.dock.x0, l.dock.y0, l.dock.y1, c.border_default);
    }
    let cable_clip = if source { l.dock.x0 + 10.0 } else { l.canvas.x1 };
    p.clip(Rect::new(l.canvas.x0, l.canvas.y0, cable_clip, l.canvas.y1), |p| {
        for w in &l.wires {
            p.cable(w.from, w.to, 2.0, port_color(th, w.kind), w.kind == PortKind::Mod);
        }
        if let Some((a, b, k)) = view.wire {
            p.cable(a, b, 2.0, port_color(th, k).alpha(0.8), true);
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
            let mark = sat_modules::ports(&m.kind)
                .iter()
                .find(|s| !s.input)
                .or(sat_modules::ports(&m.kind).first())
                .map_or(PortKind::Audio, |s| s.kind);
            p.rrect(Rect::new(r.x0 + 10.0, hy - 5.0, r.x0 + 20.0, hy + 5.0), 2.0, port_color(th, mark));
            p.text(r.x0 + 28.0, hy, &m.name, &TextStyle::mono(12.0, c.text_max).medium(), Align::Left);
            let kind = sat_modules::label_of(&m.kind);
            let right = card.power.map_or(r.x1 - 10.0, |pw| pw.x0 - 4.0);
            if kind != m.name {
                p.text(right, hy, kind, &TextStyle::mono(10.5, c.text_secondary), Align::Right);
            }
            let off = sat_modules::fx::bypassed(m);
            if let Some(pw) = card.power {
                icons::draw(p, Icon::Power, pw.inset(-3.0), if off { c.text_secondary } else { c.accent_primary });
            }
            for (k, at) in &card.outs {
                p.text(at.x - 12.0, at.y, k, &TextStyle::mono(11.0, c.text_secondary), Align::Right);
            }
            for (k, at) in &card.ins {
                p.text(at.x + 12.0, at.y, k, &TextStyle::mono(11.0, c.text_secondary), Align::Left);
            }
            if m.kind == sat_modules::instrument::DRUM_SYNTH {
                let y = card.header.y1 + PORT_H / 2.0;
                p.text(r.x0 + 12.0, y, "plays drums by pitch", &TextStyle::mono(10.5, c.text_secondary), Align::Left);
            }
            if is_sampler(&m.kind) {
                let y = card.header.y1 + PORT_H / 2.0;
                let msg = "plays its sound by pitch";
                p.text(r.x0 + 12.0, y, msg, &TextStyle::mono(10.5, c.text_secondary), Align::Left);
            }
            // a sampler's sound
            if let Some(pv) = card.preview.filter(|_| is_sampler(&m.kind)) {
                draw_sound(p, th, doc, m, sounds, pv);
            }
            // preview
            if let Some(pv) = card.preview.filter(|_| is_source(&m.kind)) {
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
                            let x1 =
                                (pv.x0 + 3.0 + (n.start.0 + n.len.0) as f64 / len * (pv.width() - 6.0)).max(x0 + 2.0);
                            let y = pv.y1 - 5.0 - (n.pitch.saturating_sub(lo)) as f64 / span * (pv.height() - 10.0);
                            p.rect(Rect::new(x0, y - 1.0, x1 - 0.5, y + 1.0), c.accent_warning.alpha(0.9));
                        }
                    }
                    None if m.kind == sat_modules::EUCLID => {
                        let cyc = Params::of(&sat_modules::macros::resolved(doc, m)).cycle();
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
            }
            // parameters
            for (k, pr) in &card.params {
                p.rrect(*pr, 3.0, c.bg_field);
                match k {
                    Some(key) => {
                        let Some(spec) = sat_modules::spec(&m.kind, key) else { continue };
                        let v = sat_modules::macros::value(doc, m.id, key).unwrap_or(spec.default);
                        let bound = sat_modules::macros::bound(doc, m.id, key).map(|(i, _)| i);
                        let mcol = crate::macros::kind_color(l.kind);
                        let fill = Rect::new(pr.x0, pr.y0, pr.x0 + pr.width() * spec.pos(v), pr.y1);
                        let tint = if bound.is_some() { mcol.alpha(0.16) } else { Rgba::new(255, 255, 255, 14) };
                        p.rrect(fill, 3.0, tint);
                        let lw = p.text(
                            pr.x0 + 6.0,
                            pr.center().y,
                            spec.label,
                            &TextStyle::mono(11.0, c.text_secondary),
                            Align::Left,
                        );
                        if let Some(i) = bound {
                            // which macro drives it
                            let bx = pr.x0 + 10.0 + lw;
                            let badge = Rect::new(bx, pr.y0 + 3.0, bx + 24.0, pr.y1 - 3.0);
                            p.rrect(badge, 3.0, mcol.alpha(0.25));
                            let tag = format!("m{}", i + 1);
                            p.text(
                                badge.center().x,
                                badge.center().y,
                                &tag,
                                &TextStyle::mono(9.5, mcol),
                                Align::Center,
                            );
                        }
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
            if sat_modules::fx::bypassed(m) {
                let body = Rect::new(r.x0 + 1.0, card.header.y1 + 1.0, r.x1 - 1.0, r.y1 - 1.0);
                p.rect(body, c.bg_surface.alpha(0.6));
            }
        }
        if l.cards.is_empty() {
            let at = l.canvas.center();
            p.text(
                at.x,
                at.y,
                if l.kind == GridKind::Fx {
                    "no effects: audio passes straight through · + module adds one"
                } else {
                    "no modules · + module adds one"
                },
                &TextStyle::mono(12.0, c.text_secondary),
                Align::Center,
            );
        }
    });
    if source {
        draw_dock(p, th, doc, &l, selected_clip);
    }
    // port dots over the cards
    p.clip(l.canvas, |p| {
        for card in &l.cards {
            let kind = |k: &str| port_kind(doc, card.module, k);
            for (k, at) in &card.outs {
                port_dot(p, th, *at, port_color(th, kind(k)), l.wires.iter().any(|w| w.from == *at));
            }
            for (k, at) in &card.ins {
                port_dot(p, th, *at, port_color(th, kind(k)), l.wires.iter().any(|w| w.to == *at));
            }
        }
    });
    // breadcrumb bar
    p.rect(l.bar, c.bg_base);
    p.hline(l.bar.x0, l.bar.x1, l.bar.y1 - 1.0, c.border_default);
    let cy = l.bar.center().y;
    let back = if view.back_label.is_empty() { "arrange" } else { view.back_label };
    chip(p, th, l.back, back, Some(Icon::ChevronLeft));
    // the track, with buttons to step to its neighbours' grids of the same kind
    for (r, ic) in [(l.prev, Icon::ChevronLeft), (l.next, Icon::ChevronRight)] {
        p.rrect(r, 5.0, c.bg_control);
        p.rrect_stroke(r.inset(-0.5), 5.0, 1.0, c.border_default);
        icons::draw(p, ic, Rect::new(r.center().x - 6.0, cy - 6.0, r.center().x + 6.0, cy + 6.0), c.text_primary);
    }
    let x = l.prev.x1 + 10.0;
    p.rrect(Rect::new(x, cy - 5.0, x + 10.0, cy + 5.0), 2.0, tcol);
    p.text_fit(x + 16.0, cy, &tr.name, &TextStyle::mono(12.0, c.text_strong), l.next.x0 - x - 24.0);
    // the track's grids as tabs
    if let (Some((_, first)), Some((_, last))) = (l.kinds.first(), l.kinds.last()) {
        let all = Rect::new(first.x0, first.y0, last.x1, last.y1);
        p.rrect(all, 5.0, c.bg_control);
        p.rrect_stroke(all.inset(-0.5), 5.0, 1.0, c.border_default);
    }
    for (k, r) in &l.kinds {
        let on = *k == l.kind;
        if on {
            p.rrect(r.inset(-2.0), 4.0, c.bg_raised);
        }
        let title = match k {
            GridKind::Source => "source",
            GridKind::Instrument => "instrument",
            GridKind::Fx => "fx",
        };
        let col = if on { c.text_max } else { c.text_secondary };
        p.text(r.center().x, cy, title, &TextStyle::mono(12.0, col), Align::Center);
    }
    let x = l.kinds.last().map_or(l.next.x1, |(_, r)| r.x1) + 16.0;
    if let Some(clip) = selected_clip.and_then(|id| doc.clips.get(&id)).filter(|cl| cl.track == l.track && source) {
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
            let what = if is_source(k) { "source" } else { "module" };
            p.text(r.x1 - 10.0, r.center().y, what, &TextStyle::mono(10.5, c.text_secondary), Align::Right);
        }
    }
    Some(l)
}

fn draw_dock(p: &mut Painter<'_>, th: &Theme, doc: &Document, l: &GridLayout, selected_clip: Option<ClipId>) {
    let c = &th.c;
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
}
