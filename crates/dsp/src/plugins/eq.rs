//! Parametric equalizers: EQ 1-Band and EQ 7-Band (HPF or low notch, low shelf or peak, three
//! peaks, high shelf or peak, LPF or high notch; every band switchable; polarity invert).

use crate::biquad::{Biquad, Coeffs, FilterType, butterworth_qs};
use crate::params::{Params, choice, lin, log, toggle};
use crate::util::{Smoother, frames_in, sane_channels, sane_sr};
use crate::{Category, ParamInfo, Plugin, PluginInfo, Unit, db_to_gain};

const SLOPES: &[&str] = &["6 dB/oct", "12 dB/oct", "18 dB/oct", "24 dB/oct"];
const SHELF_TYPES: &[&str] = &["Shelf", "Peak"];
const HPF_TYPES: &[&str] = &["High Pass", "Notch"];
const LPF_TYPES: &[&str] = &["Low Pass", "Notch"];

// Parameters are addressed by position in `design_eq7`: new ones go at the end, with defaults that
// sound exactly as before they existed, so sessions and presets saved earlier are unchanged.
static EQ7_PARAMS: [ParamInfo; 35] = [
    lin(crate::INPUT_GAIN, "Input", -24.0, 24.0, 0.0, Unit::Db),
    toggle("hpf_on", "HPF", false),
    log("hpf_freq", "HPF Freq", 10.0, 8000.0, 80.0, Unit::Hz),
    choice("hpf_slope", "HPF Slope", SLOPES, 1),
    log("low_shelf_freq", "LF Freq", 20.0, 1000.0, 100.0, Unit::Hz),
    lin("low_shelf_gain", "LF Gain", -24.0, 24.0, 0.0, Unit::Db),
    log("low_shelf_q", "LF Q", 0.1, 10.0, 0.707, Unit::None),
    log("low_mid_freq", "LMF Freq", 20.0, 2000.0, 250.0, Unit::Hz),
    lin("low_mid_gain", "LMF Gain", -24.0, 24.0, 0.0, Unit::Db),
    log("low_mid_q", "LMF Q", 0.1, 10.0, 1.0, Unit::None),
    log("mid_freq", "MF Freq", 100.0, 8000.0, 1000.0, Unit::Hz),
    lin("mid_gain", "MF Gain", -24.0, 24.0, 0.0, Unit::Db),
    log("mid_q", "MF Q", 0.1, 10.0, 1.0, Unit::None),
    log("high_mid_freq", "HMF Freq", 500.0, 20000.0, 4000.0, Unit::Hz),
    lin("high_mid_gain", "HMF Gain", -24.0, 24.0, 0.0, Unit::Db),
    log("high_mid_q", "HMF Q", 0.1, 10.0, 1.0, Unit::None),
    log("high_shelf_freq", "HF Freq", 1000.0, 20000.0, 8000.0, Unit::Hz),
    lin("high_shelf_gain", "HF Gain", -24.0, 24.0, 0.0, Unit::Db),
    log("high_shelf_q", "HF Q", 0.1, 10.0, 0.707, Unit::None),
    toggle("lpf_on", "LPF", false),
    log("lpf_freq", "LPF Freq", 120.0, 20000.0, 18000.0, Unit::Hz),
    choice("lpf_slope", "LPF Slope", SLOPES, 1),
    lin("output_gain", "Output", -24.0, 24.0, 0.0, Unit::Db),
    toggle("low_shelf_on", "LF In", true),
    toggle("low_mid_on", "LMF In", true),
    toggle("mid_on", "MF In", true),
    toggle("high_mid_on", "HMF In", true),
    toggle("high_shelf_on", "HF In", true),
    choice("low_shelf_type", "LF Type", SHELF_TYPES, 0),
    choice("high_shelf_type", "HF Type", SHELF_TYPES, 0),
    choice("hpf_type", "HPF Type", HPF_TYPES, 0),
    log("hpf_q", "HPF Notch Q", 0.1, 10.0, 1.0, Unit::None),
    choice("lpf_type", "LPF Type", LPF_TYPES, 0),
    log("lpf_q", "LPF Notch Q", 0.1, 10.0, 1.0, Unit::None),
    toggle("phase_invert", "Phase Invert", false),
];

