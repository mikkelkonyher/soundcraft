use egui::{Event, PointerButton, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;
use soundcraft_ui_egui::{Services, SoundApp, theme::ThemeMode};
use std::time::Duration;

/// The demo session with the Kick's EQ 7-Band (insert a) open.
fn harness() -> Harness<'static, SoundApp> {
    let mut app = SoundApp::new(soundcraft_engine::demo::demo_engine(), None, Services::default());
    let kick = app.engine.session().track_by_name("Kick").unwrap().id;
    app.ui.plugin_windows = vec![(kick, 0)];
    // Real frame pacing, so two clicks in successive frames are a double-click.
    let mut h = Harness::builder().with_size(vec2(1500.0, 1000.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut SoundApp| {
            app.logic(&ui.ctx().clone());
            app.ui(ui);
        },
        app,
    );
    h.run_steps(4);
    h
}

fn param(h: &Harness<'_, SoundApp>, id: &str) -> f32 {
    let s = h.state().engine.session();
    let kick = s.track_by_name("Kick").unwrap();
    let ins = kick.mixer.inserts[0].as_ref().unwrap();
    let p = soundcraft_dsp::plugins::eq::EQ7_INFO.params.iter().find(|p| p.id == id).unwrap();
    ins.params.get(id).copied().unwrap_or(p.default)
}

/// Press at `from`, move in steps to `to`, release: a real multi-frame drag.
fn drag(h: &mut Harness<'_, SoundApp>, from: egui::Pos2, to: egui::Pos2) {
    h.event(Event::PointerMoved(from));
    h.step();
    h.event(Event::PointerButton { pos: from, button: PointerButton::Primary, pressed: true, modifiers: Default::default() });
    h.step();
    for i in 1..=8 {
        let p = from + (to - from) * (i as f32 / 8.0);
        h.event(Event::PointerMoved(p));
        h.step();
    }
    h.event(Event::PointerButton { pos: to, button: PointerButton::Primary, pressed: false, modifiers: Default::default() });
    h.run_steps(2);
}

#[test]
fn dragging_a_node_sets_frequency_and_gain_as_one_undo_step() {
    let mut h = harness();
    let (f0, g0) = (param(&h, "mid_freq"), param(&h, "mid_gain"));
    let undo0 = h.state().engine.undo_history().len();
    let node = h.get_by_label("MF node").rect().center();
    drag(&mut h, node, node + vec2(80.0, -60.0));
    let (f1, g1) = (param(&h, "mid_freq"), param(&h, "mid_gain"));
    assert!(f1 > f0 * 1.2, "frequency rose: {f0} -> {f1}");
    assert!(g1 > g0 + 3.0, "gain rose: {g0} -> {g1}");
    assert_eq!(h.state().engine.undo_history().len(), undo0 + 1, "one drag, one undo step");
    assert!(h.state_mut().engine.undo());
    assert_eq!((param(&h, "mid_freq"), param(&h, "mid_gain")), (f0, g0));

    // A second drag of the same node after release is its own step.
    h.run_steps(2);
    let node = h.get_by_label("MF node").rect().center();
    drag(&mut h, node, node + vec2(0.0, 40.0));
    let node = h.get_by_label("MF node").rect().center();
    drag(&mut h, node, node + vec2(0.0, 20.0));
    assert_eq!(h.state().engine.undo_history().len(), undo0 + 2);
}

/// Scroll over `pos` for a few frames, as a wheel or trackpad would, then let egui's smoothing settle.
fn scroll(h: &mut Harness<'_, SoundApp>, pos: egui::Pos2, dy: f32) {
    h.event(Event::PointerMoved(pos));
    h.step();
    for _ in 0..4 {
        h.event(Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: vec2(0.0, dy),
            modifiers: Default::default(),
            phase: egui::TouchPhase::Move,
        });
        h.step();
    }
    h.run_steps(60);
}

