use std::fs;

use backup_core::{
    error::PublicErrorCode,
    ledger::Ledger,
    rule::{BackupRule, BackupRuleDraft},
};
use dji_mic_backup_lib::{
    platform::device_registry::MountedVolume,
    rule_runtime::{RuleVolumeMatch, match_mounted_volume},
};
use tempfile::{TempDir, tempdir};
use time::UtcOffset;

fn draft(name: &str, volume_glob: &str, root: &str) -> BackupRuleDraft {
    BackupRuleDraft {
        id: None,
        name: name.to_owned(),
        archive_directory_name: name.to_owned(),
        enabled: true,
        volume_name_glob: volume_glob.to_owned(),
        required_path_globs: vec![format!("{root}/**")],
        backup_file_globs: vec![format!("{root}/**/*.WAV")],
        session_directory_globs: Vec::new(),
        filename_prefix: String::new(),
        filename_suffix: String::new(),
    }
}

fn mounted(root: &TempDir, uuid: &str, display_name: &str) -> MountedVolume {
    MountedVolume {
        descriptor: backup_core::device::VolumeDescriptor {
            volume_uuid: uuid.to_owned(),
            mount_root: root.path().to_path_buf(),
            protocol: "USB".to_owned(),
            is_internal: false,
            is_removable: true,
            is_writable: true,
            media_name: "Generic Recorder".to_owned(),
            nominal_capacity: 32_000_000_000,
            mount_generation: 1,
        },
        display_name: display_name.to_owned(),
    }
}

fn create_recording(root: &TempDir, directory: &str, name: &str) {
    fs::create_dir_all(root.path().join(directory)).unwrap();
    fs::write(root.path().join(directory).join(name), b"recorder audio").unwrap();
}

fn save(ledger: &mut Ledger, draft: BackupRuleDraft) -> BackupRule {
    ledger
        .save_backup_rule(draft, "2026-08-10T00:00:00Z")
        .unwrap()
}

fn matched(result: RuleVolumeMatch) -> dji_mic_backup_lib::rule_runtime::MatchedSource {
    let RuleVolumeMatch::Matched(source) = result else {
        panic!("expected one matched source");
    };
    *source
}

#[test]
fn exactly_one_rule_matches_while_zero_and_two_matches_fail_closed() {
    let state = tempdir().unwrap();
    let mut ledger = Ledger::open(state.path().join("ledger.sqlite3")).unwrap();
    let zoom = save(&mut ledger, draft("Zoom H1n", "ZOOM_*", "RECORD"));
    let source = tempdir().unwrap();
    create_recording(&source, "RECORD/FOLDER01", "ZOOM0001.WAV");
    let volume = mounted(&source, "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa", "ZOOM_H1N");

    let one = match_mounted_volume(
        &mut ledger,
        &volume,
        std::slice::from_ref(&zoom),
        UtcOffset::UTC,
        "2026-08-10T00:01:00Z",
    )
    .unwrap();
    assert_eq!(matched(one).rule.id, zoom.id);

    let sony = save(&mut ledger, draft("Sony PCM", "SONY_REC", "VOICE"));
    let sony_source = tempdir().unwrap();
    create_recording(&sony_source, "VOICE/01", "REC0001.WAV");
    let sony_match = match_mounted_volume(
        &mut ledger,
        &mounted(
            &sony_source,
            "cccccccc-cccc-4ccc-8ccc-cccccccccccc",
            "SONY_REC",
        ),
        std::slice::from_ref(&sony),
        UtcOffset::UTC,
        "2026-08-10T00:01:30Z",
    )
    .unwrap();
    assert_eq!(matched(sony_match).rule.id, sony.id);

    let unrelated = match_mounted_volume(
        &mut ledger,
        &volume,
        &[],
        UtcOffset::UTC,
        "2026-08-10T00:02:00Z",
    )
    .unwrap();
    assert!(matches!(unrelated, RuleVolumeMatch::Unrelated));

    let overlapping = save(&mut ledger, draft("Field Zoom", "ZOOM_*", "RECORD"));
    let conflict = match_mounted_volume(
        &mut ledger,
        &volume,
        &[overlapping.clone(), zoom.clone()],
        UtcOffset::UTC,
        "2026-08-10T00:03:00Z",
    )
    .unwrap();
    let RuleVolumeMatch::Conflict {
        rule_ids,
        display_name,
    } = conflict
    else {
        panic!("expected a fail-closed conflict");
    };
    let mut expected = vec![zoom.id.clone(), overlapping.id.clone()];
    expected.sort();
    assert_eq!(rule_ids, expected);
    assert_eq!(display_name, "ZOOM_H1N");
    assert_eq!(ledger.sources_for_rule(&overlapping.id).unwrap().len(), 0);
    assert_eq!(ledger.sources_for_rule(&zoom.id).unwrap().len(), 1);
    assert_eq!(ledger.sources_for_rule(&sony.id).unwrap().len(), 1);
}

