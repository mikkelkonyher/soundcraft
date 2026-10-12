//! The EQ 7-Band window: input/output and the two filters beside a response graph with a draggable
//! node per band, and the five bands underneath. Every change goes through `mix.insert_params`, so
//! undo, the control channel and MCP see the same edits. The curves are cached and only recomputed
//! when a parameter changes; the window adds no repaints of its own.

use crate::SoundApp;
use crate::mix_window::editable_readout;
use crate::theme::{Tokens, bold, regular};
use crate::widgets::{confined, hmeter, meter_pos, param_knob, pill_switch};
use egui::{Align2, Color32, CornerRadius, Id, Mesh, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use serde_json::{Map, Value, json};
use soundcraft_dsp::{ParamInfo, PluginInfo};
use soundcraft_model::{Insert, TrackId};
use std::sync::Arc;

/// The shapes a band can take, drawn on its type buttons.
#[derive(Clone, Copy)]
enum Shape {
    HighPass,
    LowPass,
    LowShelf,
    HighShelf,
    Peak,
    Notch,
}

struct Band {
    name: &'static str,
    on: &'static str,
    freq: &'static str,
    gain: Option<&'static str>,
    /// Q: a peak's or shelf's width, or a filter's width as a notch.
    q: &'static str,
    slope: Option<&'static str>,
    /// A two-way type choice and the shapes of its options (shelf/peak, filter/notch).
    kind: Option<(&'static str, [Shape; 2])>,
}

const BANDS: [Band; 7] = [
    Band {
        name: "HPF",
        on: "hpf_on",
        freq: "hpf_freq",
        gain: None,
        q: "hpf_q",
        slope: Some("hpf_slope"),
        kind: Some(("hpf_type", [Shape::HighPass, Shape::Notch])),
    },
    Band {
        name: "LF",
        on: "low_shelf_on",
        freq: "low_shelf_freq",
        gain: Some("low_shelf_gain"),
        q: "low_shelf_q",
        slope: None,
        kind: Some(("low_shelf_type", [Shape::LowShelf, Shape::Peak])),
    },
    Band { name: "LMF", on: "low_mid_on", freq: "low_mid_freq", gain: Some("low_mid_gain"), q: "low_mid_q", slope: None, kind: None },
    Band { name: "MF", on: "mid_on", freq: "mid_freq", gain: Some("mid_gain"), q: "mid_q", slope: None, kind: None },
    Band { name: "HMF", on: "high_mid_on", freq: "high_mid_freq", gain: Some("high_mid_gain"), q: "high_mid_q", slope: None, kind: None },
    Band {
        name: "HF",
        on: "high_shelf_on",
        freq: "high_shelf_freq",
        gain: Some("high_shelf_gain"),
        q: "high_shelf_q",
        slope: None,
        kind: Some(("high_shelf_type", [Shape::HighShelf, Shape::Peak])),
    },
    Band {
        name: "LPF",
        on: "lpf_on",
        freq: "lpf_freq",
        gain: None,
        q: "lpf_q",
        slope: Some("lpf_slope"),
        kind: Some(("lpf_type", [Shape::LowPass, Shape::Notch])),
    },
];

impl Band {
    fn keys(&self) -> impl Iterator<Item = &'static str> {
        [Some(self.on), Some(self.freq), self.gain, Some(self.q), self.slope, self.kind.map(|(k, _)| k)].into_iter().flatten()
    }
}

/// Graph range: 10 Hz (the high-pass filter's lowest cutoff) to 20 kHz, ±24 dB (every band's gain range).
const F_MIN: f32 = 10.0;
const F_MAX: f32 = 20_000.0;
const DB_RANGE: f32 = 24.0;
/// Room left and right of the frequency axis, so a node at either end stays inside the graph.
const X_PAD: f32 = 10.0;
/// A band counts as in use (drawn bright and filled) above this gain; the DSP skips it only at 0.
const SHOWN_GAIN: f32 = 0.05;
const AUDIBLE_GAIN: f32 = 1e-4;

/// Layout on an 8-point rhythm: the gap between sections and the padding inside them.
const GAP: f32 = 8.0;
const PAD: f32 = 8.0;
/// Heights of the two rows (filters, I/O and graph on top; the five bands below), of the I/O
/// section and of a section's title row.
const TOP_H: f32 = 280.0;
const BANDS_H: f32 = 192.0;
const IO_H: f32 = 104.0;
const TITLE_H: f32 = 28.0;
/// Text sizes: values (the thing you read most), labels, band names.
const VALUE_PT: f32 = 11.5;
const LABEL_PT: f32 = 10.5;
const NAME_PT: f32 = 13.0;
/// Knob sizes: the small Q and frequency knobs, and the large gain (and filter frequency) knobs.
const SMALL_KNOB: f32 = 36.0;
const LARGE_KNOB: f32 = 54.0;
/// Beside a knob: the label's line and the value box under it, 2 points apart.
const LABEL_H: f32 = 14.0;
const VALUE_H: f32 = 22.0;
/// A title row's IN/Ø switch and its type buttons (spaced 2 points apart, 6 from IN).
const SWITCH: egui::Vec2 = egui::Vec2::new(30.0, 18.0);
const TYPE_BUTTON: egui::Vec2 = egui::Vec2::new(20.0, 18.0);
/// The IN and OUT meters: their top, height and row pitch, the width of their labels, and where the
/// gain knobs start below them and their scale.
const METER_TOP: f32 = 10.0;
const METER_H: f32 = 10.0;
const METER_ROW: f32 = 16.0;
const METER_LABEL_W: f32 = 32.0;
const IO_KNOBS_TOP: f32 = 50.0;
/// The graph's plot inside its frame: room for the dB labels on the left and the Hz labels below.
const AXIS_W: f32 = 34.0;
const AXIS_H: f32 = 22.0;
const PLOT_INSET: f32 = 10.0;
/// A node's radius and the square it can be grabbed in.
const NODE_R: f32 = 6.0;
const NODE_HIT: f32 = 18.0;

/// The insert being edited, and the session rate its curves are drawn at.
#[derive(Clone, Copy)]
struct Ctx<'a> {
    tid: TrackId,
    slot: usize,
    info: &'static PluginInfo,
    ins: &'a Insert,
    sr: f32,
}

