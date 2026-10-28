import type { TranscriptionSnapshot } from "./contracts";

/**
 * The languages the window offers by name.
 *
 * Rust accepts every whisper.cpp language code, so a stored code that is not listed here is
 * shown as an extra option rather than silently replaced.
 */
export const languageChoices = [
  { value: "auto", label: "자동 감지" },
  { value: "ko", label: "한국어" },
  { value: "en", label: "영어" },
  { value: "ja", label: "일본어" },
  { value: "zh", label: "중국어" },
  { value: "yue", label: "광둥어" },
  { value: "es", label: "스페인어" },
  { value: "fr", label: "프랑스어" },
  { value: "de", label: "독일어" },
  { value: "it", label: "이탈리아어" },
  { value: "pt", label: "포르투갈어" },
  { value: "ru", label: "러시아어" },
  { value: "vi", label: "베트남어" },
  { value: "th", label: "태국어" },
  { value: "id", label: "인도네시아어" },
  { value: "hi", label: "힌디어" },
  { value: "ar", label: "아랍어" },
  { value: "tr", label: "튀르키예어" },
  { value: "nl", label: "네덜란드어" },
  { value: "pl", label: "폴란드어" },
  { value: "uk", label: "우크라이나어" },
] as const;

export const samplingChoices = [
  { value: "beam_search", label: "빔 서치" },
  { value: "greedy", label: "그리디" },
] as const;

export const decoderChoices = [1, 2, 3, 4, 5, 6, 7, 8].map((value) => ({
  value,
  label: String(value),
}));

export const threadChoices = [0, 1, 2, 4, 6, 8, 12, 16].map((value) => ({
  value,
  label: value === 0 ? "자동" : `${value}개`,
}));

export const formatChoices = [
  { key: "txt", label: "TXT", description: "문장만 담은 텍스트" },
  { key: "srt", label: "SRT", description: "시간이 붙은 자막" },
  { key: "vtt", label: "VTT", description: "웹 자막" },
  { key: "json", label: "JSON", description: "세그먼트별 시간과 텍스트" },
] as const;

/** The whisper.cpp defaults, mirrored from `WhisperOptions::default` in Rust. */
export const defaultWhisperOptions: TranscriptionSnapshot["whisper"] = {
  language: "auto",
  translate: false,
  sampling: "beam_search",
  beam_size: 5,
  best_of: 5,
  temperature: 0,
  temperature_increment: 0.2,
  initial_prompt: "",
  threads: 0,
  use_gpu: true,
  flash_attention: true,
  no_speech_threshold: 0.6,
  entropy_threshold: 2.4,
  logprob_threshold: -1,
  max_segment_length: 0,
  split_on_word: false,
  suppress_blank: true,
  suppress_non_speech: false,
  condition_on_previous_text: false,
};

export const defaultModelId = "medium";

/** One line describing what the queue is doing. */
export function jobSummary(snapshot: TranscriptionSnapshot) {
  const { job } = snapshot;
  if (!snapshot.enabled) return "전사가 꺼져 있습니다";
  switch (job.phase) {
    case "transcribing":
      return `${job.current_item ?? "녹음"} 전사 중`;
    case "waiting_for_model":
      return `모델이 준비되면 녹음 ${job.queued_count}개를 전사합니다`;
    case "waiting_for_backup":
      return "백업이 끝나면 전사를 이어갑니다";
    case "idle":
      return "새 녹음이 백업되면 전사합니다";
  }
}
