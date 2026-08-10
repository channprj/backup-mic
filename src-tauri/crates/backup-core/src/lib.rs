pub const PRODUCT_NAME: &str = "DJI Mic Backup";

pub mod additional_file;
pub mod artifact;
pub mod audit_log;
pub mod backup;
pub mod batch;
pub mod clock;
pub mod deletion;
pub mod destination;
pub mod device;
pub mod error;
pub mod events;
pub mod filesystem;
pub mod hash;
pub mod layout;
pub mod ledger;
pub mod preferences;
pub mod recording;
pub mod recovery;
pub mod rule;
pub mod scanner;
pub mod state;

#[cfg(test)]
mod tests {
    use super::PRODUCT_NAME;

    #[test]
    fn exposes_the_product_name() {
        assert_eq!(PRODUCT_NAME, "DJI Mic Backup");
    }
}