#[test]
fn separate_scrolls_over_a_node_are_separate_undo_steps() {
    let mut h = harness();
    let undo0 = h.state().engine.undo_history().len();
    let q0 = param(&h, "mid_q");
    let node = h.get_by_label("MF node").rect().center();
    scroll(&mut h, node, 40.0);
    let q1 = param(&h, "mid_q");
    assert!(q1 > q0, "scrolling up raises Q: {q0} -> {q1}");
    assert_eq!(h.state().engine.undo_history().len(), undo0 + 1, "one scroll, one step");
    // Another scroll on the same node: its own step.
    scroll(&mut h, node, 40.0);
    assert_eq!(h.state().engine.undo_history().len(), undo0 + 2);
    assert!(h.state_mut().engine.undo());
    assert_eq!(param(&h, "mid_q"), q1, "undo takes back only the second scroll");
}

#[test]
fn dragging_far_outside_the_graph_clamps_to_the_parameter_range() {
    let mut h = harness();
    let node = h.get_by_label("HMF node").rect().center();
    drag(&mut h, node, node + vec2(5000.0, -5000.0));
    let (f, g) = (param(&h, "high_mid_freq"), param(&h, "high_mid_gain"));
    assert!(f.is_finite() && (500.0..=20_000.0).contains(&f), "{f}");
    assert!(g.is_finite() && (-24.0..=24.0).contains(&g), "{g}");
}

#[test]
fn double_click_resets_a_band() {
    let mut h = harness();
    let kick = h.state().engine.session().track_by_name("Kick").unwrap().id.0;
    h.state_mut()
        .run("mix.insert_params", json!({"track": kick, "slot": 0, "params": {"low_mid_gain": 9.0, "low_mid_freq": 600.0, "low_mid_q": 4.0}}))
        .unwrap();
    h.run_steps(2);
    h.get_by_label("LMF node").click();
    h.step();
    h.get_by_label("LMF node").click();
    h.run_steps(2);
    assert_eq!((param(&h, "low_mid_gain"), param(&h, "low_mid_freq"), param(&h, "low_mid_q")), (0.0, 250.0, 1.0));
}

#[test]
fn the_in_switch_turns_the_high_pass_filter_on_and_off() {
    let mut h = harness();
    let start = param(&h, "hpf_on");
    h.get_by_label("HPF in").click();
    h.run_steps(2);
    assert_ne!(param(&h, "hpf_on"), start);
    h.get_by_label("HPF in").click();
    h.run_steps(2);
    assert_eq!(param(&h, "hpf_on"), start);
}

