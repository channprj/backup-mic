//! Translating finished transcripts with a local Ollama model.
//!
//! Whisper only translates into English, so other targets go to a chat model. Segments are sent
//! in batches and must come back one for one: Ollama's structured output pins the reply to
//! `{"translations": [...]}`, and a batch whose count does not match is retried a segment at a
//! time so a model that merges lines cannot shift every later timestamp.

use std::{
    fs,
    path::Path,
    process::{Command, Stdio},
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

use serde::Deserialize;
use serde_json::json;

const POLL_INTERVAL: Duration = Duration::from_millis(200);
/// Segments per request; small enough for a 7B model's context with room for the reply.
const BATCH_SEGMENTS: usize = 40;
const BATCH_CHARS: usize = 6_000;
const MAX_RESPONSE_BYTES: u64 = 16 * 1024 * 1024;

/// Why a translation stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TranslateError {
    Cancelled,
    /// Ollama did not answer: not running, wrong address, or the model is missing.
    Unavailable,
    /// Ollama answered with something other than one translation per segment.
    InvalidResponse,
}

/// Translates transcript segments, one output per input, in order.
pub trait Translator: Send + Sync {
    fn translate(
        &self,
        texts: &[String],
        target_language: &str,
        glossary: Option<&str>,
        cancel: &AtomicBool,
    ) -> Result<Vec<String>, TranslateError>;
}

/// Ollama's `/api/chat`, reached with the system `curl`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OllamaTranslator {
    pub url: String,
    pub model: String,
}

impl Translator for OllamaTranslator {
    fn translate(
        &self,
        texts: &[String],
        target_language: &str,
        glossary: Option<&str>,
        cancel: &AtomicBool,
    ) -> Result<Vec<String>, TranslateError> {
        translate_in_batches(texts, |batch| {
            if cancel.load(Ordering::SeqCst) {
                return Err(TranslateError::Cancelled);
            }
            let body = request_body(&self.model, batch, target_language, glossary);
            let response = post_json(&self.url, &body, cancel)?;
            parse_response(&response, batch.len())
        })
    }
}

/// Splits `texts` into batches, sends each through `send`, and falls back to one segment per
/// request for a batch whose reply has the wrong number of translations.
pub fn translate_in_batches(
    texts: &[String],
    mut send: impl FnMut(&[String]) -> Result<Vec<String>, TranslateError>,
) -> Result<Vec<String>, TranslateError> {
    let mut translated = Vec::with_capacity(texts.len());
    let mut start = 0;
    while start < texts.len() {
        let mut end = start;
        let mut chars = 0;
        while end < texts.len()
            && end - start < BATCH_SEGMENTS
            && (end == start || chars + texts[end].chars().count() <= BATCH_CHARS)
        {
            chars += texts[end].chars().count();
            end += 1;
        }
        let batch = &texts[start..end];
        match send(batch) {
            Ok(result) => translated.extend(result),
            Err(TranslateError::InvalidResponse) if batch.len() > 1 => {
                for text in batch {
                    translated.extend(send(std::slice::from_ref(text))?);
                }
            }
            Err(error) => return Err(error),
        }
        start = end;
    }
    Ok(translated)
}

/// The chat request for one batch.
pub fn request_body(
    model: &str,
    batch: &[String],
    target_language: &str,
    glossary: Option<&str>,
) -> String {
    let language = language_name(target_language);
    let mut instructions = format!(
        "You translate segments of a voice recording transcript into {language} \
         ({target_language}). Reply with JSON {{\"translations\": [...]}} holding exactly one \
         translation per input segment, in the same order. Translate each segment's meaning \
         faithfully and keep names and numbers. Never merge, split, skip, or comment on \
         segments. A segment already in {language} is returned unchanged."
    );
    if let Some(glossary) = glossary.filter(|glossary| !glossary.is_empty()) {
        instructions.push_str(&format!(
            " Keep these terms exactly as written: {glossary}."
        ));
    }
    json!({
        "model": model,
        "stream": false,
        "format": {
            "type": "object",
            "properties": {
                "translations": { "type": "array", "items": { "type": "string" } }
            },
            "required": ["translations"]
        },
        "options": { "temperature": 0 },
        "messages": [
            { "role": "system", "content": instructions },
            { "role": "user", "content": serde_json::to_string(batch).unwrap_or_default() }
        ]
    })
    .to_string()
}

