use tauri::{Manager, WindowEvent};

use crate::app_state::AppState;

pub fn handle_event(window: &tauri::Window, event: &WindowEvent) {
    match event {
        WindowEvent::CloseRequested { api, .. } => {
            api.prevent_close();
            let _ = window.hide();
        }
        WindowEvent::Focused(false) => {
            let state = window.state::<AppState>();
            if !state.should_keep_window_open() {
                let _ = window.hide();
            }
        }
        _ => {}
    }
}