impl Ctx<'_> {
    fn param(&self, id: &str) -> Option<&'static ParamInfo> {
        self.info.param(id)
    }

    /// The insert's value for `p`, clamped to its range (default when unset).
    fn current(&self, p: &ParamInfo) -> f32 {
        p.clamp(self.ins.params.get(p.id).copied().unwrap_or(p.default))
    }

    fn value(&self, id: &str) -> f32 {
        self.param(id).map_or(0.0, |p| self.current(p))
    }

    /// Switched in and, for a band with gain, more than `min_gain` of it.
    fn engaged(&self, b: &Band, min_gain: f32) -> bool {
        self.value(b.on) >= 0.5 && b.gain.is_none_or(|g| self.value(g).abs() > min_gain)
    }

    /// The band's type is its second option (peak instead of shelf, notch instead of filter).
    fn second_kind(&self, b: &Band) -> bool {
        b.kind.is_some_and(|(k, _)| self.value(k) >= 0.5)
    }

    /// The band's Q control as it applies now: none for a filter (only a notch has a width), and a
    /// shelf's narrower range, so the knob shows the Q the shelf actually uses.
    fn q_param(&self, b: &Band) -> Option<ParamInfo> {
        let p = self.param(b.q)?;
        match (b.gain, b.kind, self.second_kind(b)) {
            (None, _, false) => None,
            (Some(_), Some(_), false) => {
                let (min, max) = soundcraft_dsp::plugins::eq::SHELF_Q;
                Some(ParamInfo { min, max, ..p.clone() })
            }
            _ => Some(p.clone()),
        }
    }

    fn id(&self, what: &str, which: impl std::hash::Hash + std::fmt::Debug) -> Id {
        Id::new((what, self.tid.0, self.slot, which))
    }

    fn selected(&self, ui: &Ui) -> Option<usize> {
        ui.data(|d| d.get_temp(self.id("eq_selected", ())))
    }

    fn select(&self, ui: &Ui, k: usize) {
        ui.data_mut(|d| d.insert_temp(self.id("eq_selected", ()), k));
    }
}

fn band_color(k: usize) -> Color32 {
    let t = Tokens::current();
    t.eq_band.get(k).copied().unwrap_or(t.accent)
}

/// Part of a continuous gesture (a drag, a scroll, arrow keys on a focused control): edits with the
/// same `key` merge into one undo step until the app ends the gesture (pointer released, scrolling
/// stopped, focus lost).
fn gesture(app: &mut SoundApp, c: Ctx, key: &str, params: Map<String, Value>) {
    if !params.is_empty() {
        let _ = app.engine.execute_merged("mix.insert_params", &json!({"track": c.tid.0, "slot": c.slot, "params": params}), key);
    }
}

/// One discrete edit (a switch, a menu, a typed value, a reset): its own undo step.
fn commit(app: &mut SoundApp, c: Ctx, params: Map<String, Value>) {
    app.engine.end_merge();
    let _ = app.run("mix.insert_params", json!({"track": c.tid.0, "slot": c.slot, "params": params}));
}

fn one(id: &str, value: f32) -> Map<String, Value> {
    Map::from_iter([(id.to_string(), json!(value))])
}

/// Curves for one parameter state.
struct Curves {
    key: u64,
    /// Sample frequencies: log-spaced, plus every band's own frequency so peaks are drawn exactly.
    freqs: Vec<f32>,
    total: Vec<f32>,
    /// Each band's own response at `freqs`; empty for a band that leaves the signal flat.
    bands: [Vec<f32>; 7],
}

/// `n` log-spaced frequencies across the graph plus each band's own frequency, sorted; none at or
/// above Nyquist.
fn sample_freqs(c: Ctx, n: usize) -> Vec<f32> {
    let n = n.max(2);
    let top = F_MAX.min(c.sr * 0.49);
    let mut fs: Vec<f32> = (0..n).map(|i| F_MIN * (F_MAX / F_MIN).powf(i as f32 / (n - 1) as f32)).filter(|f| *f <= top).collect();
    fs.extend(BANDS.iter().map(|b| c.value(b.freq)).filter(|f| f.is_finite() && (F_MIN..=top).contains(f)));
    fs.sort_by(f32::total_cmp);
    fs.dedup_by(|a, b| (*a - *b).abs() < 1e-3);
    fs
}

fn curves(c: Ctx, n: usize, key: u64) -> Curves {
    let freqs = sample_freqs(c, n);
    // Cascaded filters multiply, so their levels in dB add: the total is every band's own response,
    // and a flat band costs nothing. Input and output gain are left out: the graph shows the EQ's
    // shape around 0 dB, and the meters show the level.
    let mut total = vec![0.0; freqs.len()];
    let mut bands: [Vec<f32>; 7] = Default::default();
    for (k, b) in BANDS.iter().enumerate() {
        if !c.engaged(b, AUDIBLE_GAIN) {
            continue;
        }
        // Only this band's parameters: every other band stays at its (flat) default.
        let own: Vec<(&str, f32)> = b.keys().map(|id| (id, c.value(id))).collect();
        let resp = soundcraft_dsp::eq7_response(&own, &freqs, c.sr);
        for (t, db) in total.iter_mut().zip(&resp) {
            *t += db;
        }
        if let Some(slot) = bands.get_mut(k) {
            *slot = resp;
        }
    }
    Curves { key, freqs, total, bands }
}

