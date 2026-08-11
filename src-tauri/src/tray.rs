use tauri::{
    App, Manager,
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};
use tauri_plugin_positioner::{Position, WindowExt};

use crate::PRODUCT_NAME;

const STATUS_ICON: tauri::image::Image<'static> =
    tauri::include_image!("./icons/tray-template.png");

fn status_icon() -> tauri::image::Image<'static> {
    STATUS_ICON.clone()
}

pub fn setup(app: &App) -> tauri::Result<()> {
    TrayIconBuilder::with_id("status")
        .tooltip(PRODUCT_NAME)
        .show_menu_on_left_click(false)
        .icon(status_icon())
        .icon_as_template(true)
        .on_tray_icon_event(|tray, event| {
            tauri_plugin_positioner::on_tray_event(tray.app_handle(), &event);
            if matches!(
                event,
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                }
            ) {
                toggle_popover(tray.app_handle());
            }
        })
        .build(app)?;
    Ok(())
}

fn toggle_popover(app: &tauri::AppHandle) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    if window.is_visible().unwrap_or(false) {
        let _ = window.hide();
        return;
    }
    show_popover(app);
}

pub(crate) fn show_popover(app: &tauri::AppHandle) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    let _ = window.move_window_constrained(Position::TrayCenter);
    let _ = window.show();
    let _ = window.set_focus();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alpha(image: &tauri::image::Image<'_>, x: u32, y: u32) -> u8 {
        image.rgba()[((y * image.width() + x) * 4 + 3) as usize]
    }

    #[test]
    fn dedicated_template_icon_preserves_the_app_face_as_transparent_cutouts() {
        let icon = status_icon();

        assert_eq!((icon.width(), icon.height()), (36, 36));
        assert_eq!(alpha(&icon, 0, 0), 0);
        assert!(alpha(&icon, 18, 6) > 240, "the face body must be opaque");
        assert!(alpha(&icon, 13, 14) < 16, "the left eye must be a cutout");
        assert!(alpha(&icon, 22, 14) < 16, "the right eye must be a cutout");
        assert!(alpha(&icon, 18, 22) < 16, "the mouth must be a cutout");
    }
}
