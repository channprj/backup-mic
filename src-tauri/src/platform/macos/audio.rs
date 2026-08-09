use std::{path::Path, process::Command};

use backup_core::{artifact::AudioDescription, error::CoreError};
use serde::Deserialize;

const MAX_TOOL_OUTPUT_BYTES: usize = 4 * 1024 * 1024;

pub trait AudioTools: Send + Sync {
    fn inspect(&self, path: &Path) -> Result<AudioDescription, CoreError>;
    fn convert_aac_lc_192k(&self, input: &Path, output: &Path) -> Result<(), CoreError>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct AppleAudioTools;

impl AudioTools for AppleAudioTools {
    fn inspect(&self, path: &Path) -> Result<AudioDescription, CoreError> {
        if !path.is_absolute() {
            return Err(CoreError::InvalidRequest);
        }
        let output = Command::new("/usr/bin/afinfo")
            .arg("-x")
            .arg(path)
            .output()
            .map_err(|_| CoreError::AudioToolFailed)?;
        if !output.status.success()
            || output.stdout.is_empty()
            || output.stdout.len() > MAX_TOOL_OUTPUT_BYTES
            || output.stderr.len() > MAX_TOOL_OUTPUT_BYTES
        {
            return Err(CoreError::AudioToolFailed);
        }
        parse_afinfo(&output.stdout)
    }

    fn convert_aac_lc_192k(&self, input: &Path, output: &Path) -> Result<(), CoreError> {
        if !input.is_absolute() || !output.is_absolute() || output.exists() {
            return Err(CoreError::InvalidRequest);
        }
        let result = Command::new("/usr/bin/afconvert")
            .arg(input)
            .arg("-o")
            .arg(output)
            .args(["-f", "m4af", "-d", "aac", "-b", "192000"])
            .args(["-q", "127", "-s", "2"])
            .output()
            .map_err(|_| CoreError::AudioToolFailed)?;
        if !result.status.success()
            || result.stdout.len() > MAX_TOOL_OUTPUT_BYTES
            || result.stderr.len() > MAX_TOOL_OUTPUT_BYTES
        {
            return Err(CoreError::AudioToolFailed);
        }
        let metadata = std::fs::symlink_metadata(output).map_err(CoreError::CopyFailed)?;
        if !metadata.file_type().is_file() || metadata.len() == 0 {
            return Err(CoreError::AudioToolFailed);
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
struct AudioInfo {
    audio_file: AudioFile,
}

#[derive(Debug, Deserialize)]
struct AudioFile {
    file_type: String,
    tracks: Tracks,
}

#[derive(Debug, Deserialize)]
struct Tracks {
    track: Vec<Track>,
}

#[derive(Debug, Deserialize)]
struct Track {
    num_channels: u16,
    sample_rate: u32,
    format_type: String,
    audio_bytes: u64,
    audio_packets: u64,
    duration: f64,
    frames_per_packet: Option<u64>,
    packet_table_info: Option<PacketTableInfo>,
}

#[derive(Debug, Deserialize)]
struct PacketTableInfo {
    total_frames: u64,
    valid_frames: u64,
    priming_frames: u64,
    remainder_frames: u64,
}

fn parse_afinfo(xml: &[u8]) -> Result<AudioDescription, CoreError> {
    let info: AudioInfo =
        quick_xml::de::from_reader(xml).map_err(|_| CoreError::ArtifactInvalid)?;
    let track = info
        .audio_file
        .tracks
        .track
        .into_iter()
        .next()
        .ok_or(CoreError::ArtifactInvalid)?;
    if !track.duration.is_finite() || track.duration <= 0.0 {
        return Err(CoreError::ArtifactInvalid);
    }
    let frames_per_packet = track.frames_per_packet.unwrap_or(1);
    let fallback_frames = track
        .audio_packets
        .checked_mul(frames_per_packet)
        .ok_or(CoreError::ArtifactInvalid)?;
    let (total_frames, valid_frames, priming_frames, remainder_frames) = track
        .packet_table_info
        .map_or((fallback_frames, fallback_frames, 0, 0), |packet| {
            (
                packet.total_frames,
                packet.valid_frames,
                packet.priming_frames,
                packet.remainder_frames,
            )
        });
    let duration_micros = (track.duration * 1_000_000.0).round();
    if duration_micros <= 0.0 || duration_micros > u64::MAX as f64 {
        return Err(CoreError::ArtifactInvalid);
    }
    Ok(AudioDescription {
        container: info
            .audio_file
            .file_type
            .trim()
            .trim_matches('\'')
            .to_owned(),
        codec: track.format_type.trim().to_owned(),
        sample_rate_hz: track.sample_rate,
        channels: track.num_channels,
        audio_bytes: track.audio_bytes,
        packets: track.audio_packets,
        frames_per_packet,
        valid_frames,
        total_frames,
        priming_frames,
        remainder_frames,
        duration_micros: duration_micros as u64,
    })
}
