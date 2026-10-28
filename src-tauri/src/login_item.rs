//! Keeping the login item on the installed build.
//!
//! The autostart plugin writes `~/Library/LaunchAgents/<product>.plist` pointing at whichever
//! binary turned it on. A `pnpm tauri dev` session that enabled autostart left every login
//! launching `target/debug/backup-mic`, a development build missing later fixes, instead of the
//! installed app. Only the installed copy, a release build inside an `Applications` folder,
//! therefore writes the login item, and it takes the item back when it points anywhere else.
//! A build run from `target/`, a mounted disk image, or a translocated path leaves it alone.

use std::path::{Component, Path, PathBuf};

use tauri::AppHandle;
use tauri_plugin_autostart::ManagerExt as _;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LoginItem {
    Disabled,
    /// The login item launches this binary.
    Current,
    /// It pointed at another binary and now launches this one.
    Repaired,
    RepairFailed,
    /// It points at another binary, and this copy is not the installed one, so it leaves it.
    PointsElsewhere,
    Unreadable,
}

impl LoginItem {
    pub fn audit_name(self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::Current => "current",
            Self::Repaired => "repaired",
            Self::RepairFailed => "repair_failed",
            Self::PointsElsewhere => "points_elsewhere",
            Self::Unreadable => "unreadable",
        }
    }
}

/// Whether this copy may write the login item. Any copy may remove it.
pub fn may_enable_from_this_copy() -> bool {
    !cfg!(debug_assertions)
        && match (
            std::env::current_exe().and_then(|path| path.canonicalize()),
            directories::BaseDirs::new(),
        ) {
            (Ok(executable), Some(directories)) => {
                is_installed_copy(&executable, directories.home_dir())
            }
            _ => false,
        }
}

/// `/Applications/<name>.app/Contents/MacOS/<binary>`, or the same under `~/Applications`.
fn is_installed_copy(executable: &Path, home: &Path) -> bool {
    [PathBuf::from("/Applications"), home.join("Applications")]
        .iter()
        .filter_map(|root| executable.strip_prefix(root).ok())
        .any(|inside| {
            let parts = inside.components().collect::<Vec<_>>();
            matches!(
                parts.as_slice(),
                [Component::Normal(bundle), Component::Normal(contents), Component::Normal(macos), Component::Normal(_)]
                    if bundle.to_str().is_some_and(|name| name.ends_with(".app"))
                        && *contents == "Contents"
                        && *macos == "MacOS"
            )
        })
}

pub fn reconcile(app: &AppHandle) -> LoginItem {
    let manager = app.autolaunch();
    if !manager.is_enabled().unwrap_or(false) {
        return LoginItem::Disabled;
    }
    let (Some(plist), Ok(executable)) = (
        launch_agent_path(&app.package_info().name),
        std::env::current_exe().and_then(|path| path.canonicalize()),
    ) else {
        return LoginItem::Unreadable;
    };
    let Ok(contents) = std::fs::read_to_string(plist) else {
        return LoginItem::Unreadable;
    };
    if launches(&contents, &executable) {
        LoginItem::Current
    } else if !may_enable_from_this_copy() {
        LoginItem::PointsElsewhere
    } else if manager.enable().is_ok() {
        LoginItem::Repaired
    } else {
        LoginItem::RepairFailed
    }
}

fn launch_agent_path(product: &str) -> Option<PathBuf> {
    directories::BaseDirs::new().map(|directories| {
        directories
            .home_dir()
            .join("Library/LaunchAgents")
            .join(format!("{product}.plist"))
    })
}

/// The plugin writes the canonical executable path verbatim as the first program argument.
fn launches(plist: &str, executable: &Path) -> bool {
    executable
        .to_str()
        .is_some_and(|path| plist.contains(&format!("<string>{path}</string>")))
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEBUG_LOGIN_ITEM: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
  <dict>
  <key>Label</key>
  <string>Backup Mic</string>
  <key>ProgramArguments</key>
  <array><string>/Users/me/workspace/backup-mic/src-tauri/target/debug/backup-mic</string></array>
  <key>RunAtLoad</key>
  <true/>
  </dict>
</plist>"#;

    #[test]
    fn a_login_item_left_by_a_dev_build_does_not_launch_the_installed_app() {
        assert!(!launches(
            DEBUG_LOGIN_ITEM,
            Path::new("/Users/me/Applications/Backup Mic.app/Contents/MacOS/backup-mic")
        ));
        assert!(launches(
            DEBUG_LOGIN_ITEM,
            Path::new("/Users/me/workspace/backup-mic/src-tauri/target/debug/backup-mic")
        ));
        assert!(!launches(
            DEBUG_LOGIN_ITEM,
            Path::new("/Users/me/workspace/backup-mic/src-tauri/target/debug")
        ));
    }

    #[test]
    fn only_a_copy_inside_an_applications_folder_counts_as_installed() {
        let home = Path::new("/Users/me");
        for installed in [
            "/Users/me/Applications/Backup Mic.app/Contents/MacOS/backup-mic",
            "/Applications/Backup Mic.app/Contents/MacOS/backup-mic",
        ] {
            assert!(is_installed_copy(Path::new(installed), home), "{installed}");
        }
        for elsewhere in [
            "/Users/me/workspace/backup-mic/src-tauri/target/release/backup-mic",
            "/Users/me/workspace/backup-mic/src-tauri/target/release/bundle/macos/Backup Mic.app/Contents/MacOS/backup-mic",
            "/Volumes/Backup Mic/Backup Mic.app/Contents/MacOS/backup-mic",
            "/private/var/folders/x/AppTranslocation/1/d/Backup Mic.app/Contents/MacOS/backup-mic",
            "/Users/me/Applications/Backup Mic.app/Contents/Resources/backup-mic",
            "/Users/me/Applications/Old/Backup Mic.app/Contents/MacOS/backup-mic",
            "/Users/other/Applications/Backup Mic.app/Contents/MacOS/backup-mic",
        ] {
            assert!(
                !is_installed_copy(Path::new(elsewhere), home),
                "{elsewhere}"
            );
        }
    }

    #[test]
    fn the_login_item_is_found_under_the_product_name() {
        assert!(
            launch_agent_path("Backup Mic")
                .unwrap()
                .ends_with("Library/LaunchAgents/Backup Mic.plist")
        );
    }
}
