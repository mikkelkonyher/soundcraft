//! EQ 7-Band's band switches, shelf/peak and notch types, and polarity switch.

use soundcraft_dsp::{Plugin, create, eq7_response, gain_to_db};
use std::f32::consts::TAU;

const SR: f32 = 48_000.0;

fn sine(freq: f32, len: usize) -> Vec<f32> {
    (0..len).map(|i| 0.25 * (TAU * freq * i as f32 / SR).sin()).collect()
}

fn run(p: &mut dyn Plugin, x: &[f32]) -> Vec<f32> {
    let mut out = Vec::with_capacity(x.len());
    for chunk in x.chunks(512) {
        let mut io = vec![chunk.to_vec()];
        p.process(&mut io, chunk.len());
        out.extend_from_slice(&io[0]);
    }
    out
}

fn eq(params: &[(&str, f32)]) -> Box<dyn Plugin> {
    let mut p = create("eq_7band").unwrap();
    p.prepare(SR, 512, 1);
    for (k, v) in params {
        assert!(p.set_param(k, *v), "{k}");
    }
    p.reset();
    p
}

#[test]
fn a_band_switched_out_is_flat() {
    let on = eq7_response(&[("low_mid_freq", 300.0), ("low_mid_gain", 12.0)], &[300.0], SR);
    let off = eq7_response(&[("low_mid_freq", 300.0), ("low_mid_gain", 12.0), ("low_mid_on", 0.0)], &[300.0], SR);
    assert!((on[0] - 12.0).abs() < 0.05, "{on:?}");
    assert!(off[0].abs() < 1e-3, "{off:?}");
    for id in ["low_shelf_on", "mid_on", "high_mid_on", "high_shelf_on"] {
        let band = id.trim_end_matches("_on");
        let gain = format!("{band}_gain");
        let r = eq7_response(&[(gain.as_str(), 9.0), (id, 0.0)], &[30.0, 1000.0, 15_000.0], SR);
        assert!(r.iter().all(|db| db.abs() < 1e-3), "{id}: {r:?}");
    }
}

#[test]
fn lf_and_hf_can_be_peaks_instead_of_shelves() {
    let f = [20.0, 100.0];
    let shelf = eq7_response(&[("low_shelf_freq", 100.0), ("low_shelf_gain", 9.0)], &f, SR);
    let peak = eq7_response(&[("low_shelf_freq", 100.0), ("low_shelf_gain", 9.0), ("low_shelf_q", 4.0), ("low_shelf_type", 1.0)], &f, SR);
    assert!(shelf[0] > 8.0, "a shelf lifts everything below: {shelf:?}");
    assert!((peak[1] - 9.0).abs() < 0.05 && peak[0] < 1.0, "a peak lifts only around its frequency: {peak:?}");
    let f = [5000.0, 19_000.0];
    let hpeak = eq7_response(&[("high_shelf_freq", 5000.0), ("high_shelf_gain", -6.0), ("high_shelf_q", 4.0), ("high_shelf_type", 1.0)], &f, SR);
    assert!((hpeak[0] + 6.0).abs() < 0.05 && hpeak[1].abs() < 1.0, "{hpeak:?}");
}

#[test]
fn the_filters_can_be_notches_whose_width_follows_q() {
    let f = [100.0, 800.0, 1000.0, 1250.0, 10_000.0];
    let notch = |q: f32| eq7_response(&[("hpf_on", 1.0), ("hpf_type", 1.0), ("hpf_freq", 1000.0), ("hpf_q", q), ("hpf_slope", 3.0)], &f, SR);
    let wide = notch(1.0);
    let narrow = notch(8.0);
    assert!(wide[2] < -40.0, "deep at its frequency: {wide:?}");
    assert!(wide[0].abs() < 0.5 && wide[4].abs() < 0.5, "a notch, not a high-pass: {wide:?}");
    assert!(narrow[1] > wide[1] + 3.0 && narrow[3] > wide[3] + 3.0, "higher Q is narrower: {narrow:?} vs {wide:?}");
    let lpf = eq7_response(&[("lpf_on", 1.0), ("lpf_type", 1.0), ("lpf_freq", 3000.0)], &[3000.0, 15_000.0], SR);
    assert!(lpf[0] < -40.0 && lpf[1].abs() < 0.5, "{lpf:?}");
    // Switched out, the notch is gone too.
    let off = eq7_response(&[("hpf_on", 0.0), ("hpf_type", 1.0), ("hpf_freq", 1000.0)], &[1000.0], SR);
    assert!(off[0].abs() < 1e-3, "{off:?}");
}

#[test]
fn the_processed_audio_follows_the_new_types() {
    // The live EQ agrees with the curve for a notch and a low peak.
    for (params, freq) in [
        (vec![("hpf_on", 1.0), ("hpf_type", 1.0), ("hpf_freq", 1000.0), ("hpf_q", 2.0)], 1000.0),
        (vec![("low_shelf_type", 1.0), ("low_shelf_freq", 200.0), ("low_shelf_gain", 6.0), ("low_shelf_q", 3.0)], 200.0),
    ] {
        let mut p = eq(&params);
        let x = sine(freq, 48_000);
        let y = run(p.as_mut(), &x);
        let rms = |s: &[f32]| (s.iter().map(|v| v * v).sum::<f32>() / s.len() as f32).sqrt();
        let measured = gain_to_db(rms(&y[24_000..]) / rms(&x[24_000..]));
        let curve = eq7_response(&params, &[freq], SR)[0];
        assert!((measured - curve).abs() < 0.5 || (curve < -40.0 && measured < -30.0), "{params:?}: measured {measured}, curve {curve}");
    }
}

#[test]
fn phase_invert_flips_the_output_without_a_click() {
    let x = vec![0.5f32; 9600];
    let mut plain = eq(&[]);
    let mut flipped = eq(&[("phase_invert", 1.0)]);
    let a = run(plain.as_mut(), &x);
    let b = run(flipped.as_mut(), &x);
    assert!((a[9000] - 0.5).abs() < 1e-4 && (b[9000] + 0.5).abs() < 1e-4, "{} {}", a[9000], b[9000]);
    // Flip a running EQ: the output ramps through zero instead of jumping from +0.5 to -0.5.
    assert!(plain.set_param("phase_invert", 1.0));
    let c = run(plain.as_mut(), &x);
    let max_step =
        std::iter::once(a[a.len() - 1]).chain(c.iter().copied()).collect::<Vec<_>>().windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0f32, f32::max);
    assert!(max_step < 0.01, "step {max_step}");
    assert!((c[9000] + 0.5).abs() < 1e-4, "{}", c[9000]);
    // Polarity does not change the magnitude curve.
    assert_eq!(eq7_response(&[("phase_invert", 1.0)], &[1000.0], SR), eq7_response(&[], &[1000.0], SR));
}

#[test]
fn hostile_values_are_clamped() {
    let mut p = eq(&[]);
    for id in ["hpf_q", "lpf_q", "low_shelf_q", "high_shelf_q", "hpf_freq", "lpf_freq"] {
        for v in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -1e9, 1e9] {
            p.set_param(id, v);
            let got = p.param(id).unwrap();
            assert!(got.is_finite(), "{id}={v} -> {got}");
        }
    }
    for id in ["low_shelf_type", "hpf_type", "lpf_type", "mid_on", "phase_invert"] {
        p.set_param(id, 7.5);
        assert!(p.param(id).unwrap() <= 1.0, "{id}");
    }
    let y = run(p.as_mut(), &sine(1000.0, 4800));
    assert!(y.iter().all(|v| v.is_finite()));
}
