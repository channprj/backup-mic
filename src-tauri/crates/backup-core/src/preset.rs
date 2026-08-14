use crate::rule::{BackupRuleDraft, DateFolderLayout, RuleId};

pub const DJI_PRESET_ID: &str = "6d784c99-8b0e-4a32-a0a2-d7730f68cf28";
pub const DJI_PRESET_KIND: &str = "dji_mic_mini_2s";
pub const DJI_PRESET_REVISION: u32 = 2;

pub fn dji_mic_mini_2s_preset() -> BackupRuleDraft {
    BackupRuleDraft {
        id: Some(RuleId::parse(DJI_PRESET_ID).expect("the DJI preset ID is a valid UUID")),
        name: "DJI Mic Mini 2S".to_owned(),
        archive_directory_name: "DJI Mic Mini 2S".to_owned(),
        enabled: true,
        volume_name_glob: "*".to_owned(),
        required_path_globs: Vec::new(),
        backup_file_globs: vec!["*.WAV".to_owned(), "TX_MIC*/*.WAV".to_owned()],
        session_directory_globs: vec!["TX_MIC*".to_owned()],
        filename_prefix: String::new(),
        filename_suffix: String::new(),
        date_folder_layout: DateFolderLayout::YearMonthDay,
    }
}