/// Indices of the parameters added after the first 23 (see `EQ7_PARAMS`).
const EQ7_BAND_ON: [usize; 5] = [23, 24, 25, 26, 27];
const EQ7_LF_TYPE: usize = 28;
const EQ7_HF_TYPE: usize = 29;
const EQ7_HPF_TYPE: usize = 30;
const EQ7_HPF_Q: usize = 31;
const EQ7_LPF_TYPE: usize = 32;
const EQ7_LPF_Q: usize = 33;
const EQ7_PHASE: usize = 34;
/// The Q range a shelf uses (the LF/HF Q parameters span a wider range for the peak setting; as
/// a shelf their value is clamped to this, the range they always had).
pub const SHELF_Q: (f32, f32) = (0.3, 2.0);

pub static EQ7_INFO: PluginInfo = PluginInfo {
    id: "eq_7band",
    name: "EQ 7-Band",
    short_name: "EQ7",
    category: Category::Eq,
    params: &EQ7_PARAMS,
    is_instrument: false,
    audiosuite: true,
};

const EQ1_TYPES: &[&str] = &["Peak", "Low Shelf", "High Shelf", "High Pass", "Low Pass", "Notch", "Band Pass"];

static EQ1_PARAMS: [ParamInfo; 5] = [
    choice("type", "Type", EQ1_TYPES, 0),
    log("freq", "Freq", 20.0, 20000.0, 1000.0, Unit::Hz),
    lin("gain", "Gain", -24.0, 24.0, 0.0, Unit::Db),
    log("q", "Q", 0.1, 10.0, 1.0, Unit::None),
    lin("output_gain", "Output", -24.0, 24.0, 0.0, Unit::Db),
];

pub static EQ1_INFO: PluginInfo = PluginInfo {
    id: "eq_1band",
    name: "EQ 1-Band",
    short_name: "EQ1",
    category: Category::Eq,
    params: &EQ1_PARAMS,
    is_instrument: false,
    audiosuite: true,
};

#[inline]
fn val(v: &[f32], i: usize) -> f32 {
    v.get(i).copied().unwrap_or(0.0)
}

/// Writes 1–2 cascaded sections for an HPF/LPF of the given slope index (6/12/18/24 dB/oct).
fn pass_filter(slope: usize, high: bool, freq: f32, sr: f32) -> [Coeffs; 2] {
    let (k1, k2) = if high { (FilterType::HighPass1, FilterType::HighPass) } else { (FilterType::LowPass1, FilterType::LowPass) };
    match slope {
        0 => [Coeffs::design(k1, freq, 0.707, 0.0, sr), Coeffs::IDENTITY],
        2 => [Coeffs::design(k1, freq, 0.707, 0.0, sr), Coeffs::design(k2, freq, 1.0, 0.0, sr)],
        3 => {
            let q = butterworth_qs(4);
            let q0 = q.first().copied().unwrap_or(0.54);
            let q1 = q.get(1).copied().unwrap_or(1.31);
            [Coeffs::design(k2, freq, q0, 0.0, sr), Coeffs::design(k2, freq, q1, 0.0, sr)]
        }
        _ => [Coeffs::design(k2, freq, std::f32::consts::FRAC_1_SQRT_2, 0.0, sr), Coeffs::IDENTITY],
    }
}

/// Whether a two-way type choice (shelf/peak, filter/notch) is set to its second option.
fn second_choice(v: &[f32], i: usize) -> bool {
    val(v, i).round() as usize == 1
}

/// HPF or LPF sections: the filter at its slope, or a notch at its frequency and Q.
fn pass_or_notch(v: &[f32], high: bool, sr: f32) -> [Coeffs; 2] {
    let (freq, slope, kind, q) = if high { (2, 3, EQ7_HPF_TYPE, EQ7_HPF_Q) } else { (20, 21, EQ7_LPF_TYPE, EQ7_LPF_Q) };
    if second_choice(v, kind) {
        [Coeffs::design(FilterType::Notch, val(v, freq), val(v, q), 0.0, sr), Coeffs::IDENTITY]
    } else {
        pass_filter(val(v, slope).round() as usize, high, val(v, freq), sr)
    }
}

