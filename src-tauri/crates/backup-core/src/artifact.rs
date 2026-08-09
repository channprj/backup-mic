use std::path::PathBuf;

use serde::{Deserialize, Serialize};

pub const M4A_TARGET_BITRATE_BPS: u32 = 128_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OutputFormat {
    Wav,
    M4a,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerifiedAudioProperties {
    pub codec: String,
    pub sample_rate_hz: u32,
    pub channel_count: u16,
    pub valid_frames: u64,
    pub duration_micros: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioDescription {
    pub container: String,
    pub codec: String,
    pub sample_rate_hz: u32,
    pub channels: u16,
    pub audio_bytes: u64,
    pub packets: u64,
    pub frames_per_packet: u64,
    pub valid_frames: u64,
    pub total_frames: u64,
    pub priming_frames: u64,
    pub remainder_frames: u64,
    pub duration_micros: u64,
}

pub fn validate_m4a_artifact(
    source: &AudioDescription,
    converted: &AudioDescription,
) -> Result<VerifiedAudioProperties, crate::error::CoreError> {
    let declared_total = converted
        .valid_frames
        .checked_add(converted.priming_frames)
        .and_then(|frames| frames.checked_add(converted.remainder_frames));
    let duration_difference = source.duration_micros.abs_diff(converted.duration_micros);
    let packet_tolerance_micros = converted
        .frames_per_packet
        .checked_mul(1_000_000)
        .and_then(|value| value.checked_div(u64::from(converted.sample_rate_hz)))
        .unwrap_or(0);
    if source.container != "WAVE"
        || source.codec != "lpcm"
        || source.sample_rate_hz == 0
        || source.channels == 0
        || source.valid_frames == 0
        || converted.container != "m4af"
        || converted.codec != "aac"
        || converted.sample_rate_hz != source.sample_rate_hz
        || converted.channels != source.channels
        || converted.audio_bytes == 0
        || converted.packets == 0
        || converted.frames_per_packet == 0
        || converted.valid_frames != source.valid_frames
        || declared_total != Some(converted.total_frames)
        || duration_difference > packet_tolerance_micros
    {
        return Err(crate::error::CoreError::ArtifactInvalid);
    }
    Ok(VerifiedAudioProperties {
        codec: converted.codec.clone(),
        sample_rate_hz: converted.sample_rate_hz,
        channel_count: converted.channels,
        valid_frames: converted.valid_frames,
        duration_micros: converted.duration_micros,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedArtifact {
    pub relative_path: PathBuf,
    pub format: OutputFormat,
    pub byte_count: u64,
    pub sha256: String,
    pub audio: Option<VerifiedAudioProperties>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConversionStatus {
    NotRequired,
    Pending,
    Complete,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetirementStatus {
    Present,
    TrashPending,
    MovedToTrash,
    LegacyDeleted,
    Failed,
}