/// Cached curves for this window, recomputed only when a parameter, the rate or the width changed.
fn cached(ui: &Ui, c: Ctx, n: usize) -> Arc<Curves> {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    n.hash(&mut h);
    c.sr.to_bits().hash(&mut h);
    for p in c.info.params {
        c.current(p).to_bits().hash(&mut h);
    }
    let key = h.finish();
    let id = c.id("eq_curves", ());
    if let Some(cur) = ui.data(|d| d.get_temp::<Arc<Curves>>(id)).filter(|cur| cur.key == key) {
        return cur;
    }
    let cur = Arc::new(curves(c, n, key));
    ui.data_mut(|d| d.insert_temp(id, Arc::clone(&cur)));
    cur
}

/// Samples for a graph `width` points wide: about two per point.
fn points_for(width: f32) -> usize {
    if width.is_finite() { ((width * 2.0) as usize).clamp(400, 1600) } else { 400 }
}

/// Maps between the graph rectangle and frequency/level.
#[derive(Clone, Copy)]
struct Plot(Rect);

impl Plot {
    fn span(self) -> (f32, f32) {
        let left = self.0.left() + X_PAD;
        (left, (self.0.width() - 2.0 * X_PAD).max(0.0))
    }
    fn x(self, f: f32) -> f32 {
        let (left, w) = self.span();
        let f = if f.is_finite() { f.max(F_MIN) } else { F_MIN };
        let t = (f / F_MIN).ln() / (F_MAX / F_MIN).ln();
        left + w * t.clamp(0.0, 1.0)
    }
    /// Where a curve at `db` is drawn: beyond the range it leaves the graph (and is clipped)
    /// instead of running along the edge. Bounded so the coordinates stay sane.
    fn curve_y(self, db: f32) -> f32 {
        let db = if db.is_finite() { db.clamp(-4.0 * DB_RANGE, 4.0 * DB_RANGE) } else { 0.0 };
        self.0.center().y - db / DB_RANGE * self.0.height() * 0.5
    }
    /// Where a node at `db` sits: always inside the graph.
    fn y(self, db: f32) -> f32 {
        self.curve_y(if db.is_finite() { db.clamp(-DB_RANGE, DB_RANGE) } else { 0.0 })
    }
    fn freq(self, x: f32) -> f32 {
        let (left, w) = self.span();
        let t = if w > 0.0 { ((x - left) / w).clamp(0.0, 1.0) } else { 0.0 };
        F_MIN * (F_MAX / F_MIN).powf(t)
    }
    fn db(self, y: f32) -> f32 {
        if self.0.height() <= 0.0 {
            return 0.0;
        }
        ((self.0.center().y - y) / (self.0.height() * 0.5) * DB_RANGE).clamp(-DB_RANGE, DB_RANGE)
    }
}

fn with_alpha(c: Color32, a: f32) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), (a.clamp(0.0, 1.0) * 255.0) as u8)
}

fn hz_label(f: f32) -> String {
    if f >= 1000.0 { format!("{}k", f / 1000.0) } else { format!("{f}") }
}

/// Laid out the way Pro Tools users expect to find the controls: input/output and the two filters
/// in a column beside the graph, the five bands in a row underneath, each with Q, frequency and a
/// large gain knob from top to bottom.
pub(crate) fn show(app: &mut SoundApp, ui: &mut Ui, tid: TrackId, slot: usize, info: &'static PluginInfo, ins: &Insert) {
    let sr = app.engine.session().sample_rate.hz() as f32;
    let c = Ctx { tid, slot, info, ins, sr };
    if ins.bypass {
        // Still editable, but plainly not in the signal path.
        ui.multiply_opacity(0.45);
    }
    let width = ui.available_width().max(1.0);
    // One five-column grid for the whole window: the bands fill it below; above, HPF sits over LF,
    // LPF over LMF, input/output spans both, and the graph spans MF to HF.
    let col_w = ((width - GAP * 4.0) / 5.0).max(1.0);
    let col_x = |i: usize| i as f32 * (col_w + GAP);
    let (top, _) = ui.allocate_exact_size(vec2(width, TOP_H), Sense::hover());
    let io = Rect::from_min_size(top.min, vec2(2.0 * col_w + GAP, IO_H));
    for (i, k) in [0usize, 6].into_iter().enumerate() {
        let left = top.left() + col_x(i);
        filter_section(app, ui, c, k, Rect::from_min_max(pos2(left, io.bottom() + GAP), pos2(left + col_w, top.bottom())));
    }
    io_section(app, ui, c, io);
    let graph_rect = Rect::from_min_max(pos2((top.left() + col_x(2)).min(top.right()), top.top()), top.max);
    if graph_rect.width() > 1.0 {
        graph(app, ui, c, graph_rect);
    }
    ui.add_space(GAP);
    let (row, _) = ui.allocate_exact_size(vec2(width, BANDS_H), Sense::hover());
    for (i, k) in (1usize..=5).enumerate() {
        band_section(app, ui, c, k, Rect::from_min_size(pos2(row.left() + col_x(i), row.top()), vec2(col_w, BANDS_H)));
    }
}

/// Band `k`'s section frame, coloured top edge and title. Clicking the title row selects the band.
fn section_frame(ui: &mut Ui, c: Ctx, k: usize, r: Rect) {
    let t = Tokens::current();
    let Some(b) = BANDS.get(k) else { return };
    let color = band_color(k);
    let on = c.engaged(b, SHOWN_GAIN);
    let selected = c.selected(ui) == Some(k);
    // Sections are told apart by tone, not outlines; the band's colour marks its title tab.
    ui.painter().rect_filled(r, CornerRadius::same(4), t.eq_section);
    let tab = Rect::from_min_size(r.min, vec2(r.width(), if selected { 3.0 } else { 2.0 }));
    ui.painter().rect_filled(tab, CornerRadius { nw: 4, ne: 4, sw: 0, se: 0 }, if on { color } else { with_alpha(color, 0.35) });
    ui.painter().text(
        pos2(r.left() + PAD, r.top() + TITLE_H / 2.0 + 1.0),
        Align2::LEFT_CENTER,
        b.name,
        bold(NAME_PT),
        if on { color } else { t.text_dim },
    );
    if ui.interact(Rect::from_min_size(r.min, vec2(r.width(), TITLE_H)), c.id("eq_head", k), Sense::click()).clicked() {
        c.select(ui, k);
    }
}

