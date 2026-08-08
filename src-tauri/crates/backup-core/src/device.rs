use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::{error::PublicError, state::Transmitter};

const MINIMUM_CAPACITY_TOLERANCE: u64 = 256 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VolumeDescriptor {
    pub volume_uuid: String,
    pub mount_root: PathBuf,
    pub protocol: String,
    pub is_internal: bool,
    pub is_removable: bool,
    pub is_writable: bool,
    pub media_name: String,
    pub nominal_capacity: u64,
    pub mount_generation: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairedDevice {
    pub transmitter: Transmitter,
    pub expected_uuid: String,
    pub expected_protocol: String,
    pub expected_media_name: String,
    pub expected_capacity: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceMatch {
    Trusted(Transmitter),
    UnpairedCandidate,
    Rejected(PublicError),
    Unrelated,
}

pub fn match_volume(volume: &VolumeDescriptor, paired: &[PairedDevice]) -> DeviceMatch {
    if let Some(expected) = paired.iter().find(|expected| {
        expected
            .expected_uuid
            .eq_ignore_ascii_case(&volume.volume_uuid)
    }) {
        if paired_properties_match(volume, expected) {
            return DeviceMatch::Trusted(expected.transmitter);
        }
        return DeviceMatch::Rejected(
            crate::error::CoreError::IdentityMismatch.public(Some(expected.transmitter)),
        );
    }
    if looks_like_dji_transmitter(volume) {
        DeviceMatch::UnpairedCandidate
    } else {
        DeviceMatch::Unrelated
    }
}

fn paired_properties_match(volume: &VolumeDescriptor, expected: &PairedDevice) -> bool {
    let tolerance = (expected.expected_capacity / 100).max(MINIMUM_CAPACITY_TOLERANCE);
    volume
        .protocol
        .eq_ignore_ascii_case(&expected.expected_protocol)
        && volume.protocol.eq_ignore_ascii_case("USB")
        && !volume.is_internal
        && volume.is_removable
        && volume.is_writable
        && accepted_media_name(&volume.media_name)
        && accepted_media_name(&expected.expected_media_name)
        && volume.nominal_capacity.abs_diff(expected.expected_capacity) <= tolerance
}

fn looks_like_dji_transmitter(volume: &VolumeDescriptor) -> bool {
    volume.protocol.eq_ignore_ascii_case("USB")
        && !volume.is_internal
        && volume.is_removable
        && volume.is_writable
        && accepted_media_name(&volume.media_name)
        && (12_000_000_000..=20_000_000_000).contains(&volume.nominal_capacity)
}

fn accepted_media_name(name: &str) -> bool {
    name.eq_ignore_ascii_case("Mic Tx") || name.eq_ignore_ascii_case("Wireless Mic Tx Media")
}

#[cfg(test)]
mod tests {
    use crate::error::PublicErrorCode;

    use super::*;

    fn volume() -> VolumeDescriptor {
        VolumeDescriptor {
            volume_uuid: "11111111-1111-4111-8111-111111111111".to_owned(),
            mount_root: PathBuf::from("/Volumes/RENAMED"),
            protocol: "USB".to_owned(),
            is_internal: false,
            is_removable: true,
            is_writable: true,
            media_name: "Wireless Mic Tx Media".to_owned(),
            nominal_capacity: 15_636_365_312,
            mount_generation: 7,
        }
    }

    fn paired() -> PairedDevice {
        PairedDevice {
            transmitter: Transmitter::Tx01,
            expected_uuid: "11111111-1111-4111-8111-111111111111".to_owned(),
            expected_protocol: "USB".to_owned(),
            expected_media_name: "Wireless Mic Tx Media".to_owned(),
            expected_capacity: 15_636_365_312,
        }
    }

    #[test]
    fn trusts_a_paired_device_after_its_label_and_mount_path_change() {
        assert_eq!(
            match_volume(&volume(), &[paired()]),
            DeviceMatch::Trusted(Transmitter::Tx01)
        );
    }

    #[test]
    fn rejects_uuid_match_when_any_physical_property_changes() {
        let mutations: [fn(&mut VolumeDescriptor); 6] = [
            |value| value.protocol = "Thunderbolt".to_owned(),
            |value| value.is_internal = true,
            |value| value.is_removable = false,
            |value| value.is_writable = false,
            |value| value.media_name = "Generic Drive".to_owned(),
            |value| value.nominal_capacity += 512 * 1024 * 1024,
        ];
        for mutate in mutations {
            let mut candidate = volume();
            mutate(&mut candidate);
            match match_volume(&candidate, &[paired()]) {
                DeviceMatch::Rejected(error) => {
                    assert_eq!(error.code, PublicErrorCode::IdentityMismatch);
                }
                result => panic!("unexpected match result: {result:?}"),
            }
        }
    }

    #[test]
    fn exposes_dji_shaped_unpaired_media_as_a_pairing_candidate() {
        assert_eq!(match_volume(&volume(), &[]), DeviceMatch::UnpairedCandidate);
    }
}
