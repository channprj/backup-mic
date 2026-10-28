//! Where whisper models live on disk, and fetching a catalog model into place.
//!
//! A catalog model only ever appears under its final name after its size and SHA-256 matched
//! the pinned catalog entry. Until then the bytes sit in a `.part` file beside it, which a later
//! download resumes instead of starting a multi-gigabyte transfer over.

use std::{
    fs::{self, File},
    io::{self, Read as _},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

use backup_core::{
    error::CoreError,
    hash::hash_file,
    transcription::{WhisperModel, is_safe_absolute_path},
};

/// The first four bytes of every ggml file whisper.cpp loads: `0x67676d6c` stored little-endian.
const GGML_MAGIC: [u8; 4] = *b"lmgg";
const PROGRESS_INTERVAL: Duration = Duration::from_millis(250);

/// Why a model download stopped, in the terms the settings window explains.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DownloadError {
    Cancelled,
    Network,
    Checksum,
    Storage,
}

impl DownloadError {
    pub const fn message_code(self) -> &'static str {
        match self {
            Self::Cancelled => "model_download_cancelled",
            Self::Network => "model_download_failed",
            Self::Checksum => "model_checksum_mismatch",
            Self::Storage => "model_storage_failed",
        }
    }

    pub const fn retryable(self) -> bool {
        !matches!(self, Self::Storage)
    }
}

/// Moves the bytes at a URL into a partial file.
///
/// A trait so the install logic — resume, verification, the atomic rename — can be tested
/// without the network.
pub trait ModelFetcher: Send + Sync {
    /// Appends the bytes at `url` to `partial`, resuming from its current length, and reports
    /// the partial file's size as it grows.
    fn fetch(
        &self,
        url: &str,
        partial: &Path,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(u64),
    ) -> Result<(), DownloadError>;
}

/// Downloads with the system `curl`, restricted to HTTPS for the request and every redirect.
#[derive(Debug, Clone, Copy, Default)]
pub struct CurlFetcher;

impl ModelFetcher for CurlFetcher {
    fn fetch(
        &self,
        url: &str,
        partial: &Path,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(u64),
    ) -> Result<(), DownloadError> {
        let mut child = Command::new("/usr/bin/curl")
            .args(curl_arguments(url, partial))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| DownloadError::Network)?;
        loop {
            if cancel.load(Ordering::SeqCst) {
                let _ = child.kill();
                let _ = child.wait();
                return Err(DownloadError::Cancelled);
            }
            match child.try_wait() {
                Ok(Some(status)) => {
                    progress(regular_file_len(partial).unwrap_or(0));
                    return if status.success() {
                        Ok(())
                    } else {
                        Err(DownloadError::Network)
                    };
                }
                Ok(None) => {
                    progress(regular_file_len(partial).unwrap_or(0));
                    std::thread::sleep(PROGRESS_INTERVAL);
                }
                Err(_) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(DownloadError::Network);
                }
            }
        }
    }
}

/// The exact `curl` invocation, kept separate so a test can pin its safety flags.
pub fn curl_arguments(url: &str, partial: &Path) -> Vec<std::ffi::OsString> {
    let mut arguments = [
        "--fail",
        "--location",
        "--silent",
        "--proto",
        "=https",
        "--proto-redir",
        "=https",
        "--tlsv1.2",
        "--connect-timeout",
        "30",
        "--retry",
        "3",
        "--speed-limit",
        "1024",
        "--speed-time",
        "60",
        "--continue-at",
        "-",
        "--output",
    ]
    .map(std::ffi::OsString::from)
    .to_vec();
    arguments.push(partial.as_os_str().to_owned());
    arguments.push(url.into());
    arguments
}

pub fn catalog_model_path(models_directory: &Path, model: &WhisperModel) -> PathBuf {
    models_directory.join(model.file_name())
}

fn partial_path(final_path: &Path) -> PathBuf {
    let mut name = final_path.file_name().unwrap_or_default().to_owned();
    name.push(".part");
    final_path.with_file_name(name)
}

/// Whether a catalog model is present at its exact catalog size.
///
/// Size only: the digest was checked before the file got its final name, and re-hashing
/// gigabytes every time the settings window opens would make the window unusable.
pub fn catalog_model_installed(models_directory: &Path, model: &WhisperModel) -> bool {
    regular_file_len(&catalog_model_path(models_directory, model)) == Some(model.byte_count)
}

/// The length of `path` when it is a regular file and not a link to one.
pub fn regular_file_len(path: &Path) -> Option<u64> {
    let metadata = fs::symlink_metadata(path).ok()?;
    metadata.file_type().is_file().then_some(metadata.len())
}

