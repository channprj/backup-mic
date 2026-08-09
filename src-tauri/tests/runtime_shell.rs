use serde_json::Value;

#[test]
fn config_declares_one_hidden_standard_settings_singleton() {
    let config: Value = serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
    let windows = config["app"]["windows"].as_array().unwrap();
    assert_eq!(windows.len(), 2);
    let settings = windows
        .iter()
        .find(|window| window["label"] == "settings")
        .unwrap();
    assert_eq!(settings["url"], "index.html?window=settings");
    assert_eq!(settings["visible"], false);
    assert_eq!(settings["decorations"], true);
    assert_eq!(settings["alwaysOnTop"], false);
    assert_eq!(settings["width"], 540);
    assert_eq!(settings["height"], 620);

    let capabilities: Value =
        serde_json::from_str(include_str!("../capabilities/main.json")).unwrap();
    assert_eq!(
        capabilities["windows"],
        serde_json::json!(["main", "settings"])
    );
}

#[test]
fn only_the_popover_hides_on_focus_loss() {
    let source = include_str!("../src/window.rs");
    assert!(source.contains("window.label() == \"main\""));
    assert!(source.contains("WindowEvent::Focused(false)"));
}

#[test]
fn settings_window_owns_a_bounded_vertical_scroll_area() {
    let stylesheet = include_str!("../../src/index.css");
    assert!(
        stylesheet.contains(".settings-shell {\n  width: 100%;\n  height: 100%;\n  min-height: 0;")
    );
    assert!(stylesheet.contains("  overflow-y: auto;\n  overscroll-behavior-y: contain;"));
}
