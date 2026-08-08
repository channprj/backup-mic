use std::collections::HashMap;

use backup_core::{
    device::{DeviceMatch, PairedDevice, match_volume},
    error::CoreError,
    ledger::Ledger,
    state::Transmitter,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::platform::device_registry::MountedVolume;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PairingCandidateSummary {
    pub candidate_id: String,
    pub display_name: String,
    pub capacity_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PairingAssignment {
    pub candidate_id: String,
    pub transmitter: Transmitter,
}

#[derive(Debug, Clone)]
struct PrivateCandidate {
    mounted: MountedVolume,
}

#[derive(Debug, Default)]
pub struct PairingManager {
    candidates: HashMap<String, PrivateCandidate>,
}

impl PairingManager {
    pub fn observe(&mut self, mounted: MountedVolume, paired: &[PairedDevice]) -> DeviceMatch {
        let result = match_volume(&mounted.descriptor, paired);
        self.candidates.retain(|_, candidate| {
            !candidate
                .mounted
                .descriptor
                .volume_uuid
                .eq_ignore_ascii_case(&mounted.descriptor.volume_uuid)
        });
        if result == DeviceMatch::UnpairedCandidate {
            self.candidates
                .insert(Uuid::new_v4().to_string(), PrivateCandidate { mounted });
        }
        result
    }

    pub fn remove(&mut self, volume_uuid: &str, mount_generation: u64) {
        self.candidates.retain(|_, candidate| {
            !(candidate
                .mounted
                .descriptor
                .volume_uuid
                .eq_ignore_ascii_case(volume_uuid)
                && candidate.mounted.descriptor.mount_generation == mount_generation)
        });
    }

    pub fn summaries(&self) -> Vec<PairingCandidateSummary> {
        let mut summaries = self
            .candidates
            .iter()
            .map(|(candidate_id, candidate)| PairingCandidateSummary {
                candidate_id: candidate_id.clone(),
                display_name: candidate.mounted.display_name.clone(),
                capacity_bytes: candidate.mounted.descriptor.nominal_capacity,
            })
            .collect::<Vec<_>>();
        summaries.sort_by(|left, right| {
            left.display_name
                .cmp(&right.display_name)
                .then_with(|| left.candidate_id.cmp(&right.candidate_id))
        });
        summaries
    }

    pub fn pair(
        &mut self,
        assignments: &[PairingAssignment],
        ledger: &mut Ledger,
        paired_at: &str,
    ) -> Result<Vec<PairedDevice>, CoreError> {
        if assignments.len() != 2 {
            return Err(CoreError::InvalidRequest);
        }
        let unique_candidates = assignments
            .iter()
            .map(|assignment| assignment.candidate_id.as_str())
            .collect::<std::collections::HashSet<_>>();
        let unique_transmitters = assignments
            .iter()
            .map(|assignment| assignment.transmitter)
            .collect::<std::collections::HashSet<_>>();
        if unique_candidates.len() != assignments.len()
            || unique_transmitters.len() != assignments.len()
        {
            return Err(CoreError::InvalidRequest);
        }
        let paired = assignments
            .iter()
            .map(|assignment| {
                let candidate = self
                    .candidates
                    .get(&assignment.candidate_id)
                    .ok_or(CoreError::InvalidRequest)?;
                Ok(PairedDevice {
                    transmitter: assignment.transmitter,
                    expected_uuid: candidate.mounted.descriptor.volume_uuid.clone(),
                    expected_protocol: candidate.mounted.descriptor.protocol.clone(),
                    expected_media_name: candidate.mounted.descriptor.media_name.clone(),
                    expected_capacity: candidate.mounted.descriptor.nominal_capacity,
                })
            })
            .collect::<Result<Vec<_>, CoreError>>()?;
        ledger.pair_devices(&paired, paired_at)?;
        for assignment in assignments {
            self.candidates.remove(&assignment.candidate_id);
        }
        Ok(paired)
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use backup_core::device::{DeviceMatch, VolumeDescriptor};
    use tempfile::tempdir;

    use super::*;

    fn mounted(generation: u64) -> MountedVolume {
        MountedVolume {
            descriptor: VolumeDescriptor {
                volume_uuid: "11111111-1111-4111-8111-111111111111".to_owned(),
                mount_root: PathBuf::from("/Volumes/RENAMED"),
                protocol: "USB".to_owned(),
                is_internal: false,
                is_removable: true,
                is_writable: true,
                media_name: "Wireless Mic Tx Media".to_owned(),
                nominal_capacity: 15_636_365_312,
                mount_generation: generation,
            },
            display_name: "RENAMED".to_owned(),
        }
    }

    #[test]
    fn candidate_ids_are_opaque_and_stale_after_reconnect() {
        let state = tempdir().unwrap();
        let mut ledger = Ledger::open(state.path().join("ledger.sqlite3")).unwrap();
        let mut manager = PairingManager::default();
        assert_eq!(
            manager.observe(mounted(1), &[]),
            DeviceMatch::UnpairedCandidate
        );
        let first = manager.summaries().pop().unwrap();
        assert!(!first.candidate_id.contains("11111111"));

        manager.remove("11111111-1111-4111-8111-111111111111", 1);
        manager.observe(mounted(2), &[]);
        assert!(matches!(
            manager.pair(
                &[PairingAssignment {
                    candidate_id: first.candidate_id,
                    transmitter: Transmitter::Tx01,
                }],
                &mut ledger,
                "2026-08-09T00:00:00Z"
            ),
            Err(CoreError::InvalidRequest)
        ));
    }

    #[test]
    fn pairs_distinct_candidates_transactionally_and_rejects_duplicate_tx_labels() {
        let state = tempdir().unwrap();
        let mut ledger = Ledger::open(state.path().join("ledger.sqlite3")).unwrap();
        let mut manager = PairingManager::default();
        manager.observe(mounted(1), &[]);
        let mut second = mounted(1);
        second.descriptor.volume_uuid = "22222222-2222-4222-8222-222222222222".to_owned();
        second.display_name = "SECOND".to_owned();
        manager.observe(second, &[]);
        let summaries = manager.summaries();
        let duplicate = summaries
            .iter()
            .map(|candidate| PairingAssignment {
                candidate_id: candidate.candidate_id.clone(),
                transmitter: Transmitter::Tx01,
            })
            .collect::<Vec<_>>();
        assert!(matches!(
            manager.pair(&duplicate, &mut ledger, "2026-08-09T00:00:00Z"),
            Err(CoreError::InvalidRequest)
        ));
        assert_eq!(ledger.paired_device_count().unwrap(), 0);

        let assignments = summaries
            .iter()
            .zip([Transmitter::Tx01, Transmitter::Tx02])
            .map(|(candidate, transmitter)| PairingAssignment {
                candidate_id: candidate.candidate_id.clone(),
                transmitter,
            })
            .collect::<Vec<_>>();
        let paired = manager
            .pair(&assignments, &mut ledger, "2026-08-09T00:00:00Z")
            .unwrap();
        assert_eq!(paired.len(), 2);
        assert_eq!(ledger.paired_device_count().unwrap(), 2);
    }
}
