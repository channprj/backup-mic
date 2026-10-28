//! Writing a finished transcript in each selected format.
//!
//! Transcripts mirror the backup layout under the output folder, so a recording at
//! `DJI/2026/10/28/TX01.m4a` gets `DJI/2026/10/28/TX01.txt`. Every file is written to a
//! temporary name first and never replaces one that already exists: a transcript the user has
//! edited by hand is theirs.

use std::{
    fs,
    io::Write as _,
    path::{Path, PathBuf},
};

use backup_core::{
    error::CoreError, filesystem::is_safe_relative_path, transcription::TranscriptFormat,
};
use serde::Serialize;

use super::engine::Transcript;

/// Where `format` of the recording at `relative` goes, or `None` for a path that could leave
/// the output folder.
pub fn transcript_path(
    output_root: &Path,
    relative: &Path,
    format: TranscriptFormat,
) -> Option<PathBuf> {
    is_safe_relative_path(relative).then(|| {
        output_root
            .join(relative)
            .with_extension(format.extension())
    })
}

/// The selected formats this recording does not have a transcript file for yet.
pub fn missing_formats(
    output_root: &Path,
    relative: &Path,
    formats: &[TranscriptFormat],
) -> Vec<TranscriptFormat> {
    formats
        .iter()
        .copied()
        .filter(|format| {
            transcript_path(output_root, relative, *format)
                .is_some_and(|path| fs::symlink_metadata(path).is_err())
        })
        .collect()
}

/// Writes `formats` for one recording and returns how many files were created.
pub fn write_transcript(
    output_root: &Path,
    relative: &Path,
    formats: &[TranscriptFormat],
    transcript: &Transcript,
    model_label: &str,
) -> Result<usize, CoreError> {
    let first = formats
        .first()
        .and_then(|format| transcript_path(output_root, relative, *format))
        .ok_or(CoreError::InvalidRequest)?;
    let parent = first.parent().ok_or(CoreError::InvalidRequest)?;
    fs::create_dir_all(parent).map_err(CoreError::CopyFailed)?;
    let canonical_root = fs::canonicalize(output_root).map_err(CoreError::CopyFailed)?;
    let canonical_parent = fs::canonicalize(parent).map_err(CoreError::CopyFailed)?;
    if !canonical_parent.starts_with(&canonical_root) {
        return Err(CoreError::DestinationUnavailable);
    }
    let recording = relative.to_string_lossy();
    let mut written = 0;
    for format in formats {
        let name = transcript_path(output_root, relative, *format)
            .and_then(|path| path.file_name().map(ToOwned::to_owned))
            .ok_or(CoreError::InvalidRequest)?;
        let target = canonical_parent.join(name);
        let body = render(*format, transcript, &recording, model_label);
        let mut temporary =
            tempfile::NamedTempFile::new_in(&canonical_parent).map_err(CoreError::CopyFailed)?;
        temporary
            .write_all(body.as_bytes())
            .and_then(|()| temporary.as_file().sync_data())
            .map_err(CoreError::CopyFailed)?;
        match temporary.persist_noclobber(&target) {
            Ok(_) => written += 1,
            Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(CoreError::CopyFailed(error.error)),
        }
    }
    Ok(written)
}

/// The text of one transcript file.
pub fn render(
    format: TranscriptFormat,
    transcript: &Transcript,
    recording: &str,
    model_label: &str,
) -> String {
    match format {
        TranscriptFormat::Txt => transcript
            .segments
            .iter()
            .map(|segment| format!("{}\n", segment.text))
            .collect(),
        TranscriptFormat::Srt => transcript
            .segments
            .iter()
            .enumerate()
            .map(|(index, segment)| {
                format!(
                    "{}\n{} --> {}\n{}\n\n",
                    index + 1,
                    timestamp(segment.start_ms, ','),
                    timestamp(segment.end_ms, ','),
                    segment.text
                )
            })
            .collect(),
        TranscriptFormat::Vtt => std::iter::once("WEBVTT\n\n".to_owned())
            .chain(transcript.segments.iter().map(|segment| {
                format!(
                    "{} --> {}\n{}\n\n",
                    timestamp(segment.start_ms, '.'),
                    timestamp(segment.end_ms, '.'),
                    segment.text
                )
            }))
            .collect(),
        TranscriptFormat::Json => {
            let document = JsonTranscript {
                recording,
                model: model_label,
                language: transcript.language.as_deref(),
                text: transcript
                    .segments
                    .iter()
                    .map(|segment| segment.text.as_str())
                    .collect::<Vec<_>>()
                    .join(" "),
                segments: transcript
                    .segments
                    .iter()
                    .map(|segment| JsonSegment {
                        start_ms: segment.start_ms,
                        end_ms: segment.end_ms,
                        text: &segment.text,
                    })
                    .collect(),
            };
            let mut body = serde_json::to_string_pretty(&document).unwrap_or_default();
            body.push('\n');
            body
        }
    }
}