/// How a knob is drawn: its label, diameter, arc colour and whether the arc starts at the centre.
struct KnobStyle<'a> {
    label: &'a str,
    size: f32,
    color: Color32,
    bipolar: bool,
}

/// A knob with its label and value beside it, in the row `r`: the knob in the left column, the
/// label above its value in the right one, both left-aligned on one edge. The value sits in a dark
/// display box, like the transport counters.
fn labeled_knob(app: &mut SoundApp, ui: &mut Ui, c: Ctx, p: &ParamInfo, r: Rect, style: &KnobStyle) {
    let t = Tokens::current();
    let knob = Rect::from_min_size(pos2(r.left() + PAD, r.center().y - style.size / 2.0), vec2(style.size, style.size));
    knob_control(app, ui, c, p, knob, style);
    let left = knob.right() + PAD;
    let right = (r.right() - PAD).max(left);
    // Label and value box, centred together on the knob.
    let top = r.center().y - (LABEL_H + 2.0 + VALUE_H) / 2.0;
    ui.painter().text(pos2(left, top + LABEL_H / 2.0), Align2::LEFT_CENTER, style.label, bold(LABEL_PT), t.text_dim);
    let value = Rect::from_min_max(pos2(left, top + LABEL_H + 2.0), pos2(right, top + LABEL_H + 2.0 + VALUE_H));
    ui.painter().rect_filled(value, CornerRadius::same(3), t.counter_bg);
    value_readout(app, ui, c, p, value);
}

/// Input and output: IN and OUT meters across the top (with clip lamps you click to clear), then
/// the gain knobs, with the polarity switch beside INPUT.
fn io_section(app: &mut SoundApp, ui: &mut Ui, c: Ctx, r: Rect) {
    let t = Tokens::current();
    let ui = &mut confined(ui, r);
    ui.painter().rect_filled(r, CornerRadius::same(4), t.eq_section);
    let meters = app.plugin_meters.get(&(c.tid, c.slot)).copied().unwrap_or_default();
    let bar_left = r.left() + PAD + METER_LABEL_W;
    let mut scale = Rect::NOTHING;
    for (i, (name, m)) in [("IN", meters[0]), ("OUT", meters[1])].into_iter().enumerate() {
        let y = r.top() + METER_TOP + i as f32 * METER_ROW;
        ui.painter().text(pos2(r.left() + PAD, y + METER_H / 2.0), Align2::LEFT_CENTER, name, bold(LABEL_PT - 1.0), t.text_dim);
        let bar = Rect::from_min_max(pos2(bar_left, y), pos2(r.right() - PAD, y + METER_H));
        let lamp = hmeter(ui, bar, m.level[0], m.hold[0], m.clip);
        scale = Rect::from_min_max(pos2(bar.left() + 1.0, bar.bottom()), pos2(lamp.left() - 3.0, bar.bottom()));
        let label = if i == 0 { "Clear input clip" } else { "Clear output clip" };
        let resp = ui.interact(lamp.expand(3.0), c.id("eq_clip", i), Sense::click());
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
        if resp.clicked()
            && let Some(d) = app.plugin_meters.get_mut(&(c.tid, c.slot)).and_then(|pair| pair.get_mut(i))
        {
            d.clip = false;
        }
    }
    // The scale under the OUT meter.
    for db in [-40.0f32, -20.0, -12.0, -6.0, -3.0, 0.0] {
        let x = scale.left() + scale.width() * meter_pos(db);
        let text = if db == 0.0 { "0".to_string() } else { format!("{db}") };
        ui.painter().text(pos2(x, scale.bottom() + 2.0), Align2::CENTER_TOP, text, regular(8.5), t.text_dim);
    }
    let knobs = Rect::from_min_max(pos2(r.left(), r.top() + IO_KNOBS_TOP), r.max);
    let half = knobs.width() / 2.0;
    for (i, (id, label)) in [("input_gain", "INPUT"), ("output_gain", "OUTPUT")].into_iter().enumerate() {
        let Some(p) = c.param(id) else { continue };
        let cell = Rect::from_min_size(pos2(knobs.left() + i as f32 * half, knobs.top()), vec2(half, knobs.height()));
        labeled_knob(app, ui, c, p, cell, &KnobStyle { label, size: SMALL_KNOB, color: t.text_dim, bipolar: true });
        if i == 0 && c.param("phase_invert").is_some() {
            // On the INPUT label's line, at the right of the cell.
            let flipped = c.value("phase_invert") >= 0.5;
            let label_y = cell.center().y - (VALUE_H + 2.0) / 2.0;
            let pr = Rect::from_center_size(pos2(cell.right() - PAD - SWITCH.x / 2.0, label_y), SWITCH);
            if pill_switch(ui, c.id("eq_phase", ()), pr, "Ø", flipped, "Phase invert").clicked() {
                commit(app, c, one("phase_invert", if flipped { 0.0 } else { 1.0 }));
            }
        }
    }
}

/// HPF or LPF: IN and filter/notch switches, then the slope (or, as a notch, its Q) and frequency.
fn filter_section(app: &mut SoundApp, ui: &mut Ui, c: Ctx, k: usize, r: Rect) {
    let Some(b) = BANDS.get(k) else { return };
    let ui = &mut confined(ui, r);
    section_frame(ui, c, k, r);
    header_switches(app, ui, c, k, r);
    let neutral = Tokens::current().text_dim;
    // The slope (or, as a notch, its Q) under the title; a large frequency knob fills the rest.
    let upper = Rect::from_min_size(pos2(r.left(), r.top() + TITLE_H), vec2(r.width(), 48.0));
    if let Some(qp) = c.q_param(b) {
        labeled_knob(app, ui, c, &qp, upper, &KnobStyle { label: "Q", size: SMALL_KNOB, color: neutral, bipolar: false });
    } else if let Some(sp) = b.slope.and_then(|s| c.param(s)) {
        slope_menu(app, ui, c, sp, Rect::from_center_size(upper.center(), vec2(r.width() - 2.0 * PAD, 24.0)));
    }
    if let Some(fp) = c.param(b.freq) {
        let lower = Rect::from_min_max(pos2(r.left(), upper.bottom()), pos2(r.right(), r.bottom() - PAD));
        labeled_knob(app, ui, c, fp, lower, &KnobStyle { label: "FREQ", size: LARGE_KNOB, color: neutral, bipolar: false });
    }
}

