use serde_json::Value;

#[test]
fn product_identity_is_backup_mic_everywhere_public() {
    let package: Value = serde_json::from_str(include_str!("../../package.json")).unwrap();
    let cargo = include_str!("../Cargo.toml");
    let config: Value = serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
    let main = include_str!("../src/main.rs");
    let library = include_str!("../src/lib.rs");
    let orchestrator = include_str!("../src/orchestrator.rs");
    let disk_monitor = include_str!("../src/platform/macos/mod.rs");
    let package_script = include_str!("../../scripts/package-local.sh");

    assert_eq!(backup_mic_lib::PRODUCT_NAME, "Backup Mic");
    assert_eq!(package["name"], "backup-mic");
    assert!(cargo.contains("name = \"backup-mic\""));
    assert!(cargo.contains("name = \"backup_mic_lib\""));
    assert_eq!(config["productName"], "Backup Mic");
    assert_eq!(config["identifier"], "com.channprj.BackupMic");
    assert!(main.contains("backup_mic_lib::run()"));
    assert!(library.contains("documents.join(PRODUCT_NAME)"));
    assert!(
        library.find("prepare_app_data(&app_data").unwrap() < library.find("Ledger::open").unwrap()
    );
    assert!(orchestrator.contains("backup-mic-device-events"));
    assert!(disk_monitor.contains("backup-mic-disk-arbitration"));
    assert!(package_script.contains("macos/Backup Mic.app"));
    assert!(package_script.contains("Contents/MacOS/backup-mic"));
    assert!(package_script.contains("com.channprj.BackupMic"));
}

#[test]
fn legacy_identity_is_confined_to_migration_and_exact_installer_transition() {
    let migration = include_str!("../src/legacy_app_data.rs");
    let installer = include_str!("../../scripts/install-local.sh");

    assert!(migration.contains("com.channprj.DJIMicBackup"));
    assert!(installer.contains("DJI Mic Backup.app"));
    assert!(installer.contains("com.channprj.DJIMicBackup"));
}

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
