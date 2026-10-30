//! Read-only discovery of compatible models installed by WakeNote.
//!
//! WakeNote uses `whisper-*.bin` for built-ins and `ggml-*.bin` for imported models.
//! Only exact Backup Mic catalog bytes are eligible. Hashing runs on the queue/download
//! threads; snapshots only inspect metadata and previously verified results.

use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    time::SystemTime,
};

use backup_core::{
    hash::hash_file,
    transcription::{WhisperModel, is_safe_absolute_path},
};
use parking_lot::Mutex;
use serde::Deserialize;

const DEFAULT_MODELS: &str = "Library/Application Support/WakeNote/models";
const SETTINGS: &str = "Library/Application Support/com.chann.wakenote/settings.json";

pub(super) struct WakeNoteModels {
    home: Option<PathBuf>,
    cache: Mutex<HashMap<(PathBuf, &'static str), Verification>>,
    refresh_lock: Mutex<()>,
}

#[derive(PartialEq, Eq)]
struct FileStamp {
    size: u64,
    modified: SystemTime,
    #[cfg(unix)]
    identity: (u64, u64, i64, i64),
}

struct Verification {
    stamp: FileStamp,
    valid: bool,
}

#[derive(Deserialize)]
struct WakeNoteSettings {
    model_directory: Option<String>,
}

impl FileStamp {
    fn read(path: &Path) -> Option<Self> {
        let metadata = fs::symlink_metadata(path).ok()?;
        if !metadata.is_file() || !is_safe_absolute_path(path) {
            return None;
        }
        #[cfg(unix)]
        use std::os::unix::fs::MetadataExt as _;
        Some(Self {
            size: metadata.len(),
            modified: metadata.modified().ok()?,
            #[cfg(unix)]
            identity: (
                metadata.dev(),
                metadata.ino(),
                metadata.ctime(),
                metadata.ctime_nsec(),
            ),
        })
    }
}

impl WakeNoteModels {
    pub(super) fn new(home: Option<&Path>) -> Self {
        Self {
            home: home.map(Path::to_path_buf),
            cache: Mutex::default(),
            refresh_lock: Mutex::default(),
        }
    }

    fn directories(&self) -> Vec<PathBuf> {
        let Some(home) = &self.home else {
            return Vec::new();
        };
        let default = home.join(DEFAULT_MODELS);
        let configured = (|| {
            let path = home.join(SETTINGS);
            // Read only the one setting needed; do not load credentials or other app state.
            if FileStamp::read(&path)?.size > 1024 * 1024 {
                return None;
            }
            let settings: WakeNoteSettings = serde_json::from_slice(&fs::read(path).ok()?).ok()?;
            let chosen = settings.model_directory?;
            let path = chosen
                .strip_prefix("~/")
                .map_or_else(|| PathBuf::from(&chosen), |relative| home.join(relative));
            is_safe_absolute_path(&path).then_some(path)
        })();
        let mut directories = Vec::new();
        if let Some(path) = configured.filter(|path| path != &default) {
            directories.push(path);
        }
        directories.push(default);
        directories
    }

    fn candidates(&self, model: &WhisperModel) -> Vec<PathBuf> {
        let alias = match model.id {
            "small" => Some("whisper-small.bin"),
            "medium" => Some("whisper-medium.bin"),
            "large-v3-turbo" => Some("whisper-turbo.bin"),
            "large-v3-q5_0" => Some("whisper-large.bin"),
            _ => None,
        };
        let mut paths = Vec::new();
        for directory in self.directories() {
            if let Some(alias) = alias {
                paths.push(directory.join(alias));
            }
            paths.push(directory.join(model.file_name()));
        }
        paths
    }

    /// Blocking: verify newly discovered or changed files without holding the snapshot lock.
    /// Cache failures too, so a corrupt multi-gigabyte file is not hashed at every wake.
    pub(super) fn refresh(&self, models: &[WhisperModel]) {
        let _refresh = self.refresh_lock.lock();
        for model in models {
            for path in self.candidates(model) {
                let Some(stamp) =
                    FileStamp::read(&path).filter(|stamp| stamp.size == model.byte_count)
                else {
                    continue;
                };
                let key = (path.clone(), model.sha256);
                if self
                    .cache
                    .lock()
                    .get(&key)
                    .is_some_and(|entry| entry.stamp == stamp)
                {
                    continue;
                }
                let valid = hash_file(&path).is_ok_and(|digest| {
                    digest.size == model.byte_count && digest.sha256 == model.sha256
                });
                // A file replaced or written while hashing must be verified again next time.
                if FileStamp::read(&path).as_ref() == Some(&stamp) {
                    self.cache.lock().insert(key, Verification { stamp, valid });
                }
            }
        }
    }

