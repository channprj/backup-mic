import { LanguagesIcon } from "lucide-react";
import { SettingRow } from "../backup/SettingsControls";
import {
  ollamaModelPattern,
  ollamaUrlPattern,
  type TranslationSettings,
} from "./contracts";
import {
  defaultOllamaUrl,
  isLocalOllamaUrl,
  translationLanguageChoices,
} from "./options";
import { TextChoiceRow, TextInputRow } from "./TranscriptionControls";

/**
 * Translating finished transcripts with a local Ollama model.
 *
 * Whisper only translates into English, so this replaces its switch with a target language and
 * a model of the user's choosing. Nothing is sent until a model name is entered.
 */
export function TranslationSection({
  translation,
  pending,
  onSave,
}: {
  translation: TranslationSettings;
  pending: boolean;
  onSave: (translation: TranslationSettings) => void;
}) {
  const save = (change: Partial<TranslationSettings>) =>
    onSave({ ...translation, ...change });
  const waitingForModel = translation.enabled && !translation.ollama_model;
  const remote = !isLocalOllamaUrl(translation.ollama_url);

  return (
    <section className="settings-section" aria-labelledby="translation-title">
      <div className="settings-section-title">
        <LanguagesIcon aria-hidden="true" />
        <div>
          <h2 id="translation-title">번역</h2>
          <p>
            전사가 끝나면 Ollama 모델로 번역해 원본 옆에{" "}
            <code>이름.언어.txt</code>처럼 저장합니다.
          </p>
        </div>
      </div>
      <div className="settings-group">
        <SettingRow
          id="translation-enabled"
          label="전사 후 번역"
          description="Ollama가 실행 중이어야 합니다. 원본 전사는 번역과 상관없이 저장됩니다."
          checked={translation.enabled}
          pending={pending}
          onChange={(enabled) => save({ enabled })}
        />
        <TextChoiceRow
          id="translation-language"
          label="번역 언어"
          description="녹음이 이미 이 언어이면 모델에 보내지 않고 그대로 저장합니다."
          value={translation.target_language}
          choices={translationLanguageChoices}
          pending={pending}
          onChange={(target_language) => save({ target_language })}
        />
        <TextInputRow
          id="translation-ollama-url"
          label="Ollama 주소"
          description={`기본값은 ${defaultOllamaUrl}입니다.`}
          value={translation.ollama_url}
          placeholder={defaultOllamaUrl}
          pending={pending}
          isValid={(url) => url.length <= 512 && ollamaUrlPattern.test(url)}
          onCommit={(url) => save({ ollama_url: url.replace(/\/+$/u, "") })}
        />
        <TextInputRow
          id="translation-ollama-model"
          label="모델"
          description="Ollama에 받아 둔 모델 이름입니다. 비워 두면 번역하지 않습니다."
          value={translation.ollama_model}
          placeholder="예: qwen2.5:7b"
          pending={pending}
          isValid={(model) =>
            model.length <= 128 && ollamaModelPattern.test(model)
          }
          onCommit={(ollama_model) => save({ ollama_model })}
        />
        {waitingForModel ? (
          <p className="settings-status-note is-info" role="status">
            모델 이름을 넣으면 다음 녹음부터 번역합니다.
          </p>
        ) : null}
        {remote ? (
          <p className="settings-status-note" role="status">
            다른 컴퓨터의 Ollama 주소입니다. 전사한 텍스트가 그 컴퓨터로
            전송됩니다.
          </p>
        ) : null}
      </div>
    </section>
  );
}