#[derive(Deserialize)]
struct ChatResponse {
    message: ChatMessage,
}

#[derive(Deserialize)]
struct ChatMessage {
    content: String,
}

#[derive(Deserialize)]
struct Translations {
    translations: Vec<String>,
}

/// Reads one batch's translations, refusing a reply that does not have exactly `expected`.
pub fn parse_response(response: &str, expected: usize) -> Result<Vec<String>, TranslateError> {
    let chat = serde_json::from_str::<ChatResponse>(response)
        .map_err(|_| TranslateError::InvalidResponse)?;
    let translations = serde_json::from_str::<Translations>(&chat.message.content)
        .map_err(|_| TranslateError::InvalidResponse)?
        .translations;
    if translations.len() == expected {
        Ok(translations
            .into_iter()
            .map(|text| text.split_whitespace().collect::<Vec<_>>().join(" "))
            .collect())
    } else {
        Err(TranslateError::InvalidResponse)
    }
}

/// Posts `body` to `{url}/api/chat` and returns the reply, polling `cancel` while it waits.
fn post_json(url: &str, body: &str, cancel: &AtomicBool) -> Result<String, TranslateError> {
    use std::io::Write as _;

    let directory = tempfile::tempdir().map_err(|_| TranslateError::Unavailable)?;
    let output = directory.path().join("response.json");
    let mut child = Command::new("/usr/bin/curl")
        .args(curl_arguments(url, &output))
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| TranslateError::Unavailable)?;
    let written = child
        .stdin
        .take()
        .map(|mut stdin| stdin.write_all(body.as_bytes()));
    if !matches!(written, Some(Ok(()))) {
        let _ = child.kill();
        let _ = child.wait();
        return Err(TranslateError::Unavailable);
    }
    loop {
        if cancel.load(Ordering::SeqCst) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(TranslateError::Cancelled);
        }
        match child.try_wait() {
            Ok(Some(status)) if status.success() => break,
            Ok(Some(_)) | Err(_) => return Err(TranslateError::Unavailable),
            Ok(None) => std::thread::sleep(POLL_INTERVAL),
        }
    }
    let length = fs::metadata(&output)
        .map_err(|_| TranslateError::InvalidResponse)?
        .len();
    if length > MAX_RESPONSE_BYTES {
        return Err(TranslateError::InvalidResponse);
    }
    fs::read_to_string(&output).map_err(|_| TranslateError::InvalidResponse)
}

/// The exact `curl` invocation: plain HTTP(S) only, the body from stdin, the reply to a file.
pub fn curl_arguments(url: &str, output: &Path) -> Vec<std::ffi::OsString> {
    let mut arguments = [
        "--fail",
        "--silent",
        "--proto",
        "=http,https",
        "--connect-timeout",
        "10",
        "--max-time",
        "900",
        "--header",
        "Content-Type: application/json",
        "--data-binary",
        "@-",
        "--output",
    ]
    .map(std::ffi::OsString::from)
    .to_vec();
    arguments.push(output.as_os_str().to_owned());
    arguments.push(format!("{}/api/chat", url.trim_end_matches('/')).into());
    arguments
}