/// LF, LMF, MF, HMF or HF: IN (and, for LF and HF, shelf/peak) switches, then Q, frequency and a
/// large gain knob, top to bottom.
fn band_section(app: &mut SoundApp, ui: &mut Ui, c: Ctx, k: usize, r: Rect) {
    let Some(b) = BANDS.get(k) else { return };
    let ui = &mut confined(ui, r);
    section_frame(ui, c, k, r);
    header_switches(app, ui, c, k, r);
    // Q and frequency are neutral; only the large gain knob carries the band's colour.
    let neutral = Tokens::current().text_dim;
    let row = |top: f32, height: f32| Rect::from_min_size(pos2(r.left(), r.top() + top), vec2(r.width(), height));
    if let Some(qp) = c.q_param(b) {
        labeled_knob(app, ui, c, &qp, row(TITLE_H, 48.0), &KnobStyle { label: "Q", size: SMALL_KNOB, color: neutral, bipolar: false });
    }
    if let Some(fp) = c.param(b.freq) {
        labeled_knob(app, ui, c, fp, row(TITLE_H + 48.0, 48.0), &KnobStyle { label: "FREQ", size: SMALL_KNOB, color: neutral, bipolar: false });
    }
    if let Some(gp) = b.gain.and_then(|g| c.param(g)) {
        let gain = Rect::from_min_max(pos2(r.left(), r.top() + TITLE_H + 96.0), pos2(r.right(), r.bottom() - PAD));
        labeled_knob(app, ui, c, gp, gain, &KnobStyle { label: "GAIN", size: LARGE_KNOB, color: band_color(k), bipolar: true });
    }
}

/// The right of a section's title row: the type buttons (if the band has a choice), then IN.
fn header_switches(app: &mut SoundApp, ui: &mut Ui, c: Ctx, k: usize, r: Rect) {
    let Some(b) = BANDS.get(k) else { return };
    let is_on = c.value(b.on) >= 0.5;
    let in_rect = Rect::from_min_size(pos2(r.right() - PAD - SWITCH.x, r.top() + (TITLE_H - SWITCH.y) / 2.0), SWITCH);
    if pill_switch(ui, c.id("eq_in", k), in_rect, "IN", is_on, &format!("{} in", b.name)).clicked() {
        commit(app, c, one(b.on, if is_on { 0.0 } else { 1.0 }));
    }
    let Some((kind, shapes)) = b.kind else { return };
    let Some(kp) = c.param(kind) else { return };
    let chosen = usize::from(c.second_kind(b));
    for (i, shape) in shapes.into_iter().enumerate() {
        let x = in_rect.left() - 6.0 - (2 - i) as f32 * (TYPE_BUTTON.x + 2.0);
        let rect = Rect::from_min_size(pos2(x, in_rect.top()), TYPE_BUTTON);
        let label = format!("{} {}", b.name, kp.choices.get(i).copied().unwrap_or(""));
        if type_button(ui, c.id("eq_type", (k, i)), rect, shape, chosen == i, &label).clicked() && chosen != i {
            commit(app, c, one(kind, i as f32));
        }
    }
}

/// A small button showing a filter shape; the selected one is lit with the accent blue.
fn type_button(ui: &mut Ui, id: Id, r: Rect, shape: Shape, selected: bool, label: &str) -> egui::Response {
    let t = Tokens::current();
    let resp = ui.interact(r, id, Sense::click());
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::RadioButton, true, selected, label));
    let fill = if selected {
        t.accent
    } else if resp.hovered() {
        t.button_hi
    } else {
        t.button
    };
    let p = ui.painter();
    p.rect_filled(r, CornerRadius::same(3), fill);
    if resp.has_focus() {
        p.rect_stroke(r.expand(2.0), CornerRadius::same(3), Stroke::new(1.0, t.accent), StrokeKind::Outside);
    }
    // The shape in a unit square: x 0..1 left to right, y 0 (low) .. 1 (high).
    let pts: &[(f32, f32)] = match shape {
        Shape::HighPass => &[(0.05, 0.1), (0.4, 0.75), (0.95, 0.75)],
        Shape::LowPass => &[(0.05, 0.75), (0.6, 0.75), (0.95, 0.1)],
        Shape::LowShelf => &[(0.05, 0.8), (0.35, 0.8), (0.65, 0.35), (0.95, 0.35)],
        Shape::HighShelf => &[(0.05, 0.35), (0.35, 0.35), (0.65, 0.8), (0.95, 0.8)],
        Shape::Peak => &[(0.05, 0.35), (0.35, 0.4), (0.5, 0.85), (0.65, 0.4), (0.95, 0.35)],
        Shape::Notch => &[(0.05, 0.75), (0.38, 0.7), (0.5, 0.1), (0.62, 0.7), (0.95, 0.75)],
    };
    let inner = r.shrink(3.0);
    let line: Vec<Pos2> = pts.iter().map(|(x, y)| pos2(inner.left() + x * inner.width(), inner.bottom() - y * inner.height())).collect();
    p.add(egui::Shape::line(line, Stroke::new(1.2, if selected { Color32::WHITE } else { t.text_dim })));
    resp.on_hover_text(label)
}