fn design_eq7(v: &[f32], sr: f32) -> [Coeffs; 9] {
    let mut c = [Coeffs::IDENTITY; 9];
    if val(v, 1) >= 0.5 {
        let [a, b] = pass_or_notch(v, true, sr);
        c[0] = a;
        c[1] = b;
    }
    let bands = [
        (4, if second_choice(v, EQ7_LF_TYPE) { FilterType::Peak } else { FilterType::LowShelf }),
        (7, FilterType::Peak),
        (10, FilterType::Peak),
        (13, FilterType::Peak),
        (16, if second_choice(v, EQ7_HF_TYPE) { FilterType::Peak } else { FilterType::HighShelf }),
    ];
    for (k, (i, kind)) in bands.into_iter().enumerate() {
        let on = EQ7_BAND_ON.get(k).is_none_or(|&o| val(v, o) >= 0.5);
        let g = val(v, i + 1);
        let q = val(v, i + 2);
        let q = if matches!(kind, FilterType::LowShelf | FilterType::HighShelf) { q.clamp(SHELF_Q.0, SHELF_Q.1) } else { q };
        if on
            && g.abs() > 1e-4
            && let Some(slot) = c.get_mut(2 + k)
        {
            *slot = Coeffs::design(kind, val(v, i), q, g, sr);
        }
    }
    if val(v, 19) >= 0.5 {
        let [a, b] = pass_or_notch(v, false, sr);
        c[7] = a;
        c[8] = b;
    }
    c
}

fn design_eq1(v: &[f32], sr: f32) -> [Coeffs; 1] {
    let kind = match val(v, 0).round() as usize {
        1 => FilterType::LowShelf,
        2 => FilterType::HighShelf,
        3 => FilterType::HighPass,
        4 => FilterType::LowPass,
        5 => FilterType::Notch,
        6 => FilterType::BandPass,
        _ => FilterType::Peak,
    };
    let g = val(v, 2);
    if kind == FilterType::Peak && g.abs() < 1e-4 {
        return [Coeffs::IDENTITY];
    }
    [Coeffs::design(kind, val(v, 1), val(v, 3), g, sr)]
}

/// Generic smoothed EQ: `S` cascaded sections designed from the parameter values.
pub struct Eq<const S: usize> {
    info: &'static PluginInfo,
    design: fn(&[f32], f32) -> [Coeffs; S],
    gain_ids: &'static [usize],
    /// The polarity switch, if this EQ has one.
    invert_id: Option<usize>,
    p: Params,
    sm: Vec<Smoother>,
    vals: Vec<f32>,
    coeffs: [Coeffs; S],
    filt: Vec<[Biquad; S]>,
    gain: Smoother,
    sr: f32,
    /// A parameter changed since the sections were last designed. Switches and choices snap, so
    /// no smoother settles to trigger the redesign for them (#239).
    dirty: bool,
}

/// EQ 7-Band.
pub type Eq7 = Eq<9>;
/// EQ 1-Band.
pub type Eq1 = Eq<1>;

impl Eq7 {
    pub fn new() -> Self {
        Eq::build(&EQ7_INFO, design_eq7, &[0, 22], Some(EQ7_PHASE))
    }
}

impl Default for Eq7 {
    fn default() -> Self {
        Self::new()
    }
}

impl Eq1 {
    pub fn new() -> Self {
        Eq::build(&EQ1_INFO, design_eq1, &[4], None)
    }
}

impl Default for Eq1 {
    fn default() -> Self {
        Self::new()
    }
}

impl<const S: usize> Eq<S> {
    fn build(info: &'static PluginInfo, design: fn(&[f32], f32) -> [Coeffs; S], gain_ids: &'static [usize], invert_id: Option<usize>) -> Self {
        let p = Params::new(info.params);
        let sm = info.params.iter().map(|i| Smoother::new(i.default)).collect();
        let vals = info.params.iter().map(|i| i.default).collect();
        let mut eq = Self {
            info,
            design,
            gain_ids,
            invert_id,
            p,
            sm,
            vals,
            coeffs: [Coeffs::IDENTITY; S],
            filt: Vec::new(),
            gain: Smoother::new(1.0),
            sr: 48_000.0,
            dirty: false,
        };
        eq.redesign();
        eq
    }

    fn total_gain_db(&self, v: impl Fn(usize) -> f32) -> f32 {
        self.gain_ids.iter().map(|&i| v(i)).sum()
    }

    fn update(&mut self) {
        for (i, (s, info)) in self.sm.iter_mut().zip(self.info.params).enumerate() {
            s.set(self.p.v(i));
            if matches!(info.unit, Unit::Toggle | Unit::Choice) {
                s.snap();
            }
        }
        // Polarity rides on the smoothed output gain, so flipping it ramps through zero instead of
        // clicking.
        let sign = if self.invert_id.is_some_and(|i| self.p.on(i)) { -1.0 } else { 1.0 };
        let g = sign * db_to_gain(self.total_gain_db(|i| self.p.v(i)));
        self.gain.set(g);
        self.dirty = true;
    }

