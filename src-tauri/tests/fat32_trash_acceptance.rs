use std::{fs, path::PathBuf};

use backup_core::{deletion::TrashAdapter, hash::hash_file};
use dji_mic_backup_lib::platform::macos::{
    audio::{AppleAudioTools, AudioTools},
    trash::MacTrash,
};

const FIXTURE_ROOT: &str = "/Volumes/DJI-DELTEST";
const MARKER: &str = ".dji-mic-backup-delete-fixture";

#[test]
#[ignore = "requires scripts/accept-deletion-fixture.sh"]
fn foundation_moves_a_whole_session_on_the_isolated_fat32_fixture() {
    let requested = std::env::var_os("DJI_MIC_DELETION_FIXTURE")
        .map(PathBuf::from)
        .expect("fixture path is required");
    assert_eq!(requested, PathBuf::from(FIXTURE_ROOT));
    let fixture = fs::canonicalize(&requested).expect("fixture must be mounted");
    assert_eq!(fixture, PathBuf::from(FIXTURE_ROOT));
    assert_eq!(
        fs::read_to_string(fixture.join(MARKER)).expect("fixture marker is required"),
        "isolated-fat32-trash-test\n"
    );

    let session = fixture.join("TX_MIC001_20260809_021747");
    assert!(!session.exists());
    fs::create_dir(&session).unwrap();
    let files = [
        ("TX01_MIC001_20260809_021747.wav", write_pcm_wav(17)),
        ("TX01_MIC001_20260809_021748.wav", write_pcm_wav(29)),
        ("recorder-preview.m4a", b"external M4A fixture".to_vec()),
        (
            "._TX01_MIC001_20260809_021747.wav",
            b"AppleDouble fixture".to_vec(),
        ),
    ];
    for (name, contents) in &files {
        fs::write(session.join(name), contents).unwrap();
    }

    let destination = fixture.join("platform-destination");
    fs::create_dir(&destination).unwrap();
    let tools = AppleAudioTools;
    for (name, _) in &files[..2] {
        let source_wav = session.join(name);
        let destination_m4a = destination.join(PathBuf::from(name).with_extension("m4a"));
        tools
            .convert_aac_lc_128k(&source_wav, &destination_m4a)
            .unwrap();
        let inspected = tools.inspect(&destination_m4a).unwrap();
        assert_eq!(inspected.container, "m4af");
        assert_eq!(inspected.codec, "aac");
        assert_eq!(inspected.sample_rate_hz, 48_000);
        assert_eq!(inspected.channels, 1);
        assert_eq!(inspected.valid_frames, 48_000);
    }
    for (name, _) in &files[2..] {
        let source_extra = session.join(name);
        let destination_extra = destination.join("source-extras").join(name);
        fs::create_dir_all(destination_extra.parent().unwrap()).unwrap();
        fs::copy(&source_extra, &destination_extra).unwrap();
        assert_eq!(
            hash_file(&source_extra).unwrap(),
            hash_file(&destination_extra).unwrap()
        );
    }

    let empty_session = fixture.join("TX_MIC001_20260809_021749");
    assert!(!empty_session.exists());
    fs::create_dir(&empty_session).unwrap();

    MacTrash.move_to_trash(&session).unwrap();
    MacTrash.move_to_trash(&empty_session).unwrap();

    assert!(!session.exists());
    assert!(!empty_session.exists());

    // Direct Trash inspection is intentionally confined to this disposable
    // acceptance image. Production code only calls Foundation's Trash API.
    let trash = fixture
        .join(".Trashes")
        .join(unsafe { libc::geteuid() }.to_string());
    let trashed_session = trash.join("TX_MIC001_20260809_021747");
    assert!(trashed_session.is_dir());
    for (name, contents) in files {
        assert_eq!(fs::read(trashed_session.join(name)).unwrap(), contents);
    }
    assert!(trash.join("TX_MIC001_20260809_021749").is_dir());
}

fn write_pcm_wav(pattern: i16) -> Vec<u8> {
    let sample_rate = 48_000_u32;
    let channels = 1_u16;
    let frames = 48_000_u32;
    let bits_per_sample = 16_u16;
    let block_align = channels * (bits_per_sample / 8);
    let byte_rate = sample_rate * u32::from(block_align);
    let data_size = frames * u32::from(block_align);
    let mut wav = Vec::with_capacity(44 + usize::try_from(data_size).unwrap());
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_size).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16_u32.to_le_bytes());
    wav.extend_from_slice(&1_u16.to_le_bytes());
    wav.extend_from_slice(&channels.to_le_bytes());
    wav.extend_from_slice(&sample_rate.to_le_bytes());
    wav.extend_from_slice(&byte_rate.to_le_bytes());
    wav.extend_from_slice(&block_align.to_le_bytes());
    wav.extend_from_slice(&bits_per_sample.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_size.to_le_bytes());
    for frame in 0..frames {
        let sample = (((frame % 200) as i16) - 100) * pattern;
        wav.extend_from_slice(&sample.to_le_bytes());
    }
    wav
}
