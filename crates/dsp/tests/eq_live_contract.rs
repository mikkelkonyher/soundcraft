//! A switch, slope or filter type changed on a running, settled EQ must change the audio, not
//! only the stored value and the curve (#239).

use soundcraft_dsp::{Plugin, create, gain_to_db};
use std::f32::consts::TAU;

const SR: f32 = 48_000.0;
const BLOCK: usize = 512;

fn tone_level(p: &mut dyn Plugin, freq: f32) -> f32 {
    let mut out = Vec::with_capacity(24_000);
    for start in (0..24_000).step_by(BLOCK) {
        let n = BLOCK.min(24_000 - start);
        let mut io = vec![(start..start + n).map(|i| 0.5 * (TAU * freq * i as f32 / SR).sin()).collect::<Vec<_>>()];
        p.process(&mut io, n);
        out.extend_from_slice(&io[0]);
    }
    let steady = &out[12_000..];
    gain_to_db((steady.iter().map(|v| v * v).sum::<f32>() / steady.len() as f32).sqrt())
}

fn check_live_change(id: &str, setup: &[(&str, f32)], change: (&str, f32), tone: f32, nudge: (&str, f32)) {
    let mut p = create(id).unwrap();
    p.prepare(SR, BLOCK, 1);
    for &(key, value) in setup {
        assert!(p.set_param(key, value), "unsupported setup parameter {key}");
    }
    p.reset();
    let before = tone_level(p.as_mut(), tone);
    assert!(p.set_param(change.0, change.1));
    assert_eq!(p.param(change.0), Some(change.1));
    let live = tone_level(p.as_mut(), tone);
    let second = tone_level(p.as_mut(), tone);
    p.reset();
    let reference = tone_level(p.as_mut(), tone);

    let mut control = create(id).unwrap();
    control.prepare(SR, BLOCK, 1);
    for &(key, value) in setup {
        assert!(control.set_param(key, value));
    }
    control.reset();
    assert!(control.set_param(change.0, change.1));
    assert!(control.set_param(nudge.0, nudge.1));
    let nudged = tone_level(control.as_mut(), tone);
    assert!(control.set_param(nudge.0, setup.iter().find(|(key, _)| *key == nudge.0).unwrap().1));
    let restored = tone_level(control.as_mut(), tone);

    println!(
        "{id} {}={}: before={before:.3} live={live:.3} second={second:.3} reset={reference:.3} nudged={nudged:.3} restored={restored:.3} dBFS",
        change.0, change.1
    );
    assert!((before - reference).abs() > 3.0, "control must discriminate filter settings");
    assert!((restored - reference).abs() < 0.1, "frequency nudge and restoration must preserve chosen filter");
    assert!(
        (live - reference).abs() < 0.1 && (second - reference).abs() < 0.1,
        "live parameter change must match reset control: live={live}, second={second}, control={reference}"
    );
}

#[test]
fn eq7_hpf_enable() {
    check_live_change("eq_7band", &[("hpf_freq", 2000.0), ("hpf_slope", 3.0)], ("hpf_on", 1.0), 1000.0, ("hpf_freq", 1999.0));
}
#[test]
fn eq7_hpf_disable() {
    check_live_change("eq_7band", &[("hpf_freq", 2000.0), ("hpf_slope", 3.0), ("hpf_on", 1.0)], ("hpf_on", 0.0), 1000.0, ("hpf_freq", 1999.0));
}
#[test]
fn eq7_hpf_slope() {
    check_live_change("eq_7band", &[("hpf_freq", 2000.0), ("hpf_slope", 0.0), ("hpf_on", 1.0)], ("hpf_slope", 3.0), 1000.0, ("hpf_freq", 1999.0));
}
#[test]
fn eq7_lpf_enable() {
    check_live_change("eq_7band", &[("lpf_freq", 2000.0), ("lpf_slope", 3.0)], ("lpf_on", 1.0), 4000.0, ("lpf_freq", 1999.0));
}
#[test]
fn eq7_lpf_slope() {
    check_live_change("eq_7band", &[("lpf_freq", 2000.0), ("lpf_slope", 0.0), ("lpf_on", 1.0)], ("lpf_slope", 3.0), 4000.0, ("lpf_freq", 1999.0));
}
#[test]
fn eq1_low_pass_type() {
    check_live_change("eq_1band", &[("freq", 1000.0)], ("type", 4.0), 10_000.0, ("freq", 999.0));
}
#[test]
fn channel_strip_hpf_enable() {
    check_live_change("channel_strip", &[("hpf_freq", 2000.0)], ("hpf_on", 1.0), 1000.0, ("hpf_freq", 1999.0));
}
