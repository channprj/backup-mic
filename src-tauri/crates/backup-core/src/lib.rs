pub const PRODUCT_NAME: &str = "DJI Mic Backup";

pub mod error;
pub mod events;
pub mod state;

#[cfg(test)]
mod tests {
    use super::PRODUCT_NAME;

    #[test]
    fn exposes_the_product_name() {
        assert_eq!(PRODUCT_NAME, "DJI Mic Backup");
    }
}
