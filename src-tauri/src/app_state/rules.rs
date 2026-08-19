//! Saving, archiving, restoring, and dry-running a backup rule.

use std::collections::{BTreeSet, HashMap};

use backup_core::deletion::ProposalInvalidation;
use backup_core::error::CoreError;
use backup_core::ledger::Ledger;
use backup_core::rule::{
    BackupRule, BackupRuleDraft, DeviceConstraintProfile, FilenameProfile, compile_rule,
    validate_rule,
};
use backup_core::rule_scanner::scan_rule_once;

use crate::dto::{AppSnapshotDto, BackupRuleDto, RuleTestResultDto};

use super::AppState;
use super::devices::{rule_constraint_matches, safe_volume_label, shared_external_policy};

impl AppState {
    pub fn save_backup_rule_for_state(
        &self,
        draft: BackupRuleDraft,
        updated_at: &str,
    ) -> Result<BackupRule, CoreError> {
        let rule = self.ledger.lock().save_backup_rule(draft, updated_at)?;
        self.proposals
            .lock()
            .invalidate(ProposalInvalidation::NewScanResults);
        let (rules, source_rule_names) = public_rule_catalog(&self.ledger.lock())?;
        let mut runtime = self.runtime.lock();
        runtime
            .rule_deletions
            .retain(|_, evidence| evidence.rule.id != rule.id);
        runtime.awaiting_rule_deletion = None;
        refresh_rule_catalog(&mut runtime.snapshot, rules, &source_rule_names);
        runtime.snapshot.setting_applies_next_run = self.operation_is_active();
        runtime.snapshot.revision = runtime.snapshot.revision.saturating_add(1);
        Ok(rule)
    }

    pub fn archive_backup_rule_for_state(
        &self,
        rule_id: &str,
        archived_at: &str,
    ) -> Result<(), CoreError> {
        let rule_id = backup_core::rule::RuleId::parse(rule_id)?;
        {
            let mut ledger = self.ledger.lock();
            ledger.archive_backup_rule(&rule_id, archived_at)?;
        }
        self.proposals
            .lock()
            .invalidate(ProposalInvalidation::NewScanResults);
        let (rules, source_rule_names) = public_rule_catalog(&self.ledger.lock())?;
        let mut runtime = self.runtime.lock();
        runtime
            .rule_deletions
            .retain(|_, evidence| evidence.rule.id != rule_id);
        runtime.awaiting_rule_deletion = None;
        refresh_rule_catalog(&mut runtime.snapshot, rules, &source_rule_names);
        runtime.snapshot.setting_applies_next_run = self.operation_is_active();
        runtime.snapshot.revision = runtime.snapshot.revision.saturating_add(1);
        Ok(())
    }

    pub fn restore_dji_rule_for_state(&self, updated_at: &str) -> Result<BackupRule, CoreError> {
        let rule = self.ledger.lock().restore_dji_preset(updated_at)?;
        self.proposals
            .lock()
            .invalidate(ProposalInvalidation::NewScanResults);
        let (rules, source_rule_names) = public_rule_catalog(&self.ledger.lock())?;
        let mut runtime = self.runtime.lock();
        runtime
            .rule_deletions
            .retain(|_, evidence| evidence.rule.id != rule.id);
        runtime.awaiting_rule_deletion = None;
        refresh_rule_catalog(&mut runtime.snapshot, rules, &source_rule_names);
        runtime.snapshot.setting_applies_next_run = self.operation_is_active();
        runtime.snapshot.revision = runtime.snapshot.revision.saturating_add(1);
        Ok(rule)
    }

    pub fn test_backup_rule_for_state(
        &self,
        draft: BackupRuleDraft,
    ) -> Result<RuleTestResultDto, CoreError> {
        let draft = validate_rule(draft)?;
        let test_rule = BackupRule {
            id: draft.id.clone().unwrap_or_default(),
            name: draft.name,
            archive_directory_name: draft.archive_directory_name,
            enabled: draft.enabled,
            volume_name_glob: draft.volume_name_glob,
            required_path_globs: draft.required_path_globs,
            backup_file_globs: draft.backup_file_globs,
            session_directory_globs: draft.session_directory_globs,
            filename_prefix: draft.filename_prefix,
            filename_suffix: draft.filename_suffix,
            date_folder_layout: draft.date_folder_layout,
            filename_profile: FilenameProfile::Preserve,
            device_constraint_profile: DeviceConstraintProfile::GenericExternal,
            preset_kind: None,
            preset_revision: None,
            archive_directory_locked: false,
            archived_at: None,
            created_at: "rule-test".to_owned(),
            updated_at: "rule-test".to_owned(),
        };
        let compiled_test = compile_rule(test_rule.clone())?;
        let existing_rules = self.ledger.lock().backup_rules(false)?;
        let observed = self
            .runtime
            .lock()
            .observed
            .values()
            .cloned()
            .collect::<Vec<_>>();
        let local_offset = crate::clock::local_offset();
        let mut matched_volumes = Vec::new();
        let mut matched_file_count = 0_u64;
        let mut conflicts = BTreeSet::new();
        for mounted in observed {
            let display_name = safe_volume_label(&mounted.display_name);
            if !shared_external_policy(&mounted)
                || !compiled_test.matches_volume_name(&display_name)
            {
                continue;
            }
            let scan =
                scan_rule_once(&mounted.descriptor.mount_root, &compiled_test, local_offset)?;
            if scan.files.is_empty() {
                continue;
            }
            matched_volumes.push(display_name.clone());
            matched_file_count = matched_file_count
                .checked_add(u64::try_from(scan.files.len()).map_err(|_| CoreError::InvalidRule)?)
                .ok_or(CoreError::InvalidRule)?;
            for rule in existing_rules.iter().filter(|rule| {
                rule.enabled && rule.archived_at.is_none() && rule.id != test_rule.id
            }) {
                let compiled = compile_rule(rule.clone())?;
                if compiled.matches_volume_name(&display_name)
                    && rule_constraint_matches(&mounted, rule.device_constraint_profile)
                    && !scan_rule_once(&mounted.descriptor.mount_root, &compiled, local_offset)?
                        .files
                        .is_empty()
                {
                    conflicts.insert(rule.name.clone());
                }
            }
        }
        matched_volumes.sort();
        matched_volumes.dedup();
        Ok(RuleTestResultDto {
            matched_volumes,
            matched_file_count,
            conflict_rule_names: conflicts.into_iter().collect(),
        })
    }
}

pub(super) fn public_rule_catalog(
    ledger: &Ledger,
) -> Result<(Vec<BackupRule>, HashMap<String, String>), CoreError> {
    let rules = ledger.backup_rules(true)?;
    let mut source_rule_names = HashMap::new();
    for rule in &rules {
        for source in ledger.sources_for_rule(&rule.id)? {
            source_rule_names.insert(source.id.as_str().to_owned(), rule.name.clone());
        }
    }
    Ok((rules, source_rule_names))
}

pub(super) fn refresh_rule_catalog(
    snapshot: &mut AppSnapshotDto,
    rules: Vec<BackupRule>,
    source_rule_names: &HashMap<String, String>,
) {
    snapshot.backup_rules = rules.iter().map(BackupRuleDto::from).collect();
    for source in &mut snapshot.sources {
        if let Some(rule_name) = source_rule_names.get(&source.source_id) {
            source.rule_name.clone_from(rule_name);
        }
    }
}