/// The English name a chat model recognises for a whisper language code.
pub fn language_name(code: &str) -> &str {
    match code {
        "ko" => "Korean",
        "en" => "English",
        "ja" => "Japanese",
        "zh" => "Chinese",
        "yue" => "Cantonese",
        "es" => "Spanish",
        "fr" => "French",
        "de" => "German",
        "it" => "Italian",
        "pt" => "Portuguese",
        "ru" => "Russian",
        "vi" => "Vietnamese",
        "th" => "Thai",
        "id" => "Indonesian",
        "hi" => "Hindi",
        "ar" => "Arabic",
        "tr" => "Turkish",
        "nl" => "Dutch",
        "pl" => "Polish",
        "uk" => "Ukrainian",
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;

    fn texts(count: usize) -> Vec<String> {
        (0..count).map(|index| format!("segment {index}")).collect()
    }

    #[test]
    fn the_request_pins_the_reply_shape_and_carries_the_glossary() {
        let body: serde_json::Value = serde_json::from_str(&request_body(
            "qwen2.5:7b",
            &["안녕하세요".to_owned()],
            "en",
            Some("Backup Mic, DJI"),
        ))
        .unwrap();
        assert_eq!(body["model"], "qwen2.5:7b");
        assert_eq!(body["stream"], false);
        assert_eq!(body["format"]["required"][0], "translations");
        assert_eq!(body["options"]["temperature"], 0);
        let system = body["messages"][0]["content"].as_str().unwrap();
        assert!(system.contains("into English (en)"));
        assert!(system.contains("Keep these terms exactly as written: Backup Mic, DJI."));
        assert_eq!(body["messages"][1]["content"], "[\"안녕하세요\"]");
    }

    #[test]
    fn a_reply_must_have_one_translation_per_segment() {
        let reply = |content: &str| json!({ "message": { "content": content } }).to_string();
        assert_eq!(
            parse_response(&reply(r#"{"translations":["Hello\nthere","Bye"]}"#), 2).unwrap(),
            ["Hello there", "Bye"]
        );
        assert_eq!(
            parse_response(&reply(r#"{"translations":["only one"]}"#), 2),
            Err(TranslateError::InvalidResponse)
        );
        assert_eq!(
            parse_response(&reply("not json"), 1),
            Err(TranslateError::InvalidResponse)
        );
        assert_eq!(
            parse_response("{\"error\":\"model not found\"}", 1),
            Err(TranslateError::InvalidResponse)
        );
    }

    #[test]
    fn batches_stay_small_and_a_miscounted_batch_is_retried_one_by_one() {
        let sizes = RefCell::new(Vec::new());
        let result = translate_in_batches(&texts(90), |batch| {
            sizes.borrow_mut().push(batch.len());
            if batch.len() == 40 && sizes.borrow().len() == 1 {
                return Err(TranslateError::InvalidResponse);
            }
            Ok(batch.iter().map(|text| text.to_uppercase()).collect())
        })
        .unwrap();
        assert_eq!(result.len(), 90);
        assert_eq!(result[89], "SEGMENT 89");
        let sizes = sizes.into_inner();
        assert_eq!(sizes[0], 40);
        assert!(sizes[1..41].iter().all(|size| *size == 1), "{sizes:?}");
        assert_eq!(&sizes[41..], [40, 10]);
    }

    #[test]
    fn an_unreachable_server_stops_the_whole_translation() {
        let calls = RefCell::new(0);
        let result = translate_in_batches(&texts(50), |_| {
            *calls.borrow_mut() += 1;
            Err(TranslateError::Unavailable)
        });
        assert_eq!(result, Err(TranslateError::Unavailable));
        assert_eq!(*calls.borrow(), 1);
    }

    #[test]
    fn curl_posts_json_from_stdin_to_the_chat_endpoint_only_over_http() {
        let arguments = curl_arguments("http://localhost:11434/", Path::new("/tmp/out.json"));
        let arguments = arguments
            .iter()
            .map(|argument| argument.to_str().unwrap())
            .collect::<Vec<_>>();
        for pair in [
            ["--proto", "=http,https"],
            ["--data-binary", "@-"],
            ["--output", "/tmp/out.json"],
        ] {
            assert!(
                arguments.windows(2).any(|window| window == pair),
                "{pair:?}"
            );
        }
        assert_eq!(arguments.last(), Some(&"http://localhost:11434/api/chat"));
    }
}