    fn redesign(&mut self) {
        for (v, s) in self.vals.iter_mut().zip(&self.sm) {
            *v = s.value();
        }
        self.coeffs = (self.design)(&self.vals, self.sr);
        for f in &mut self.filt {
            for (b, c) in f.iter_mut().zip(&self.coeffs) {
                b.coeffs = *c;
            }
        }
        self.dirty = false;
    }

    /// Magnitude response (dB, including input/output gain) of the current settings.
    pub fn response_db(&self, freqs: &[f32], sample_rate: f32) -> Vec<f32> {
        let coeffs = (self.design)(&(0..self.info.params.len()).map(|i| self.p.v(i)).collect::<Vec<_>>(), sample_rate);
        let g = self.total_gain_db(|i| self.p.v(i));
        freqs.iter().map(|&f| g + coeffs.iter().map(|c| c.magnitude_db(f, sample_rate)).sum::<f32>()).collect()
    }
}

impl<const S: usize> Plugin for Eq<S> {
    fn info(&self) -> &'static PluginInfo {
        self.info
    }

    fn set_param(&mut self, id: &str, value: f32) -> bool {
        if self.p.set(id, value).is_some() {
            self.update();
            true
        } else {
            false
        }
    }

    fn param(&self, id: &str) -> Option<f32> {
        self.p.get(id)
    }

    fn prepare(&mut self, sample_rate: f32, _max_block: usize, channels: usize) {
        self.sr = sane_sr(sample_rate);
        for s in &mut self.sm {
            s.set_time(20.0, self.sr);
        }
        self.gain.set_time(20.0, self.sr);
        self.filt = vec![[Biquad::default(); S]; sane_channels(channels)];
        self.update();
        self.sm.iter_mut().for_each(Smoother::snap);
        self.gain.snap();
        self.redesign();
    }

    fn reset(&mut self) {
        self.filt.iter_mut().flatten().for_each(Biquad::reset);
        self.sm.iter_mut().for_each(Smoother::snap);
        self.gain.snap();
        self.redesign();
    }

    fn process(&mut self, io: &mut [Vec<f32>], frames: usize) {
        const CHUNK: usize = 32;
        let frames = frames_in(io, frames, self.filt.len());
        let mut pos = 0;
        while pos < frames {
            let n = CHUNK.min(frames - pos);
            if self.dirty || self.sm.iter().any(Smoother::settling) {
                self.sm.iter_mut().for_each(|s| {
                    s.advance(n);
                });
                self.redesign();
            }
            let mut gains = [0.0f32; CHUNK];
            for g in gains.iter_mut().take(n) {
                *g = self.gain.next();
            }
            for (buf, f) in io.iter_mut().zip(self.filt.iter_mut()) {
                let Some(seg) = buf.get_mut(pos..pos + n) else { continue };
                for (x, g) in seg.iter_mut().zip(&gains) {
                    let mut y = *x;
                    for b in f.iter_mut() {
                        y = b.process_sample(y);
                    }
                    *x = y * g;
                }
            }
            pos += n;
        }
    }
}

fn response_from(
    info: &'static PluginInfo,
    design: &dyn Fn(&[f32], f32) -> Vec<Coeffs>,
    gain_ids: &[usize],
    params: &[(&str, f32)],
    freqs: &[f32],
    sr: f32,
) -> Vec<f32> {
    let mut p = Params::new(info.params);
    for (id, v) in params {
        let _ = p.set(id, *v);
    }
    let vals: Vec<f32> = (0..info.params.len()).map(|i| p.v(i)).collect();
    let coeffs = design(&vals, sr);
    let g: f32 = gain_ids.iter().map(|&i| p.v(i)).sum();
    freqs.iter().map(|&f| g + coeffs.iter().map(|c| c.magnitude_db(f, sr)).sum::<f32>()).collect()
}

/// EQ 7-Band magnitude response in dB at `freqs`, for curve drawing. `params` overrides the
/// defaults by id; unknown ids are ignored.
pub fn eq7_response(params: &[(&str, f32)], freqs: &[f32], sr: f32) -> Vec<f32> {
    response_from(&EQ7_INFO, &|v, sr| design_eq7(v, sr).to_vec(), &[0, 22], params, freqs, sr)
}

