pub const PRODUCT_NAME: &str = "DJI Mic Backup";

pub mod dto;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .run(tauri::generate_context!())
        .expect("failed to run DJI Mic Backup");
}

#[cfg(test)]
mod tests {
    use super::PRODUCT_NAME;

    #[test]
    fn exposes_the_product_name() {
        assert_eq!(PRODUCT_NAME, "DJI Mic Backup");
    }
}