#[test]
fn physical_rejections_happen_without_creating_source_rows() {
    let state = tempdir().unwrap();
    let mut ledger = Ledger::open(state.path().join("ledger.sqlite3")).unwrap();
    let zoom = save(&mut ledger, draft("Zoom H1n", "ZOOM_*", "RECORD"));
    let source = tempdir().unwrap();
    create_recording(&source, "RECORD/FOLDER01", "ZOOM0001.WAV");
    let baseline = mounted(&source, "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa", "ZOOM_H1N");
    let mutations: [fn(&mut MountedVolume); 4] = [
        |volume| volume.descriptor.is_internal = true,
        |volume| volume.descriptor.is_writable = false,
        |volume| volume.descriptor.is_removable = false,
        |volume| volume.descriptor.protocol = "Thunderbolt".to_owned(),
    ];

    for mutate in mutations {
        let mut candidate = baseline.clone();
        mutate(&mut candidate);
        let result = match_mounted_volume(
            &mut ledger,
            &candidate,
            std::slice::from_ref(&zoom),
            UtcOffset::UTC,
            "2026-08-10T00:01:00Z",
        )
        .unwrap();
        let RuleVolumeMatch::Rejected(error) = result else {
            panic!("expected physical rejection");
        };
        assert_eq!(error.code, PublicErrorCode::IdentityMismatch);
    }
    assert!(ledger.sources_for_rule(&zoom.id).unwrap().is_empty());
}

#[test]
fn source_identity_survives_remount_but_distinguishes_equal_labels() {
    let state = tempdir().unwrap();
    let mut ledger = Ledger::open(state.path().join("ledger.sqlite3")).unwrap();
    let zoom = save(&mut ledger, draft("Zoom H1n", "ZOOM_*", "RECORD"));
    let first_root = tempdir().unwrap();
    create_recording(&first_root, "RECORD/FOLDER01", "ZOOM0001.WAV");
    let mut first_mount = mounted(
        &first_root,
        "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa",
        "ZOOM_H1N",
    );
    let first = matched(
        match_mounted_volume(
            &mut ledger,
            &first_mount,
            std::slice::from_ref(&zoom),
            UtcOffset::UTC,
            "2026-08-10T00:01:00Z",
        )
        .unwrap(),
    );
    first_mount.descriptor.mount_generation = 2;
    let remounted = matched(
        match_mounted_volume(
            &mut ledger,
            &first_mount,
            std::slice::from_ref(&zoom),
            UtcOffset::UTC,
            "2026-08-10T00:02:00Z",
        )
        .unwrap(),
    );
    assert_eq!(first.authority.source.id, remounted.authority.source.id);
    assert_ne!(
        first.authority.descriptor.mount_generation,
        remounted.authority.descriptor.mount_generation
    );

    let second_root = tempdir().unwrap();
    create_recording(&second_root, "RECORD/FOLDER01", "ZOOM0001.WAV");
    let second = matched(
        match_mounted_volume(
            &mut ledger,
            &mounted(
                &second_root,
                "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb",
                "ZOOM_H1N",
            ),
            std::slice::from_ref(&zoom),
            UtcOffset::UTC,
            "2026-08-10T00:03:00Z",
        )
        .unwrap(),
    );
    assert_ne!(first.authority.source.id, second.authority.source.id);
    assert_eq!(ledger.sources_for_rule(&zoom.id).unwrap().len(), 2);
}