/// EQ 1-Band magnitude response in dB at `freqs`.
pub fn eq1_response(params: &[(&str, f32)], freqs: &[f32], sr: f32) -> Vec<f32> {
    response_from(&EQ1_INFO, &|v, sr| design_eq1(v, sr).to_vec(), &[4], params, freqs, sr)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn index(id: &str) -> usize {
        EQ7_PARAMS.iter().position(|p| p.id == id).unwrap_or_else(|| panic!("no parameter {id}"))
    }

    /// Defaults with `overrides` (by id) applied.
    fn settings(overrides: &[(&str, f32)]) -> Vec<f32> {
        let mut v: Vec<f32> = EQ7_PARAMS.iter().map(|p| p.default).collect();
        for &(id, x) in overrides {
            v[index(id)] = x;
        }
        v
    }

    #[test]
    fn the_index_constants_name_their_parameters() {
        let ons = ["low_shelf_on", "low_mid_on", "mid_on", "high_mid_on", "high_shelf_on"];
        assert_eq!(EQ7_BAND_ON, ons.map(index));
        for (i, id) in [
            (EQ7_LF_TYPE, "low_shelf_type"),
            (EQ7_HF_TYPE, "high_shelf_type"),
            (EQ7_HPF_TYPE, "hpf_type"),
            (EQ7_HPF_Q, "hpf_q"),
            (EQ7_LPF_TYPE, "lpf_type"),
            (EQ7_LPF_Q, "lpf_q"),
            (EQ7_PHASE, "phase_invert"),
        ] {
            assert_eq!(i, index(id), "{id}");
        }
        // The original 23 keep their positions, which `design_eq7` reads.
        for (i, id) in
            [(1, "hpf_on"), (2, "hpf_freq"), (3, "hpf_slope"), (4, "low_shelf_freq"), (16, "high_shelf_freq"), (19, "lpf_on"), (22, "output_gain")]
        {
            assert_eq!(i, index(id), "{id}");
        }
    }

    #[test]
    fn the_added_parameters_default_to_the_old_sound() {
        let sr = 48_000.0;
        // Every band active, shelf Qs inside their old range: the sections the EQ designed before.
        let v = settings(&[
            ("hpf_on", 1.0),
            ("hpf_freq", 120.0),
            ("hpf_slope", 2.0),
            ("low_shelf_freq", 90.0),
            ("low_shelf_gain", 4.0),
            ("low_shelf_q", 1.4),
            ("low_mid_freq", 300.0),
            ("low_mid_gain", -3.0),
            ("low_mid_q", 2.5),
            ("mid_freq", 1200.0),
            ("mid_gain", 6.0),
            ("mid_q", 0.8),
            ("high_mid_freq", 5000.0),
            ("high_mid_gain", -2.0),
            ("high_mid_q", 4.0),
            ("high_shelf_freq", 9000.0),
            ("high_shelf_gain", 3.0),
            ("high_shelf_q", 0.5),
            ("lpf_on", 1.0),
            ("lpf_freq", 15_000.0),
            ("lpf_slope", 3.0),
        ]);
        let [h0, h1] = pass_filter(2, true, 120.0, sr);
        let [l0, l1] = pass_filter(3, false, 15_000.0, sr);
        let expected = [
            h0,
            h1,
            Coeffs::design(FilterType::LowShelf, 90.0, 1.4, 4.0, sr),
            Coeffs::design(FilterType::Peak, 300.0, 2.5, -3.0, sr),
            Coeffs::design(FilterType::Peak, 1200.0, 0.8, 6.0, sr),
            Coeffs::design(FilterType::Peak, 5000.0, 4.0, -2.0, sr),
            Coeffs::design(FilterType::HighShelf, 9000.0, 0.5, 3.0, sr),
            l0,
            l1,
        ];
        assert_eq!(design_eq7(&v, sr), expected);
    }

    #[test]
    fn a_shelf_keeps_its_old_q_range_and_a_peak_gets_the_wide_one() {
        let sr = 48_000.0;
        let shelf = design_eq7(&settings(&[("low_shelf_gain", 6.0), ("low_shelf_q", 8.0)]), sr);
        assert_eq!(shelf[2], Coeffs::design(FilterType::LowShelf, 100.0, SHELF_Q.1, 6.0, sr));
        let peak = design_eq7(&settings(&[("low_shelf_gain", 6.0), ("low_shelf_q", 8.0), ("low_shelf_type", 1.0)]), sr);
        assert_eq!(peak[2], Coeffs::design(FilterType::Peak, 100.0, 8.0, 6.0, sr));
    }
}