/// Refuses a file whisper.cpp could not load as a ggml model.
///
/// A picked file is the user's own, so it is not checked against the catalog — but a file that
/// does not even start like a ggml model would only fail later, in the middle of a run.
pub fn validate_custom_model(path: &Path) -> Result<(), CoreError> {
    if !is_safe_absolute_path(path) || regular_file_len(path).is_none_or(|len| len <= 4) {
        return Err(CoreError::InvalidRequest);
    }
    let mut magic = [0_u8; 4];
    File::open(path)
        .and_then(|mut file| file.read_exact(&mut magic))
        .map_err(CoreError::CopyFailed)?;
    if magic == GGML_MAGIC {
        Ok(())
    } else {
        Err(CoreError::InvalidRequest)
    }
}

/// Downloads `model` into `models_directory` and returns its final path.
///
/// Resumes a partial file left by an earlier attempt, refuses to write through a link, and
/// deletes a partial file whose bytes turned out wrong so the next attempt starts clean.
pub fn install_catalog_model(
    models_directory: &Path,
    model: &WhisperModel,
    fetcher: &dyn ModelFetcher,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(u64),
) -> Result<PathBuf, DownloadError> {
    fs::create_dir_all(models_directory).map_err(|_| DownloadError::Storage)?;
    let final_path = catalog_model_path(models_directory, model);
    if catalog_model_installed(models_directory, model) {
        return Ok(final_path);
    }
    let partial = partial_path(&final_path);
    for path in [&final_path, &partial] {
        match fs::symlink_metadata(path) {
            Ok(metadata) if !metadata.file_type().is_file() => return Err(DownloadError::Storage),
            Ok(_) | Err(_) => {}
        }
    }
    if regular_file_len(&partial).is_some_and(|len| len > model.byte_count) {
        remove_if_present(&partial)?;
    }
    if regular_file_len(&partial) != Some(model.byte_count) {
        fetcher.fetch(&model.download_url(), &partial, cancel, progress)?;
    }
    progress(regular_file_len(&partial).unwrap_or(0));
    let digest = hash_file(&partial).map_err(|_| DownloadError::Storage)?;
    if digest.size != model.byte_count || digest.sha256 != model.sha256 {
        remove_if_present(&partial)?;
        return Err(DownloadError::Checksum);
    }
    if cancel.load(Ordering::SeqCst) {
        return Err(DownloadError::Cancelled);
    }
    fs::rename(&partial, &final_path).map_err(|_| DownloadError::Storage)?;
    Ok(final_path)
}

