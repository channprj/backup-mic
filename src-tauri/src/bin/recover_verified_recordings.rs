use std::path::{Path, PathBuf};

use backup_core::{error::CoreError, ledger::Ledger};
use backup_mic_lib::{
    platform::macos::audio::AppleAudioTools,
    recovery_tool::{RecoveryMode, recover_verified_recordings},
};

struct Arguments {
    ledger: PathBuf,
    destination: PathBuf,
    source_root: PathBuf,
    recording_ids: Vec<String>,
    mode: RecoveryMode,
}

fn main() {
    match run() {
        Ok(()) => {}
        Err(error) => {
            eprintln!("recovery_failed={}", error.diagnostic_code());
            std::process::exit(1);
        }
    }
}

fn run() -> Result<(), CoreError> {
    let arguments = parse_arguments(std::env::args().skip(1))?;
    validate_existing_ledger(&arguments.ledger)?;
    let mut ledger = Ledger::open(&arguments.ledger)?;
    let summary = recover_verified_recordings(
        &mut ledger,
        &arguments.destination,
        &arguments.source_root,
        &arguments.recording_ids,
        arguments.mode,
        &AppleAudioTools,
    )?;
    let output = serde_json::to_string(&summary).map_err(|_| CoreError::InvalidRequest)?;
    println!("{output}");
    if summary.failed > 0 {
        return Err(CoreError::SourceChanged);
    }
    Ok(())
}

fn parse_arguments(arguments: impl IntoIterator<Item = String>) -> Result<Arguments, CoreError> {
    let mut ledger = None;
    let mut destination = None;
    let mut source_root = None;
    let mut recording_ids = Vec::new();
    let mut mode = RecoveryMode::DryRun;
    let mut arguments = arguments.into_iter();
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--ledger" => ledger = arguments.next().map(PathBuf::from),
            "--destination" => destination = arguments.next().map(PathBuf::from),
            "--source-root" => source_root = arguments.next().map(PathBuf::from),
            "--recording-id" => {
                recording_ids.push(arguments.next().ok_or(CoreError::InvalidRequest)?)
            }
            "--apply" if mode == RecoveryMode::DryRun => mode = RecoveryMode::Apply,
            "--dry-run" if mode == RecoveryMode::DryRun => {}
            _ => return Err(CoreError::InvalidRequest),
        }
    }
    Ok(Arguments {
        ledger: ledger.ok_or(CoreError::InvalidRequest)?,
        destination: destination.ok_or(CoreError::InvalidRequest)?,
        source_root: source_root.ok_or(CoreError::InvalidRequest)?,
        recording_ids,
        mode,
    })
}

fn validate_existing_ledger(path: &Path) -> Result<(), CoreError> {
    if !path.is_absolute() {
        return Err(CoreError::InvalidRequest);
    }
    let metadata = std::fs::symlink_metadata(path).map_err(CoreError::LedgerIo)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() == 0 {
        return Err(CoreError::InvalidRequest);
    }
    Ok(())
}
