use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use soundcraft_ui_egui::{Services, SoundApp};

#[test]
fn a_long_plugin_window_stays_on_a_short_screen() {
    let mut app = SoundApp::new(soundcraft_engine::demo::demo_engine(), None, Services::default());
    let kick = app.engine.session().track_by_name("Kick").unwrap().id;
    app.ui.plugin_windows = vec![(kick, 0)];
    let mut h = Harness::builder().with_size(egui::vec2(1200.0, 500.0)).build_ui_state(
        |ui, app: &mut SoundApp| {
            app.logic(&ui.ctx().clone());
            app.ui(ui);
        },
        app,
    );
    h.run_steps(4);
    let window = h.ctx.memory(|m| m.area_rect(egui::Id::new(("plugin", kick.0, 0usize)))).unwrap();
    assert!(window.bottom() <= 500.0, "{window:?}");
}

#[test]
fn a_preset_can_be_named_before_it_is_saved() {
    let dir = std::env::temp_dir().join(format!("soundcraft-preset-name-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut app = SoundApp::new(soundcraft_engine::demo::demo_engine(), None, Services::default());
    app.preset_dir = Some(dir.clone());
    let kick = app.engine.session().track_by_name("Kick").unwrap().id;
    app.ui.plugin_windows = vec![(kick, 0)];
    let mut h = Harness::builder().with_size(egui::vec2(1500.0, 1000.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut SoundApp| {
            app.logic(&ui.ctx().clone());
            app.ui(ui);
        },
        app,
    );
    h.run_steps(4);
    h.get_by_label("Presets ⏷").click();
    h.run_steps(3);
    // Clicking into the name field keeps the menu open and focuses the field.
    let field = |h: &Harness<'_, SoundApp>| {
        h.query_all_by_role(egui::accesskit::Role::TextInput).find(|f| f.value().is_some_and(|v| v.contains("preset"))).map(|f| f.rect().center())
    };
    let at = field(&h).expect("the preset name field");
    h.event(egui::Event::PointerMoved(at));
    h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() });
    h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() });
    h.run_steps(3);
    assert!(field(&h).is_some(), "the menu stays open");
    h.event(egui::Event::Key { key: egui::Key::A, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::COMMAND });
    h.event(egui::Event::Text("Kick fat".into()));
    h.run_steps(2);
    h.get_by_label("Save").click();
    h.run_steps(3);
    let saved: Vec<String> = walk(&dir);
    let _ = std::fs::remove_dir_all(&dir);
    assert!(saved.iter().any(|f| f.ends_with("Kick fat.json")), "saved under its own name: {saved:?}");
    let preset = h.state().engine.session().track_by_name("Kick").unwrap().mixer.inserts[0].as_ref().unwrap().preset.clone();
    assert_eq!(preset, "Kick fat");
}

fn walk(dir: &std::path::Path) -> Vec<String> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = e.path();
        if p.is_dir() {
            out.extend(walk(&p));
        } else {
            out.push(p.to_string_lossy().into_owned());
        }
    }
    out
}
