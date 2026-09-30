use std::{fs, os::unix::fs::PermissionsExt, path::Path, process::Command};

fn executable(path: &Path, source: &str) {
    fs::write(path, source).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
}

#[test]
fn existing_deletion_fixture_mount_is_never_detached() {
    let fixture = tempfile::tempdir().unwrap();
    let bin = fixture.path().join("bin");
    fs::create_dir(&bin).unwrap();
    let calls = fixture.path().join("calls");
    executable(
        &bin.join("mount"),
        "#!/bin/sh\nprintf '%s\\n' '/dev/existing on /Volumes/DJI-DELTEST (msdos, local)'\n",
    );
    executable(
        &bin.join("hdiutil"),
        "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"$BACKUP_MIC_FIXTURE_COMMAND_LOG\"\nexit 1\n",
    );
    let mut paths = vec![bin];
    paths.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap()));

    let output = Command::new("/bin/bash")
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("../scripts/accept-deletion-fixture.sh"))
        .env("PATH", std::env::join_paths(paths).unwrap())
        .env("TMPDIR", fixture.path())
        .env("BACKUP_MIC_FIXTURE_COMMAND_LOG", &calls)
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(
        !calls.exists(),
        "the script touched a disk image despite a pre-existing mount: {}",
        fs::read_to_string(&calls).unwrap_or_default()
    );
}

#[test]
fn fixture_identity_ignores_entity_order_and_rejects_foreign_devices() {
    let fixture = tempfile::tempdir().unwrap();
    let plist = fixture.path().join("attach.plist");
    let device = "<dict><key>dev-entry</key><string>/dev/disk999</string></dict>";
    let partition = "<dict><key>dev-entry</key><string>/dev/disk999s1</string><key>mount-point</key><string>/Volumes/FIXTURE</string></dict>";
    for (entities, accepted) in [
        (format!("{device}{partition}"), true),
        (format!("{partition}{device}"), true),
        (
            format!("{device}{partition}").replace("disk999s1", "disk998s1"),
            false,
        ),
        (device.to_owned(), true),
        (partition.to_owned(), false),
        (
            format!("{device}{partition}{device}").replacen("disk999", "disk998", 1),
            false,
        ),
    ] {
        fs::write(&plist, format!("<?xml version=\"1.0\"?><plist version=\"1.0\"><dict><key>system-entities</key><array>{entities}</array></dict></plist>")).unwrap();
        let output = Command::new("python3")
            .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("../scripts/fixture-device.py"))
            .arg(&plist)
            .output()
            .unwrap();
        assert_eq!(output.status.success(), accepted);
        if accepted {
            assert_eq!(
                String::from_utf8(output.stdout).unwrap().trim(),
                "/dev/disk999"
            );
        }
    }
}