#[test]
fn slope_menus_stay_inside_their_sections_in_a_narrow_window() {
    let mut app = SoundApp::new(soundcraft_engine::demo::demo_engine(), None, Services::default());
    let kick = app.engine.session().track_by_name("Kick").unwrap().id;
    app.ui.plugin_windows = vec![(kick, 0)];
    // A screen narrower than the window's default width squeezes it toward its minimum.
    let mut h = Harness::builder().with_size(vec2(640.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut SoundApp| {
            app.logic(&ui.ctx().clone());
            app.ui(ui);
        },
        app,
    );
    h.run_steps(6);
    let window = h.ctx.memory(|m| m.area_rect(egui::Id::new(("plugin", kick.0, 0usize)))).unwrap();
    assert!(window.width() < 720.0, "the window is narrower than its default: {window:?}");
    let combos: Vec<egui::Rect> = h.get_all_by_role(egui::accesskit::Role::ComboBox).map(|n| n.rect()).collect();
    for band in ["HPF", "LPF"] {
        // The IN switch sits 8 px inside the section's right edge.
        let section_right = h.get_by_label(&format!("{band} in")).rect().right() + 8.0;
        let combo = combos
            .iter()
            .filter(|c| c.right() <= section_right + 40.0 && c.left() >= section_right - 200.0)
            .min_by(|a, b| (section_right - a.right()).abs().total_cmp(&(section_right - b.right()).abs()))
            .unwrap_or_else(|| panic!("{band} slope menu"));
        assert!(combo.right() <= section_right + 0.5, "{band} slope menu {combo:?} overflows its section (right edge {section_right})");
        assert!(combo.right() <= window.right(), "{band} slope menu {combo:?} leaves the window {window:?}");
    }
}

#[test]
fn a_10_hz_high_pass_node_sits_on_the_flat_line_inside_the_graph() {
    let mut h = harness();
    let kick = h.state().engine.session().track_by_name("Kick").unwrap().id.0;
    h.state_mut()
        .run(
            "mix.insert_params",
            json!({"track": kick, "slot": 0, "params": {"hpf_on": 1.0, "hpf_freq": 10.0, "hpf_slope": 3.0, "low_shelf_gain": 0.0}}),
        )
        .unwrap();
    h.run_steps(3);
    let window = h.ctx.memory(|m| m.area_rect(egui::Id::new(("plugin", kick, 0usize)))).unwrap();
    let hpf = h.get_by_label("HPF node").rect();
    // LF at 0 dB marks the flat line.
    let flat = h.get_by_label("LF node").rect().center().y;
    assert!((hpf.center().y - flat).abs() <= 1.0, "HPF node {hpf:?} is on the 0 dB line ({flat})");
    assert!(hpf.center().x < h.get_by_label("LF node").rect().center().x);
    // The whole node (radius 7) is inside the graph, right of the dB labels.
    assert!(hpf.center().x - 7.0 > window.left() + 34.0, "{hpf:?} in {window:?}");
}

#[test]
fn controls_sit_where_pro_tools_users_expect_them() {
    let h = harness();
    let rect = |label: &str| h.get_by_label(label).rect();
    let nodes: Vec<egui::Rect> = ["HPF", "LF", "LMF", "MF", "HMF", "HF", "LPF"].iter().map(|b| rect(&format!("{b} node"))).collect();
    let graph_left = nodes.iter().map(|r| r.left()).fold(f32::INFINITY, f32::min);
    let graph_bottom = nodes.iter().map(|r| r.bottom()).fold(f32::NEG_INFINITY, f32::max);
    // Input/output and both filters in a column left of the graph.
    for label in ["Input", "Output", "HPF Freq", "LPF Freq"] {
        assert!(rect(label).right() < graph_left, "{label} {:?} is left of the graph ({graph_left})", rect(label));
    }
    assert!(rect("Input").bottom() <= rect("HPF Freq").top(), "I/O above the filters");
    assert!(rect("HPF Freq").right() < rect("LPF Freq").left(), "HPF beside LPF");
    // The five bands in a row under the graph, LF to HF, with Q above frequency above a larger gain knob.
    let gains: Vec<egui::Rect> = ["LF", "LMF", "MF", "HMF", "HF"].iter().map(|b| rect(&format!("{b} Gain"))).collect();
    for pair in gains.windows(2) {
        assert!(pair[0].right() < pair[1].left(), "{pair:?}");
        assert!((pair[0].center().y - pair[1].center().y).abs() < 1.0, "one row: {pair:?}");
    }
    assert!(gains[0].top() > graph_bottom, "the bands are under the graph");
    assert!(rect("MF Q").bottom() <= rect("MF Freq").top() && rect("MF Freq").bottom() <= rect("MF Gain").top());
    assert!(rect("MF Gain").width() > rect("MF Q").width() * 1.4, "the gain knob is the large one");
}

#[test]
fn every_band_has_an_in_switch_that_takes_it_out_of_the_curve() {
    let mut h = harness();
    let kick = h.state().engine.session().track_by_name("Kick").unwrap().id.0;
    h.state_mut().run("mix.insert_params", json!({"track": kick, "slot": 0, "params": {"mid_gain": 9.0}})).unwrap();
    h.run_steps(2);
    for (band, id) in [("LF", "low_shelf_on"), ("LMF", "low_mid_on"), ("MF", "mid_on"), ("HMF", "high_mid_on"), ("HF", "high_shelf_on")] {
        assert_eq!(param(&h, id), 1.0, "{band} starts in");
        h.get_by_label(&format!("{band} in")).click();
        h.run_steps(2);
        assert_eq!(param(&h, id), 0.0, "{band} switched out");
    }
}

#[test]
fn lf_and_hf_switch_between_shelf_and_peak_and_the_filters_between_filter_and_notch() {
    let mut h = harness();
    for (button, id, value) in [
        ("LF Peak", "low_shelf_type", 1.0),
        ("LF Shelf", "low_shelf_type", 0.0),
        ("HF Peak", "high_shelf_type", 1.0),
        ("HPF Notch", "hpf_type", 1.0),
        ("LPF Notch", "lpf_type", 1.0),
        ("LPF Low Pass", "lpf_type", 0.0),
    ] {
        h.get_by_label(button).click();
        h.run_steps(2);
        assert_eq!(param(&h, id), value, "{button}");
    }
    // As a notch, the HPF shows its Q instead of the slope menu.
    assert!(h.query_by_label("HPF Notch Q").is_some(), "the notch's Q knob");
    let combos = h.get_all_by_role(egui::accesskit::Role::ComboBox).count();
    assert_eq!(combos, 1, "only the LPF slope menu is left");
}

#[test]
fn the_phase_switch_flips_polarity() {
    let mut h = harness();
    assert_eq!(param(&h, "phase_invert"), 0.0);
    h.get_by_label("Phase invert").click();
    h.run_steps(2);
    assert_eq!(param(&h, "phase_invert"), 1.0);
}

#[test]
fn bypass_keeps_its_label_and_toggles_the_insert() {
    let mut h = harness();
    let bypassed = |h: &Harness<'_, SoundApp>| h.state().engine.session().track_by_name("Kick").unwrap().mixer.inserts[0].as_ref().unwrap().bypass;
    assert!(!bypassed(&h));
    h.get_by_label("Bypass").click();
    h.run_steps(2);
    assert!(bypassed(&h));
    assert!(h.query_by_label("BYPASSED").is_none(), "the button keeps one label");
    h.get_by_label("Bypass").click();
    h.run_steps(2);
    assert!(!bypassed(&h));
}

#[test]
fn the_filters_line_up_with_the_bands_below_them() {
    let h = harness();
    let rect = |label: &str| h.get_by_label(label).rect();
    // Each section's first knob sits at its left padding and its IN switch at its right one, so
    // equal edges mean equal sections in the same column.
    for (filter, band) in [("HPF", "LF"), ("LPF", "LMF")] {
        let f_left = rect(&format!("{filter} Freq")).left();
        let b_left = rect(&format!("{band} Gain")).left();
        assert!((f_left - b_left).abs() < 0.5, "{filter} starts where {band} does: {f_left} vs {b_left}");
        let f_right = rect(&format!("{filter} in")).right();
        let b_right = rect(&format!("{band} in")).right();
        assert!((f_right - b_right).abs() < 0.5, "{filter} ends where {band} does: {f_right} vs {b_right}");
    }
    // The graph starts over MF: its leftmost node lies right of LMF's section.
    assert!(rect("HPF node").left() > rect("LMF in").right(), "the graph spans MF to HF");
}

#[test]
fn clicking_a_clip_lamp_clears_it() {
    let mut h = harness();
    let kick = h.state().engine.session().track_by_name("Kick").unwrap().id;
    let hot = soundcraft_ui_egui::MeterDisplay { clip: true, ..Default::default() };
    h.state_mut().plugin_meters.insert((kick, 0), [hot, hot]);
    h.run_steps(1);
    h.get_by_label("Clear output clip").click();
    h.run_steps(1);
    let m = h.state().plugin_meters[&(kick, 0)];
    assert!(m[0].clip && !m[1].clip, "only the output lamp cleared");
}

#[test]
fn input_and_output_gain_do_not_move_the_graph() {
    let mut h = harness();
    let kick = h.state().engine.session().track_by_name("Kick").unwrap().id.0;
    let nodes = |h: &Harness<'_, SoundApp>| ["HPF", "LF", "MF", "HF"].map(|b| h.get_by_label(&format!("{b} node")).rect().center());
    let before = nodes(&h);
    h.state_mut().run("mix.insert_params", json!({"track": kick, "slot": 0, "params": {"input_gain": 12.0, "output_gain": -6.0}})).unwrap();
    h.run_steps(2);
    assert_eq!(nodes(&h), before, "the curve shows the EQ's shape, not its level");
}

/// Idle repaint delay after the window has settled (any floating window takes a few frames to size).
fn settled_delay(open_eq: bool) -> Duration {
    let ctx = egui::Context::default();
    let mut app = SoundApp::new(soundcraft_engine::demo::demo_engine(), None, Services::default());
    if open_eq {
        let kick = app.engine.session().track_by_name("Kick").unwrap().id;
        app.ui.plugin_windows = vec![(kick, 0)];
    }
    let mut delay = Duration::ZERO;
    for _ in 0..12 {
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            app.logic(ui.ctx());
            app.ui(ui);
        });
        out.textures_delta.clear();
        delay = out.viewport_output[&egui::ViewportId::ROOT].repaint_delay;
    }
    delay
}