    /// Cheap enough for snapshots: never hashes, and never trusts stale verification.
    pub(super) fn verified_path(&self, model: &WhisperModel) -> Option<PathBuf> {
        self.candidates(model).into_iter().find(|path| {
            let Some(stamp) = FileStamp::read(path) else {
                return false;
            };
            self.cache
                .lock()
                .get(&(path.clone(), model.sha256))
                .is_some_and(|entry| entry.valid && entry.stamp == stamp)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use backup_core::hash::hash_reader;
    use std::fs;

    const BODY: &[u8] = b"lmgg verified whisper fixture";

    fn model(id: &'static str) -> WhisperModel {
        let digest = hash_reader(BODY, |_| {}).unwrap();
        WhisperModel {
            id,
            label: "Fixture",
            byte_count: BODY.len() as u64,
            sha256: Box::leak(digest.sha256.into_boxed_str()),
            english_only: false,
            quantized: false,
        }
    }

    fn install(home: &Path, name: &str, body: &[u8]) -> PathBuf {
        let dir = home.join("Library/Application Support/WakeNote/models");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        fs::write(&path, body).unwrap();
        path
    }

    fn settings(home: &Path, json: &str) {
        let dir = home.join("Library/Application Support/com.chann.wakenote");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("settings.json"), json).unwrap();
    }

    #[test]
    fn discovers_wakenote_aliases_without_creating_or_copying_files() {
        for (id, name) in [
            ("small", "whisper-small.bin"),
            ("medium", "whisper-medium.bin"),
            ("large-v3-turbo", "whisper-turbo.bin"),
            ("large-v3-q5_0", "whisper-large.bin"),
            ("tiny.en", "ggml-tiny.en.bin"),
            ("medium-q5_0", "ggml-medium-q5_0.bin"),
        ] {
            let home = tempfile::tempdir().unwrap();
            let path = install(home.path(), name, BODY);
            let models = WakeNoteModels::new(Some(home.path()));
            let model = model(id);
            assert_eq!(
                models.verified_path(&model),
                None,
                "unverified bytes are unavailable"
            );
            models.refresh(&[model]);
            assert_eq!(models.verified_path(&model), Some(path.clone()), "{id}");
            assert_eq!(fs::read(&path).unwrap(), BODY);
            assert_eq!(fs::read_dir(path.parent().unwrap()).unwrap().count(), 1);
        }
    }

    #[test]
    fn reads_wakenote_custom_directory_and_tracks_settings_changes() {
        let home = tempfile::tempdir().unwrap();
        let default = install(home.path(), "whisper-medium.bin", BODY);
        let custom = home.path().join("shared models");
        fs::create_dir(&custom).unwrap();
        let path = custom.join("ggml-medium.bin");
        fs::write(&path, BODY).unwrap();
        settings(
            home.path(),
            r#"{"model_directory":"~/shared models","unrelated_setting":true}"#,
        );
        let models = WakeNoteModels::new(Some(home.path()));
        let model = model("medium");
        models.refresh(&[model]);
        assert_eq!(models.verified_path(&model), Some(path));
        settings(home.path(), "{}");
        models.refresh(&[model]);
        assert_eq!(models.verified_path(&model), Some(default));
    }

    #[test]
    fn an_absolute_custom_directory_is_supported() {
        let home = tempfile::tempdir().unwrap();
        let custom = tempfile::tempdir().unwrap();
        let path = custom.path().join("whisper-small.bin");
        fs::write(&path, BODY).unwrap();
        settings(
            home.path(),
            &serde_json::json!({"model_directory":custom.path()}).to_string(),
        );
        let models = WakeNoteModels::new(Some(home.path()));
        let model = model("small");
        models.refresh(&[model]);
        assert_eq!(models.verified_path(&model), Some(path));
    }

    #[test]
    fn missing_malformed_and_unsafe_settings_fall_back_to_the_default_folder() {
        let home = tempfile::tempdir().unwrap();
        let path = install(home.path(), "whisper-small.bin", BODY);
        let models = WakeNoteModels::new(Some(home.path()));
        let model = model("small");
        for json in [
            "{broken",
            "{}",
            r#"{"model_directory":"relative"}"#,
            r#"{"model_directory":"~/../outside"}"#,
            r#"{"model_directory":""}"#,
        ] {
            settings(home.path(), json);
            models.refresh(&[model]);
            assert_eq!(models.verified_path(&model), Some(path.clone()));
        }
    }

    #[test]
    fn wrong_size_digest_partial_and_unrelated_models_are_never_reused() {
        let home = tempfile::tempdir().unwrap();
        let path = install(home.path(), "whisper-medium.bin", b"lmgg partial");
        let models = WakeNoteModels::new(Some(home.path()));
        let model = model("medium");
        models.refresh(&[model]);
        assert_eq!(models.verified_path(&model), None);
        fs::write(&path, vec![b'x'; BODY.len()]).unwrap();
        models.refresh(&[model]);
        assert_eq!(
            models.verified_path(&model),
            None,
            "same size is not integrity"
        );
        fs::remove_file(&path).unwrap();
        for name in [
            "whisper-medium.download",
            "whisper-medium.bin.part",
            "qwen3-asr.bin",
            "whisper-large.bin",
        ] {
            install(home.path(), name, BODY);
        }
        models.refresh(&[model]);
        assert_eq!(models.verified_path(&model), None);
    }

    #[test]
    fn changed_or_deleted_files_immediately_lose_ready_status_and_can_be_rediscovered() {
        let home = tempfile::tempdir().unwrap();
        let path = install(home.path(), "whisper-medium.bin", BODY);
        let models = WakeNoteModels::new(Some(home.path()));
        let model = model("medium");
        models.refresh(&[model]);
        assert_eq!(models.verified_path(&model), Some(path.clone()));
        fs::write(&path, vec![b'x'; BODY.len()]).unwrap();
        assert_eq!(models.verified_path(&model), None);
        models.refresh(&[model]);
        assert_eq!(models.verified_path(&model), None);
        fs::write(&path, BODY).unwrap();
        models.refresh(&[model]);
        assert_eq!(models.verified_path(&model), Some(path.clone()));
        fs::remove_file(&path).unwrap();
        assert_eq!(models.verified_path(&model), None);
    }

    #[cfg(unix)]
    #[test]
    fn same_size_replacement_with_preserved_mtime_is_not_trusted() {
        let home = tempfile::tempdir().unwrap();
        let path = install(home.path(), "whisper-small.bin", BODY);
        let models = WakeNoteModels::new(Some(home.path()));
        let model = model("small");
        models.refresh(&[model]);
        assert_eq!(models.verified_path(&model), Some(path.clone()));
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        let replacement = home.path().join("replacement.bin");
        fs::write(&replacement, vec![b'x'; BODY.len()]).unwrap();
        fs::File::options()
            .write(true)
            .open(&replacement)
            .unwrap()
            .set_times(fs::FileTimes::new().set_modified(modified))
            .unwrap();
        fs::rename(&replacement, &path).unwrap();
        assert_eq!(models.verified_path(&model), None);
        models.refresh(&[model]);
        assert_eq!(models.verified_path(&model), None);
    }

    #[cfg(unix)]
    #[test]
    fn links_are_not_accepted_as_installed_models() {
        let home = tempfile::tempdir().unwrap();
        let real = home.path().join("real.bin");
        fs::write(&real, BODY).unwrap();
        let path = install(home.path(), "whisper-small.bin", BODY);
        fs::remove_file(&path).unwrap();
        std::os::unix::fs::symlink(&real, path).unwrap();
        let models = WakeNoteModels::new(Some(home.path()));
        let model = model("small");
        models.refresh(&[model]);
        assert_eq!(models.verified_path(&model), None);
        assert_eq!(fs::read(&real).unwrap(), BODY);
    }

    #[test]
    fn unavailable_home_or_missing_wakenote_is_a_no_op() {
        let models = WakeNoteModels::new(None);
        models.refresh(&[model("small")]);
        assert_eq!(models.verified_path(&model("small")), None);
        let home = tempfile::tempdir().unwrap();
        WakeNoteModels::new(Some(home.path())).refresh(&[model("small")]);
        assert_eq!(fs::read_dir(home.path()).unwrap().count(), 0);
    }
}
