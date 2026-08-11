use crate::error::CoreError;

pub const DESTINATION_SETTING: &str = "destination_path";
pub const INITIAL_SETUP_SETTING: &str = "initial_setup_state";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InitialSetupMarker {
    SettingsReviewPending,
    Complete,
}

impl InitialSetupMarker {
    pub const fn encode(self) -> &'static str {
        match self {
            Self::SettingsReviewPending => r#""settings_review_pending""#,
            Self::Complete => r#""complete""#,
        }
    }

    pub fn decode(value: Option<String>) -> Result<Option<Self>, CoreError> {
        match value.as_deref() {
            None => Ok(None),
            Some(r#""settings_review_pending""#) => Ok(Some(Self::SettingsReviewPending)),
            Some(r#""complete""#) => Ok(Some(Self::Complete)),
            Some(_) => Err(CoreError::LedgerCorrupt),
        }
    }
}