#[derive(Serialize)]
struct JsonTranscript<'a> {
    recording: &'a str,
    model: &'a str,
    language: Option<&'a str>,
    text: String,
    segments: Vec<JsonSegment<'a>>,
}

#[derive(Serialize)]
struct JsonSegment<'a> {
    start_ms: u64,
    end_ms: u64,
    text: &'a str,
}

/// `HH:MM:SS,mmm` for SubRip and `HH:MM:SS.mmm` for WebVTT.
fn timestamp(ms: u64, separator: char) -> String {
    let hours = ms / 3_600_000;
    let minutes = ms / 60_000 % 60;
    let seconds = ms / 1_000 % 60;
    let millis = ms % 1_000;
    format!("{hours:02}:{minutes:02}:{seconds:02}{separator}{millis:03}")
}

#[cfg(test)]
mod tests {
    use super::super::engine::Segment;
    use super::*;

    fn transcript() -> Transcript {
        Transcript {
            language: Some("ko".to_owned()),
            segments: vec![
                Segment {
                    start_ms: 0,
                    end_ms: 2_500,
                    text: "안녕하세요.".to_owned(),
                },
                Segment {
                    start_ms: 3_723_456,
                    end_ms: 3_725_000,
                    text: "회의를 시작합니다.".to_owned(),
                },
            ],
        }
    }

    #[test]
    fn renders_each_format_with_its_own_timestamp_style() {
        let transcript = transcript();
        assert_eq!(
            render(TranscriptFormat::Txt, &transcript, "a.m4a", "Medium"),
            "안녕하세요.\n회의를 시작합니다.\n"
        );
        assert_eq!(
            render(TranscriptFormat::Srt, &transcript, "a.m4a", "Medium"),
            "1\n00:00:00,000 --> 00:00:02,500\n안녕하세요.\n\n\
             2\n01:02:03,456 --> 01:02:05,000\n회의를 시작합니다.\n\n"
        );
        assert!(
            render(TranscriptFormat::Vtt, &transcript, "a.m4a", "Medium")
                .starts_with("WEBVTT\n\n00:00:00.000 --> 00:00:02.500\n안녕하세요.\n\n")
        );
        let json: serde_json::Value = serde_json::from_str(&render(
            TranscriptFormat::Json,
            &transcript,
            "DJI/2026/10/28/a.m4a",
            "Medium",
        ))
        .unwrap();
        assert_eq!(json["recording"], "DJI/2026/10/28/a.m4a");
        assert_eq!(json["language"], "ko");
        assert_eq!(json["text"], "안녕하세요. 회의를 시작합니다.");
        assert_eq!(json["segments"][1]["start_ms"], 3_723_456);
    }

    #[test]
    fn transcripts_mirror_the_backup_layout_and_never_overwrite() {
        let output = tempfile::tempdir().unwrap();
        let relative = Path::new("DJI/2026/10/28/TX01_MIC001.m4a");
        let formats = [TranscriptFormat::Txt, TranscriptFormat::Srt];
        assert_eq!(missing_formats(output.path(), relative, &formats), formats);

        let edited = output.path().join("DJI/2026/10/28/TX01_MIC001.txt");
        fs::create_dir_all(edited.parent().unwrap()).unwrap();
        fs::write(&edited, "edited by hand").unwrap();
        assert_eq!(
            missing_formats(output.path(), relative, &formats),
            [TranscriptFormat::Srt]
        );

        let written =
            write_transcript(output.path(), relative, &formats, &transcript(), "Medium").unwrap();
        assert_eq!(written, 1, "the existing text file is left alone");
        assert_eq!(fs::read_to_string(&edited).unwrap(), "edited by hand");
        assert!(
            output
                .path()
                .join("DJI/2026/10/28/TX01_MIC001.srt")
                .is_file()
        );
        assert!(missing_formats(output.path(), relative, &formats).is_empty());
    }

    #[test]
    fn a_path_that_could_leave_the_output_folder_is_refused() {
        let output = tempfile::tempdir().unwrap();
        for relative in ["../escape.m4a", "/absolute.m4a", ""] {
            assert!(
                transcript_path(output.path(), Path::new(relative), TranscriptFormat::Txt)
                    .is_none()
            );
            assert!(
                write_transcript(
                    output.path(),
                    Path::new(relative),
                    &[TranscriptFormat::Txt],
                    &transcript(),
                    "Medium"
                )
                .is_err()
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_linked_folder_inside_the_output_cannot_redirect_writes() {
        let output = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(elsewhere.path(), output.path().join("DJI")).unwrap();

        let result = write_transcript(
            output.path(),
            Path::new("DJI/a.m4a"),
            &[TranscriptFormat::Txt],
            &transcript(),
            "Medium",
        );
        assert!(result.is_err());
        assert!(!elsewhere.path().join("a.txt").exists());
    }
}
