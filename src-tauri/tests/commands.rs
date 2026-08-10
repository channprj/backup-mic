use backup_mic_lib::commands::REGISTERED_COMMANDS;

#[test]
fn command_surface_is_exact_and_has_no_arbitrary_settings_or_path_ipc() {
    assert_eq!(
        REGISTERED_COMMANDS,
        [
            "get_app_snapshot",
            "backup_now",
            "choose_destination",
            "save_backup_rule",
            "archive_backup_rule",
            "restore_dji_rule",
            "test_backup_rule",
            "prepare_trash",
            "confirm_trash",
            "set_autostart",
            "open_destination",
            "quit_app",
            "show_settings",
            "set_automatic_backup",
            "set_m4a_conversion",
            "set_automatic_trash",
            "open_logs",
        ]
    );
    let source = include_str!("../src/commands.rs");
    for forbidden in [
        "set_setting(key",
        "setting_key",
        "trash_destination",
        "source_uuid",
        "source_hash",
        "process_command",
    ] {
        assert!(!source.contains(forbidden));
    }
    for forbidden_command in ["pair_devices", "set_setting", "scan_path", "open_path"] {
        assert!(!REGISTERED_COMMANDS.contains(&forbidden_command));
    }
}