fn remove_if_present(path: &Path) -> Result<(), DownloadError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(DownloadError::Storage),
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::Cell, io::Write as _, sync::Mutex};

    use backup_core::hash::hash_reader;

    use super::*;

    /// Serves a fixed byte string, optionally stopping early, and records each request.
    struct FakeFetcher {
        body: Vec<u8>,
        stop_after: Option<usize>,
        requests: Mutex<Vec<u64>>,
    }

    impl FakeFetcher {
        fn new(body: &[u8]) -> Self {
            Self {
                body: body.to_vec(),
                stop_after: None,
                requests: Mutex::new(Vec::new()),
            }
        }
    }

    impl ModelFetcher for FakeFetcher {
        fn fetch(
            &self,
            _url: &str,
            partial: &Path,
            _cancel: &AtomicBool,
            progress: &mut dyn FnMut(u64),
        ) -> Result<(), DownloadError> {
            let resumed_from = regular_file_len(partial).unwrap_or(0);
            self.requests.lock().unwrap().push(resumed_from);
            let end = self.stop_after.unwrap_or(self.body.len());
            let mut file = fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(partial)
                .unwrap();
            file.write_all(&self.body[resumed_from as usize..end])
                .unwrap();
            progress(end as u64);
            if end < self.body.len() {
                Err(DownloadError::Network)
            } else {
                Ok(())
            }
        }
    }

    fn model_for(body: &[u8]) -> WhisperModel {
        let digest = hash_reader(body, |_| {}).unwrap();
        WhisperModel {
            id: "fixture",
            label: "Fixture",
            byte_count: digest.size,
            sha256: Box::leak(digest.sha256.into_boxed_str()),
            english_only: false,
            quantized: false,
        }
    }

    #[test]
    fn a_verified_download_gets_its_final_name_and_no_partial_remains() {
        let directory = tempfile::tempdir().unwrap();
        let body = b"lmgg model bytes";
        let model = model_for(body);
        let fetcher = FakeFetcher::new(body);
        let reported = Cell::new(0);

        let installed = install_catalog_model(
            directory.path(),
            &model,
            &fetcher,
            &AtomicBool::new(false),
            &mut |bytes| reported.set(bytes),
        )
        .unwrap();

        assert_eq!(installed, directory.path().join("ggml-fixture.bin"));
        assert_eq!(fs::read(&installed).unwrap(), body);
        assert!(!directory.path().join("ggml-fixture.bin.part").exists());
        assert!(catalog_model_installed(directory.path(), &model));
        assert_eq!(reported.get(), body.len() as u64);
    }

    #[test]
    fn an_interrupted_download_resumes_from_the_partial_file() {
        let directory = tempfile::tempdir().unwrap();
        let body = b"lmgg resumable model bytes";
        let model = model_for(body);
        let mut fetcher = FakeFetcher::new(body);
        fetcher.stop_after = Some(10);

        let first = install_catalog_model(
            directory.path(),
            &model,
            &fetcher,
            &AtomicBool::new(false),
            &mut |_| {},
        );
        assert_eq!(first.unwrap_err(), DownloadError::Network);
        assert!(!catalog_model_installed(directory.path(), &model));

        fetcher.stop_after = None;
        install_catalog_model(
            directory.path(),
            &model,
            &fetcher,
            &AtomicBool::new(false),
            &mut |_| {},
        )
        .unwrap();
        assert_eq!(*fetcher.requests.lock().unwrap(), [0, 10]);
        assert!(catalog_model_installed(directory.path(), &model));
    }

    #[test]
    fn wrong_bytes_are_discarded_and_never_installed() {
        let directory = tempfile::tempdir().unwrap();
        let model = model_for(b"lmgg the real model");
        let fetcher = FakeFetcher::new(b"lmgg a tampered one");

        let result = install_catalog_model(
            directory.path(),
            &model,
            &fetcher,
            &AtomicBool::new(false),
            &mut |_| {},
        );

        assert_eq!(result.unwrap_err(), DownloadError::Checksum);
        assert!(!directory.path().join("ggml-fixture.bin").exists());
        assert!(!directory.path().join("ggml-fixture.bin.part").exists());
    }

    #[test]
    fn an_installed_model_is_not_fetched_again() {
        let directory = tempfile::tempdir().unwrap();
        let body = b"lmgg already here";
        let model = model_for(body);
        fs::write(directory.path().join("ggml-fixture.bin"), body).unwrap();
        let fetcher = FakeFetcher::new(body);

        install_catalog_model(
            directory.path(),
            &model,
            &fetcher,
            &AtomicBool::new(false),
            &mut |_| {},
        )
        .unwrap();
        assert!(fetcher.requests.lock().unwrap().is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn a_link_in_place_of_the_partial_file_is_refused() {
        let directory = tempfile::tempdir().unwrap();
        let outside = directory.path().join("outside");
        fs::write(&outside, b"keep").unwrap();
        std::os::unix::fs::symlink(&outside, directory.path().join("ggml-fixture.bin.part"))
            .unwrap();
        let body = b"lmgg model";
        let model = model_for(body);

        let result = install_catalog_model(
            directory.path(),
            &model,
            &FakeFetcher::new(body),
            &AtomicBool::new(false),
            &mut |_| {},
        );

        assert_eq!(result.unwrap_err(), DownloadError::Storage);
        assert_eq!(fs::read(&outside).unwrap(), b"keep");
    }

    #[test]
    fn a_custom_model_must_start_like_a_ggml_file() {
        let directory = tempfile::tempdir().unwrap();
        let ggml = directory.path().join("ggml-custom.bin");
        fs::write(&ggml, b"lmgg custom").unwrap();
        assert!(validate_custom_model(&ggml).is_ok());

        let other = directory.path().join("notes.bin");
        fs::write(&other, b"not a model").unwrap();
        assert!(validate_custom_model(&other).is_err());
        assert!(validate_custom_model(Path::new("relative/ggml.bin")).is_err());
        assert!(validate_custom_model(&directory.path().join("missing.bin")).is_err());
    }

    #[test]
    fn curl_only_speaks_https_and_resumes_into_the_partial_file() {
        let arguments = curl_arguments(
            "https://huggingface.co/model.bin",
            Path::new("/models/ggml-medium.bin.part"),
        );
        let arguments = arguments
            .iter()
            .map(|argument| argument.to_str().unwrap())
            .collect::<Vec<_>>();
        for pair in [
            ["--proto", "=https"],
            ["--proto-redir", "=https"],
            ["--continue-at", "-"],
            ["--output", "/models/ggml-medium.bin.part"],
        ] {
            assert!(
                arguments.windows(2).any(|window| window == pair),
                "{pair:?} missing"
            );
        }
        assert!(arguments.contains(&"--fail"));
        assert_eq!(arguments.last(), Some(&"https://huggingface.co/model.bin"));
    }
}
