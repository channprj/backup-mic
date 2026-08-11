use std::{fs, os::unix::fs::PermissionsExt, process::Command};

use serde_json::Value;
use tempfile::tempdir;

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

#[test]
fn package_cleanup_compares_only_canonical_physical_paths() {
    let package_script = include_str!("../../scripts/package-local.sh");

    assert!(package_script.contains("PROJECT_ROOT=\"$(cd \"$(dirname \"$0\")/..\" && pwd -P)\""));
    assert!(package_script.contains("$(cd \"$BUNDLE_ROOT\" && pwd -P)"));
}

#[test]
fn deletion_fixture_refuses_connected_dji_before_creating_an_image() {
    let fixture = tempdir().unwrap();
    let bin = fixture.path().join("bin");
    fs::create_dir(&bin).unwrap();
    let mount = bin.join("mount");
    fs::write(
        &mount,
        "#!/bin/bash\nprintf '/dev/disk9 on /Volumes/DJI-MIC-1 (msdos, local)\\n'\n",
    )
    .unwrap();
    let hdiutil = bin.join("hdiutil");
    fs::write(
        &hdiutil,
        "#!/bin/bash\nprintf '%s\\n' \"$*\" >> \"$BACKUP_MIC_TEST_HDIUTIL_LOG\"\nexit 97\n",
    )
    .unwrap();
    for executable in [&mount, &hdiutil] {
        let mut permissions = fs::metadata(executable).unwrap().permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(executable, permissions).unwrap();
    }
    let log = fixture.path().join("hdiutil.log");
    let system_path = std::env::var("PATH").unwrap();
    let output = Command::new("/bin/bash")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../scripts/accept-deletion-fixture.sh"
        ))
        .env("PATH", format!("{}:{system_path}", bin.display()))
        .env("TMPDIR", fixture.path())
        .env("BACKUP_MIC_TEST_HDIUTIL_LOG", &log)
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        "Disconnect DJI recorder volumes before running the deletion fixture.\n"
    );
    assert!(
        !log.exists(),
        "hdiutil must not run when a DJI volume is mounted"
    );
}