fn graph(app: &mut SoundApp, ui: &mut Ui, c: Ctx, outer: Rect) {
    let t = Tokens::current();
    ui.painter().rect_filled(outer, CornerRadius::same(4), t.eq_bg);
    let plot =
        Plot(Rect::from_min_max(pos2(outer.left() + AXIS_W, outer.top() + PLOT_INSET), pos2(outer.right() - PLOT_INSET, outer.bottom() - AXIS_H)));
    let r = plot.0;
    let painter = ui.painter().with_clip_rect(outer);
    // Grid: decades and their 2/5 steps, every 6 dB.
    for f in [10.0, 20.0, 30.0, 40.0, 50.0, 100.0, 200.0, 300.0, 400.0, 500.0, 1000.0, 2000.0, 3000.0, 4000.0, 5000.0, 10_000.0, 20_000.0] {
        let x = plot.x(f);
        painter.line_segment([pos2(x, r.top()), pos2(x, r.bottom())], Stroke::new(1.0, t.eq_grid));
    }
    for f in [10.0, 20.0, 50.0, 100.0, 200.0, 500.0, 1000.0, 2000.0, 5000.0, 10_000.0, 20_000.0] {
        painter.text(pos2(plot.x(f), r.bottom() + 4.0), Align2::CENTER_TOP, hz_label(f), regular(11.0), t.text_dim);
    }
    for db in [-18.0f32, -12.0, -6.0, 0.0, 6.0, 12.0, 18.0] {
        let y = plot.y(db);
        let stroke = if db == 0.0 { Stroke::new(1.0, t.eq_zero) } else { Stroke::new(1.0, t.eq_grid) };
        painter.line_segment([pos2(r.left(), y), pos2(r.right(), y)], stroke);
        let label = if db > 0.0 { format!("+{db}") } else { format!("{db}") };
        painter.text(pos2(r.left() - 6.0, y), Align2::RIGHT_CENTER, label, regular(11.0), t.text_dim);
    }

    let cur = cached(ui, c, points_for(r.width()));
    let curve = painter.with_clip_rect(r);
    let sel = c.selected(ui);
    // Each band in use filled toward 0 dB.
    let zero = plot.y(0.0);
    for (k, b) in BANDS.iter().enumerate() {
        let Some(resp) = cur.bands.get(k).filter(|r| !r.is_empty() && c.engaged(b, SHOWN_GAIN)) else { continue };
        let mut mesh = Mesh::default();
        let fill = with_alpha(band_color(k), if sel == Some(k) { 0.30 } else { 0.16 });
        for (i, (f, db)) in cur.freqs.iter().zip(resp).enumerate() {
            let x = plot.x(*f);
            mesh.colored_vertex(pos2(x, plot.curve_y(*db)), fill);
            mesh.colored_vertex(pos2(x, zero), fill);
            if i > 0 {
                let v = (2 * i) as u32;
                mesh.add_triangle(v - 2, v - 1, v);
                mesh.add_triangle(v - 1, v, v + 1);
            }
        }
        curve.add(mesh);
    }
    let line: Vec<Pos2> = cur.freqs.iter().zip(&cur.total).map(|(f, db)| pos2(plot.x(*f), plot.curve_y(*db))).collect();
    curve.add(egui::Shape::line(line, Stroke::new(2.0, t.eq_curve)));

    // Nodes, drawn and hit-tested last so they sit above the curves.
    for k in 0..BANDS.len() {
        node(app, ui, c, plot, k);
    }
}

/// Band `k`'s node: drag for frequency and gain, scroll or pinch for Q, double-click to reset. A
/// filter's node sits on the 0 dB line.
fn node(app: &mut SoundApp, ui: &mut Ui, c: Ctx, plot: Plot, k: usize) {
    let t = Tokens::current();
    let Some(b) = BANDS.get(k) else { return };
    let Some(fp) = c.param(b.freq) else { return };
    let freq = c.current(fp);
    let gp = b.gain.and_then(|g| c.param(g));
    let gain = gp.map(|p| c.current(p));
    let center = pos2(plot.x(freq), plot.y(gain.unwrap_or(0.0)));
    let resp = ui.interact(Rect::from_center_size(center, vec2(NODE_HIT, NODE_HIT)), c.id("eq_node", k), Sense::click_and_drag());
    resp.widget_info(|| egui::WidgetInfo::slider(true, f64::from(freq), format!("{} node", b.name)));
    let fine = ui.input(|i| i.modifiers.shift);

    let mut next = center;
    if resp.dragged() {
        next += resp.drag_delta() * if fine { 0.2 } else { 1.0 };
    }
    if resp.has_focus() {
        let step = if fine { 1.0 } else { 6.0 };
        let keys = [
            (egui::Key::ArrowLeft, vec2(-step, 0.0)),
            (egui::Key::ArrowRight, vec2(step, 0.0)),
            (egui::Key::ArrowUp, vec2(0.0, -step)),
            (egui::Key::ArrowDown, vec2(0.0, step)),
        ];
        ui.input(|i| {
            for (key, d) in keys {
                if i.key_pressed(key) {
                    next += d;
                }
            }
        });
    }
    let mut changes = Map::new();
    let f = fp.clamp(plot.freq(next.x));
    if next.x != center.x && f != freq {
        changes.insert(b.freq.to_string(), json!(f));
    }
    if let (Some(gp), Some(g)) = (gp, gain) {
        let v = gp.clamp(plot.db(next.y));
        if next.y != center.y && v != g {
            changes.insert(gp.id.to_string(), json!(v));
        }
    }
    gesture(app, c, &format!("eqnode:{}:{}:{k}", c.tid.0, c.slot), changes);

    // Scroll or pinch over the node sets Q.
    if resp.hovered()
        && let Some(qp) = c.q_param(b)
    {
        let (scroll, zoom) = ui.input(|i| (i.smooth_scroll_delta.y, i.zoom_delta()));
        let delta = scroll * 0.002 + (zoom - 1.0);
        if delta.abs() > 1e-4 {
            let q = qp.from_normalized(qp.to_normalized(c.current(&qp)) + delta);
            gesture(app, c, &format!("eqq:{}:{}:{k}", c.tid.0, c.slot), one(qp.id, q));
            ui.input_mut(|i| i.smooth_scroll_delta = egui::Vec2::ZERO);
        }
    }
    if resp.double_clicked() {
        let defaults = [Some(b.freq), b.gain, Some(b.q), b.slope].into_iter().flatten().filter_map(|id| c.param(id));
        commit(app, c, defaults.map(|p| (p.id.to_string(), json!(p.default))).collect());
    } else if resp.clicked() || resp.drag_started() {
        c.select(ui, k);
    }
    if resp.drag_stopped() {
        app.engine.end_merge();
    }

    let selected = c.selected(ui) == Some(k);
    let color = band_color(k);
    let rad = if resp.hovered() || resp.dragged() || selected { NODE_R + 1.0 } else { NODE_R };
    let fill = if c.engaged(b, SHOWN_GAIN) { color } else { with_alpha(color, 0.35) };
    ui.painter().circle(center, rad, fill, Stroke::new(if selected { 2.0 } else { 1.5 }, t.eq_curve));
    if resp.has_focus() {
        ui.painter().circle_stroke(center, rad + 4.0, Stroke::new(1.0, t.accent));
    }
    if !resp.dragged() {
        // Built only while hovered.
        resp.on_hover_ui(|ui| {
            let mut tip = format!("{}  {}", b.name, fp.format(freq));
            if let (Some(gp), Some(g)) = (gp, gain) {
                tip.push_str(&format!("  {}", gp.format(g)));
            }
            if let Some(qp) = c.q_param(b) {
                tip.push_str(&format!("  Q {}", qp.format(c.current(&qp))));
            }
            ui.label(tip);
        });
    }
}