#[test]
fn an_open_eq_window_adds_no_repaints_while_idle() {
    let without = settled_delay(false);
    let with = settled_delay(true);
    assert!(without >= Duration::from_millis(100), "baseline idles: {without:?}");
    assert!(with >= Duration::from_millis(100), "the EQ window must not animate while idle: {with:?}");
}

/// Renders the window for review: `EQ_EDITOR_SHOTS=dir cargo test -p soundcraft-ui-egui --test eq_editor`.
#[test]
fn eq_editor_visuals() {
    let Some(dir) = std::env::var_os("EQ_EDITOR_SHOTS").map(std::path::PathBuf::from) else { return };
    std::fs::create_dir_all(&dir).unwrap();
    for scale in [1.0, 2.0] {
        for (name, mode) in [("dark", ThemeMode::Dark), ("light", ThemeMode::Light)] {
            let mut app = SoundApp::new(soundcraft_engine::demo::demo_engine(), None, Services::default());
            let kick = app.engine.session().track_by_name("Kick").unwrap().id;
            app.run(
                "mix.insert_params",
                json!({"track": kick.0, "slot": 0, "params": {
                    "hpf_on": 1.0, "hpf_freq": 40.0, "low_shelf_gain": 3.5, "low_mid_freq": 250.0, "low_mid_gain": -2.0,
                    "low_mid_q": 1.2, "mid_gain": 1.5, "high_mid_freq": 3500.0, "high_mid_gain": -1.5, "high_shelf_gain": 2.0
                }}),
            )
            .unwrap();
            app.ui.plugin_windows = vec![(kick, 0)];
            app.ui.theme = mode;
            let mut h = Harness::builder().with_size(vec2(1500.0, 1000.0)).with_pixels_per_point(scale).wgpu().build_ui_state(
                |ui, app: &mut SoundApp| {
                    app.logic(&ui.ctx().clone());
                    app.ui(ui);
                },
                app,
            );
            h.input_mut().max_texture_side = Some(8192);
            h.run_steps(6);
            // Meters mid-song: a healthy input, an output into the red with its clip lamp lit.
            let level =
                |l: f32, hold: f32, clip: bool| soundcraft_ui_egui::MeterDisplay { level: [l; 2], hold: [hold; 2], clip, ..Default::default() };
            h.state_mut().plugin_meters.insert((kick, 0), [level(0.35, 0.5, false), level(0.8, 0.95, true)]);
            let node = h.get_by_label("MF node").rect().center();
            h.event(Event::PointerMoved(node));
            h.run_steps(2);
            h.render().unwrap().save(dir.join(format!("eq-{name}-{scale}x.png"))).unwrap();
            h.set_size(vec2(640.0, 1000.0));
            h.run_steps(6);
            h.render().unwrap().save(dir.join(format!("eq-{name}-narrow-{scale}x.png"))).unwrap();
            for (shot, params) in [
                ("q10", json!({"mid_freq": 665.9, "mid_gain": 24.0, "mid_q": 10.0})),
                ("hpf10", json!({"hpf_on": 1.0, "hpf_freq": 10.0, "hpf_slope": 3.0})),
                ("hpf-steep", json!({"hpf_on": 1.0, "hpf_freq": 200.0, "hpf_slope": 3.0})),
                ("lpf-steep", json!({"hpf_on": 0.0, "lpf_on": 1.0, "lpf_freq": 2000.0, "lpf_slope": 3.0})),
                (
                    "types",
                    json!({"lpf_on": 0.0, "hpf_on": 1.0, "hpf_type": 1.0, "hpf_freq": 60.0, "hpf_q": 4.0, "low_shelf_type": 1.0, "low_shelf_q": 3.0, "mid_on": 0.0, "phase_invert": 1.0}),
                ),
                ("bypassed", json!({})),
            ] {
                h.state_mut().run("mix.insert_params", json!({"track": kick.0, "slot": 0, "params": params})).unwrap();
                if shot == "bypassed" {
                    h.state_mut().run("mix.insert_bypass", json!({"track": kick.0, "slot": 0})).unwrap();
                }
                h.set_size(vec2(1500.0, 1000.0));
                h.run_steps(4);
                h.render().unwrap().save(dir.join(format!("eq-{name}-{shot}-{scale}x.png"))).unwrap();
            }
        }
    }
}
