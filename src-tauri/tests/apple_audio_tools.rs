use std::{ffi::OsString, fs, path::Path};

use dji_mic_backup_lib::platform::macos::audio::{
    AppleAudioTools, AudioTools, afconvert_arguments,
};
use tempfile::tempdir;

#[test]
fn production_apple_tools_convert_and_inspect_a_pcm_wav() {
    let directory = tempdir().unwrap();
    let wav = directory.path().join("fixture.wav");
    let m4a = directory.path().join("fixture.m4a");
    write_pcm_wav(&wav, 48_000, 1, 48_000);
    let tools = AppleAudioTools;

    let source = tools.inspect(&wav).unwrap();
    tools.convert_aac_lc_128k(&wav, &m4a).unwrap();
    let converted = tools.inspect(&m4a).unwrap();

    assert_eq!(source.container, "WAVE");
    assert_eq!(source.codec, "lpcm");
    assert_eq!(source.valid_frames, 48_000);
    assert_eq!(converted.container, "m4af");
    assert_eq!(converted.codec, "aac");
    assert_eq!(converted.sample_rate_hz, 48_000);
    assert_eq!(converted.channels, 1);
    assert_eq!(converted.valid_frames, 48_000);
    assert!(converted.audio_bytes > 0);
    assert!(converted.packets > 0);
}

#[test]
fn afconvert_profile_is_exact_aac_lc_128k_without_a_shell_or_old_bitrate() {
    let input = Path::new("/tmp/source.wav");
    let output = Path::new("/tmp/output.m4a");
    let arguments = afconvert_arguments(input, output);

    assert_eq!(
        arguments,
        [
            input.as_os_str().to_owned(),
            OsString::from("-o"),
            output.as_os_str().to_owned(),
            OsString::from("-f"),
            OsString::from("m4af"),
            OsString::from("-d"),
            OsString::from("aac"),
            OsString::from("-b"),
            OsString::from("128000"),
            OsString::from("-q"),
            OsString::from("127"),
            OsString::from("-s"),
            OsString::from("2"),
        ]
    );
    assert!(!arguments.iter().any(|argument| argument == "-c"));
    assert!(!arguments.iter().any(|argument| argument == "192000"));
    assert!(!arguments.iter().any(|argument| argument == "/bin/sh"));
}

fn write_pcm_wav(path: &Path, sample_rate: u32, channels: u16, frames: u32) {
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
        let sample = (((frame % 200) as i16) - 100) * 100;
        for _ in 0..channels {
            wav.extend_from_slice(&sample.to_le_bytes());
        }
    }
    fs::write(path, wav).unwrap();
}
