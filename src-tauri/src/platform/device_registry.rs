use std::{collections::HashMap, path::PathBuf};

use backup_core::device::VolumeDescriptor;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeDiskDescription {
    pub disk_id: String,
    pub volume_uuid: Option<String>,
    pub mount_root: Option<PathBuf>,
    pub protocol: Option<String>,
    pub is_internal: Option<bool>,
    pub is_removable: Option<bool>,
    pub is_writable: Option<bool>,
    pub media_name: Option<String>,
    pub nominal_capacity: Option<u64>,
    pub display_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NativeDiskEvent {
    Appeared(NativeDiskDescription),
    DescriptionChanged(NativeDiskDescription),
    Disappeared { disk_id: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MountedVolume {
    pub descriptor: VolumeDescriptor,
    pub display_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VolumeLifecycleEvent {
    Mounted(MountedVolume),
    Unmounted {
        volume_uuid: String,
        mount_generation: u64,
    },
}

#[derive(Debug, Default)]
pub struct DeviceRegistry {
    by_disk_id: HashMap<String, MountedVolume>,
    generations: HashMap<String, u64>,
}

impl DeviceRegistry {
    pub fn apply(&mut self, event: NativeDiskEvent) -> Vec<VolumeLifecycleEvent> {
        match event {
            NativeDiskEvent::Disappeared { disk_id } => self
                .by_disk_id
                .remove(&disk_id)
                .map(unmounted_event)
                .into_iter()
                .collect(),
            NativeDiskEvent::Appeared(description)
            | NativeDiskEvent::DescriptionChanged(description) => {
                self.apply_description(description)
            }
        }
    }

    fn apply_description(
        &mut self,
        description: NativeDiskDescription,
    ) -> Vec<VolumeLifecycleEvent> {
        let disk_id = description.disk_id.clone();
        let Some(mut mounted) = complete_volume(description) else {
            return self
                .by_disk_id
                .remove(&disk_id)
                .map(unmounted_event)
                .into_iter()
                .collect();
        };
        if let Some(existing) = self.by_disk_id.get(&disk_id)
            && existing.descriptor.volume_uuid == mounted.descriptor.volume_uuid
            && existing.descriptor.mount_root == mounted.descriptor.mount_root
        {
            mounted.descriptor.mount_generation = existing.descriptor.mount_generation;
            if existing == &mounted {
                return Vec::new();
            }
            self.by_disk_id.insert(disk_id, mounted.clone());
            return vec![VolumeLifecycleEvent::Mounted(mounted)];
        }

        let previous = self.by_disk_id.remove(&disk_id);
        let generation = self
            .generations
            .entry(mounted.descriptor.volume_uuid.clone())
            .or_default();
        *generation = generation.saturating_add(1);
        mounted.descriptor.mount_generation = *generation;
        self.by_disk_id.insert(disk_id, mounted.clone());
        let mut events = previous
            .map(unmounted_event)
            .into_iter()
            .collect::<Vec<_>>();
        events.push(VolumeLifecycleEvent::Mounted(mounted));
        events
    }
}

fn complete_volume(description: NativeDiskDescription) -> Option<MountedVolume> {
    Some(MountedVolume {
        descriptor: VolumeDescriptor {
            volume_uuid: description.volume_uuid?,
            mount_root: description.mount_root?,
            protocol: description.protocol?,
            is_internal: description.is_internal?,
            is_removable: description.is_removable?,
            is_writable: description.is_writable?,
            media_name: description.media_name?,
            nominal_capacity: description.nominal_capacity?,
            mount_generation: 0,
        },
        display_name: description
            .display_name
            .unwrap_or_else(|| "External Recorder".to_owned()),
    })
}

fn unmounted_event(mounted: MountedVolume) -> VolumeLifecycleEvent {
    VolumeLifecycleEvent::Unmounted {
        volume_uuid: mounted.descriptor.volume_uuid,
        mount_generation: mounted.descriptor.mount_generation,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn description(mounted: bool) -> NativeDiskDescription {
        NativeDiskDescription {
            disk_id: "disk-test".to_owned(),
            volume_uuid: Some("11111111-1111-4111-8111-111111111111".to_owned()),
            mount_root: mounted.then(|| PathBuf::from("/Volumes/RENAMED")),
            protocol: Some("USB".to_owned()),
            is_internal: Some(false),
            is_removable: Some(true),
            is_writable: Some(true),
            media_name: Some("Wireless Mic Tx Media".to_owned()),
            nominal_capacity: Some(15_636_365_312),
            display_name: Some("RENAMED".to_owned()),
        }
    }

    #[test]
    fn waits_for_mount_completion_and_coalesces_repeated_descriptions() {
        let mut registry = DeviceRegistry::default();
        assert!(
            registry
                .apply(NativeDiskEvent::Appeared(description(false)))
                .is_empty()
        );
        let mounted = registry
            .apply(NativeDiskEvent::DescriptionChanged(description(true)))
            .pop()
            .unwrap();
        let VolumeLifecycleEvent::Mounted(mounted) = mounted else {
            panic!("expected mounted event");
        };
        assert_eq!(mounted.descriptor.mount_generation, 1);
        assert!(
            registry
                .apply(NativeDiskEvent::DescriptionChanged(description(true)))
                .is_empty()
        );
    }

    #[test]
    fn disappearance_and_reconnect_increment_the_mount_generation() {
        let mut registry = DeviceRegistry::default();
        registry.apply(NativeDiskEvent::Appeared(description(true)));
        assert_eq!(
            registry.apply(NativeDiskEvent::Disappeared {
                disk_id: "disk-test".to_owned()
            }),
            vec![VolumeLifecycleEvent::Unmounted {
                volume_uuid: "11111111-1111-4111-8111-111111111111".to_owned(),
                mount_generation: 1,
            }]
        );
        let reconnected = registry
            .apply(NativeDiskEvent::Appeared(description(true)))
            .pop()
            .unwrap();
        let VolumeLifecycleEvent::Mounted(reconnected) = reconnected else {
            panic!("expected mount");
        };
        assert_eq!(reconnected.descriptor.mount_generation, 2);
    }
}