/// A knob bound to one parameter; a drag (or a run of key presses) is one undo step.
fn knob_control(app: &mut SoundApp, ui: &mut Ui, c: Ctx, p: &ParamInfo, r: Rect, style: &KnobStyle) {
    let v = c.current(p);
    let (resp, next) = param_knob(ui, r, c.id("eq_knob", p.id), p.to_normalized(v), p.to_normalized(p.default), style.bipolar, style.color);
    resp.widget_info(|| egui::WidgetInfo::slider(true, f64::from(v), p.name));
    if let Some(n) = next {
        gesture(app, c, &format!("eqknob:{}:{}:{}", c.tid.0, c.slot, p.id), one(p.id, p.from_normalized(n)));
    }
    if resp.drag_stopped() || resp.double_clicked() {
        app.engine.end_merge();
    }
    resp.on_hover_ui(|ui| {
        ui.label(format!("{}: {}", p.name, p.format(v)));
    });
}

/// The value beside a knob; click to type a new one (Enter commits, Escape cancels).
fn value_readout(app: &mut SoundApp, ui: &mut Ui, c: Ctx, p: &ParamInfo, r: Rect) {
    let v = c.current(p);
    let (parse, initial): (fn(&str) -> Option<f32>, String) = match p.unit {
        soundcraft_dsp::Unit::Hz => (parse_hz, format!("{v:.0}")),
        _ => (parse_number, format!("{v:.2}")),
    };
    let (_, entered) = editable_readout(ui, r, c.id("eq_value", p.id), &p.format(v), &initial, parse, VALUE_PT);
    if let Some(x) = entered {
        commit(app, c, one(p.id, p.clamp(x)));
    }
}

fn slope_menu(app: &mut SoundApp, ui: &mut Ui, c: Ctx, p: &ParamInfo, r: Rect) {
    let mut idx = c.current(p).round().max(0.0) as usize;
    let before = idx;
    let label = p.choices.get(idx).copied().unwrap_or("?");
    egui::ComboBox::from_id_salt(c.id("eq_slope", p.id)).width(r.width()).truncate().selected_text(label).show_ui(&mut confined(ui, r), |ui| {
        for (i, choice) in p.choices.iter().enumerate() {
            ui.selectable_value(&mut idx, i, *choice);
        }
    });
    if idx != before {
        commit(app, c, one(p.id, idx as f32));
    }
}

/// "1200", "1.2k", "1.2 kHz", "80 Hz".
pub(crate) fn parse_hz(s: &str) -> Option<f32> {
    let s = s.trim().to_ascii_lowercase();
    let s = s.trim_end_matches("hz").trim();
    let (num, mult) = match s.strip_suffix('k') {
        Some(n) => (n.trim(), 1000.0),
        None => (s, 1.0),
    };
    num.parse::<f32>().ok().map(|v| v * mult).filter(|v| v.is_finite())
}

