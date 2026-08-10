use backup_core::{
    error::{CoreError, PublicError},
    ledger::Ledger,
    rule::{BackupRule, DeviceConstraintProfile, RuleId, compile_rule},
    rule_scanner::{RuleScanFingerprint, scan_rule_once},
    source::{MountedSourceAuthority, SourceId, SourceRecord},
};
use time::UtcOffset;

use crate::platform::device_registry::MountedVolume;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RuleVolumeMatch {
    Unrelated,
    Matched(Box<MatchedSource>),
    Conflict {
        rule_ids: Vec<RuleId>,
        display_name: String,
    },
    Rejected(PublicError),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MatchedSource {
    pub authority: MountedSourceAuthority,
    pub rule: BackupRule,
    pub mounted: MountedVolume,
    pub initial_fingerprint: RuleScanFingerprint,
}

pub fn match_mounted_volume(
    ledger: &mut Ledger,
    mounted: &MountedVolume,
    rules: &[BackupRule],
    local_offset: UtcOffset,
    seen_at: &str,
) -> Result<RuleVolumeMatch, CoreError> {
    let display_name = sanitized_display_name(&mounted.display_name);
    let mut candidates = Vec::new();
    for rule in rules
        .iter()
        .filter(|rule| rule.enabled && rule.archived_at.is_none())
    {
        let compiled = compile_rule(rule.clone())?;
        if compiled.matches_volume_name(&display_name)
            && constraint_matches(mounted, rule.device_constraint_profile)
        {
            candidates.push((rule.clone(), compiled));
        }
    }
    if candidates.is_empty() {
        return Ok(RuleVolumeMatch::Unrelated);
    }
    if !meets_shared_external_policy(mounted) {
        return Ok(RuleVolumeMatch::Rejected(
            CoreError::IdentityMismatch.public(None),
        ));
    }
    let mut matches = Vec::new();
    for (rule, compiled) in candidates {
        let scan = match scan_rule_once(&mounted.descriptor.mount_root, &compiled, local_offset) {
            Ok(scan) => scan,
            Err(error) => return Ok(RuleVolumeMatch::Rejected(error.public(None))),
        };
        if !scan.files.is_empty() {
            matches.push((rule, scan.fingerprint));
        }
    }
    if matches.is_empty() {
        return Ok(RuleVolumeMatch::Unrelated);
    }
    if matches.len() > 1 {
        let mut rule_ids = matches
            .into_iter()
            .map(|(rule, _)| rule.id)
            .collect::<Vec<_>>();
        rule_ids.sort();
        rule_ids.dedup();
        return Ok(RuleVolumeMatch::Conflict {
            rule_ids,
            display_name,
        });
    }
    let (rule, initial_fingerprint) = matches.pop().ok_or(CoreError::InvalidRequest)?;
    let source = ledger
        .sources_for_rule(&rule.id)?
        .into_iter()
        .find(|source| {
            source
                .volume_uuid
                .eq_ignore_ascii_case(&mounted.descriptor.volume_uuid)
        })
        .unwrap_or(SourceRecord {
            id: SourceId::new(),
            rule_id: rule.id.clone(),
            volume_uuid: mounted.descriptor.volume_uuid.clone(),
            legacy_slot: None,
            display_name: display_name.clone(),
        });
    let source = SourceRecord {
        display_name,
        ..source
    };
    ledger.upsert_source(&source, seen_at)?;
    Ok(RuleVolumeMatch::Matched(Box::new(MatchedSource {
        authority: MountedSourceAuthority {
            source,
            descriptor: mounted.descriptor.clone(),
        },
        rule,
        mounted: mounted.clone(),
        initial_fingerprint,
    })))
}

fn meets_shared_external_policy(mounted: &MountedVolume) -> bool {
    mounted.descriptor.protocol.eq_ignore_ascii_case("USB")
        && !mounted.descriptor.is_internal
        && mounted.descriptor.is_removable
        && mounted.descriptor.is_writable
}

fn constraint_matches(mounted: &MountedVolume, profile: DeviceConstraintProfile) -> bool {
    match profile {
        DeviceConstraintProfile::GenericExternal => true,
        DeviceConstraintProfile::DjiMicMini2s => {
            matches!(
                mounted.descriptor.media_name.to_ascii_lowercase().as_str(),
                "mic tx" | "wireless mic tx media"
            ) && (12_000_000_000..=20_000_000_000).contains(&mounted.descriptor.nominal_capacity)
        }
    }
}

fn sanitized_display_name(value: &str) -> String {
    let sanitized = value
        .chars()
        .filter(|character| !character.is_control() && !matches!(character, '/' | '\\'))
        .take(128)
        .collect::<String>();
    if sanitized.trim().is_empty() {
        "External Recorder".to_owned()
    } else {
        sanitized
    }
}