/// "+3.5", "-2 dB", "0.7".
pub(crate) fn parse_number(s: &str) -> Option<f32> {
    let s = s.trim().to_ascii_lowercase();
    let s = s.trim_end_matches("db").trim().trim_start_matches('+');
    s.parse::<f32>().ok().filter(|v| v.is_finite())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48_000.0;

    fn info() -> &'static PluginInfo {
        &soundcraft_dsp::plugins::eq::EQ7_INFO
    }

    fn insert(params: &[(&str, f32)]) -> Insert {
        let mut ins = Insert::new("eq_7band");
        for (k, v) in params {
            ins.params.insert((*k).to_string(), *v);
        }
        ins
    }

    fn ctx(ins: &Insert, sr: f32) -> Ctx<'_> {
        Ctx { tid: TrackId(1), slot: 0, info: info(), ins, sr }
    }

    #[test]
    fn frequencies_and_numbers_parse_with_units() {
        assert_eq!(parse_hz("1200"), Some(1200.0));
        assert_eq!(parse_hz("1.2k"), Some(1200.0));
        assert_eq!(parse_hz(" 1.2 kHz "), Some(1200.0));
        assert_eq!(parse_hz("80 Hz"), Some(80.0));
        assert_eq!(parse_hz("abc"), None);
        assert_eq!(parse_hz("inf"), None);
        assert_eq!(parse_number("+3.5"), Some(3.5));
        assert_eq!(parse_number("-2 dB"), Some(-2.0));
        assert_eq!(parse_number("NaN"), None);
    }

    #[test]
    fn plot_maps_round_trip_and_clamp() {
        let p = Plot(Rect::from_min_size(pos2(0.0, 0.0), vec2(1000.0, 200.0)));
        assert!((p.freq(p.x(1000.0)) - 1000.0).abs() < 0.5);
        assert!((p.db(p.y(6.0)) - 6.0).abs() < 1e-3);
        assert_eq!(p.y(f32::NAN), p.y(0.0));
        assert_eq!(p.freq(-50.0), F_MIN);
        assert_eq!(p.db(-1000.0), DB_RANGE);
        // A curve below the range is drawn below the graph, not along its bottom edge.
        assert!(p.curve_y(-60.0) > 200.0);
        assert_eq!(p.y(-60.0), 200.0);
        assert_eq!(p.curve_y(f32::NAN), p.y(0.0));
        assert!(p.curve_y(-1e9).is_finite());
        let empty = Plot(Rect::from_min_size(pos2(0.0, 0.0), vec2(0.0, 0.0)));
        assert_eq!(empty.db(10.0), 0.0);
        assert_eq!(empty.freq(10.0), F_MIN);
    }

    #[test]
    fn a_narrow_peak_is_drawn_through_its_top_and_symmetric() {
        let f = 665.9;
        let ins = insert(&[("mid_freq", f), ("mid_gain", 24.0), ("mid_q", 10.0)]);
        let c = curves(ctx(&ins, SR), points_for(680.0), 0);
        let (imax, max) = c.total.iter().copied().enumerate().max_by(|a, b| a.1.total_cmp(&b.1)).unwrap();
        assert!((c.freqs[imax] - f).abs() < 1e-3, "the top is the band's own sample, not {}", c.freqs[imax]);
        assert!((max - 24.0).abs() < 0.1, "{max}");
        // Symmetric on the log axis: the same level a little above and below.
        let k = 1.05_f32;
        let up = soundcraft_dsp::eq7_response(&[("mid_freq", f), ("mid_gain", 24.0), ("mid_q", 10.0)], &[f * k, f / k], SR);
        assert!((up[0] - up[1]).abs() < 0.2, "{up:?}");
    }

    #[test]
    fn the_summed_curve_matches_the_dsp_response_at_the_session_rate() {
        let ins = insert(&[
            ("input_gain", 2.0),
            ("output_gain", -3.5),
            ("hpf_on", 1.0),
            ("hpf_freq", 10.0),
            ("hpf_slope", 3.0),
            ("low_shelf_gain", 4.0),
            ("low_mid_gain", -6.0),
            ("mid_gain", 9.0),
            ("mid_q", 7.0),
            ("high_shelf_gain", -2.0),
            ("lpf_on", 1.0),
            ("lpf_freq", 18_000.0),
            ("lpf_slope", 2.0),
        ]);
        for sr in [44_100.0, 48_000.0, 96_000.0] {
            let c = ctx(&ins, sr);
            let cur = curves(c, 500, 0);
            let all: Vec<(&str, f32)> = info().params.iter().map(|p| (p.id, c.current(p))).collect();
            // The graph leaves out input and output gain (+2 and -3.5 here).
            let reference: Vec<f32> = soundcraft_dsp::eq7_response(&all, &cur.freqs, sr).iter().map(|db| db - (2.0 - 3.5)).collect();
            for ((f, a), b) in cur.freqs.iter().zip(&cur.total).zip(&reference) {
                assert!((a - b).abs() < 1e-3, "{sr} Hz rate, at {f} Hz: summed {a}, dsp {b}");
            }
            assert!(cur.freqs.iter().all(|f| *f < sr * 0.5), "nothing at or above Nyquist");
            // Flat bands are skipped, not computed.
            assert!(cur.bands[4].is_empty(), "HMF at 0 dB is flat");
        }
    }

    #[test]
    fn the_axis_ends_leave_room_for_a_node() {
        let p = Plot(Rect::from_min_size(pos2(0.0, 0.0), vec2(600.0, 200.0)));
        assert!(p.x(10.0) >= X_PAD - 1e-3);
        assert!(p.x(20_000.0) <= 600.0 - X_PAD + 1e-3);
        assert!(p.x(10.0) < p.x(20.0), "10 Hz is on the graph");
        for f in [10.0, 47.0, 665.9, 19_000.0] {
            assert!((p.freq(p.x(f)) / f - 1.0).abs() < 1e-3, "{f}");
        }
    }

    #[test]
    fn a_shelf_q_knob_spans_only_the_range_a_shelf_uses() {
        let lf = &BANDS[1];
        let shelf = insert(&[("low_shelf_q", 8.0)]);
        let q = ctx(&shelf, SR).q_param(lf).unwrap();
        assert_eq!((q.min, q.max), soundcraft_dsp::plugins::eq::SHELF_Q);
        assert_eq!(ctx(&shelf, SR).current(&q), 2.0, "a Q set as a peak reads as the shelf hears it");
        let peak = insert(&[("low_shelf_q", 8.0), ("low_shelf_type", 1.0)]);
        let q = ctx(&peak, SR).q_param(lf).unwrap();
        assert_eq!((q.min, q.max, ctx(&peak, SR).current(&q)), (0.1, 10.0, 8.0));
        // A filter has a Q only as a notch.
        let hpf = &BANDS[0];
        assert!(ctx(&insert(&[]), SR).q_param(hpf).is_none());
        assert!(ctx(&insert(&[("hpf_type", 1.0)]), SR).q_param(hpf).is_some());
    }

    #[test]
    fn every_band_names_real_parameters() {
        for b in &BANDS {
            for id in b.keys() {
                assert!(info().param(id).is_some(), "{id}");
            }
        }
    }
}
