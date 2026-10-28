import { useEffect, useState } from "react";
import {
  CaptionsIcon,
  CpuIcon,
  DownloadIcon,
  FileUpIcon,
  FolderOpenIcon,
  PlayIcon,
  RefreshCwIcon,
  RotateCcwIcon,
  SlidersHorizontalIcon,
  SquareIcon,
} from "lucide-react";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Progress } from "@/components/ui/progress";
import { Skeleton } from "@/components/ui/skeleton";
import { Spinner } from "@/components/ui/spinner";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
import { errorCopy, formatBytes, formatTime } from "../backup/format";
import { SettingChoiceRow, SettingRow } from "../backup/SettingsControls";
import { transcriptionClient, type TranscriptionActions } from "./client";
import { DictionarySection } from "./DictionarySection";
import {
  type TranscriptFormats,
  type TranscriptionFolder,
  type TranscriptionSnapshot,
  type WhisperModel,
  type WhisperOptions,
} from "./contracts";
import {
  decoderChoices,
  defaultModelId,
  defaultWhisperOptions,
  formatChoices,
  jobSummary,
  languageChoices,
  samplingChoices,
  threadChoices,
} from "./options";
import { useTranscriptionPersistence } from "./useTranscriptionPersistence";
import { useTranscriptionSnapshot } from "./useTranscriptionSnapshot";

/** Loads the transcription view for the settings window's transcribe tab. */
export function TranscriptionPanel() {
  const { snapshot, loading, errorCode, refresh } = useTranscriptionSnapshot();

  if (loading) {
    return (
      <div className="transcription-loading" aria-busy>
        <Skeleton className="settings-group-skeleton" />
        <Skeleton className="settings-group-skeleton" />
      </div>
    );
  }

  if (!snapshot) {
    const copy = errorCopy(errorCode ?? "snapshot_unavailable");
    return (
      <div className="transcription-loading">
        <Alert variant="destructive">
          <AlertTitle>{copy.title}</AlertTitle>
          <AlertDescription>{copy.detail}</AlertDescription>
        </Alert>
        <Button onClick={() => void refresh()}>
          <RefreshCwIcon data-icon="inline-start" />
          다시 불러오기
        </Button>
      </div>
    );
  }

  return (
    <TranscriptionView snapshot={snapshot} actions={transcriptionClient} />
  );
}

export function TranscriptionView({
  snapshot,
  actions,
}: {
  snapshot: TranscriptionSnapshot;
  actions: TranscriptionActions;
}) {
  const persistence = useTranscriptionPersistence(snapshot, actions);
  const { view, pending } = persistence;
  const optionsPending = pending.has("options");
  const error =
    persistence.actionError ??
    (view.error
      ? {
          messageCode: view.error.message_code,
          ...errorCopy(view.error.message_code),
        }
      : null);

  function setWhisper(change: Partial<WhisperOptions>) {
    void persistence.saveOptions({ whisper: { ...view.whisper, ...change } });
  }

  function folderActions(folder: TranscriptionFolder) {
    return {
      pending: pending.has(`${folder}_folder`),
      onChoose: () => void persistence.chooseFolder(folder),
      onReset: () => void persistence.resetFolder(folder),
      onOpen: () => void persistence.openFolder(folder),
    };
  }

  return (
    <div className="transcription-panel">
      {error ? (
        <Alert variant="destructive">
          <AlertTitle>{error.title}</AlertTitle>
          <AlertDescription>
            <p>{error.detail}</p>
            <p className="support-code">오류 코드: {error.messageCode}</p>
          </AlertDescription>
        </Alert>
      ) : null}

      <section
        className="settings-section"
        aria-labelledby="transcription-title"
      >
        <div className="settings-section-title">
          <CaptionsIcon aria-hidden="true" />
          <div>
            <h2 id="transcription-title">전사</h2>
            <p>백업한 녹음을 Whisper로 전사해 지정한 폴더에 저장합니다.</p>
          </div>
        </div>
        <div className="settings-group">
          <SettingRow
            id="transcription-enabled"
            label="백업 후 자동 전사"
            description="새로 검증된 녹음을 이 Mac에서 전사합니다. 녹음은 외부로 보내지 않습니다."
            checked={view.enabled}
            pending={pending.has("enabled")}
            onChange={(enabled) => void persistence.setEnabled(enabled)}
          />
          <SettingRow
            id="transcription-existing"
            label="이전에 백업한 녹음도 전사"
            description="전사를 켜기 전에 백업된 녹음까지 오래된 순서로 전사합니다."
            checked={view.include_existing_recordings}
            pending={optionsPending}
            onChange={(include) =>
              void persistence.saveOptions({
                include_existing_recordings: include,
              })
            }
          />
          <FolderRow
            label="전사 결과 폴더"
            description="백업 폴더와 같은 하위 폴더 구조로 저장합니다."
            display={view.output_directory_display}
            isDefault={view.output_directory_is_default}
            {...folderActions("output")}
          />
          <FormatsRow
            formats={view.formats}
            pending={optionsPending}
            onChange={(formats) => void persistence.saveOptions({ formats })}
          />
          <JobRow
            view={view}
            pending={pending.has("queue")}
            onRun={() => void persistence.transcribeNow()}
            onCancel={() => void persistence.cancelTranscription()}
          />
        </div>
      </section>

      <section
        className="settings-section"
        aria-labelledby="whisper-model-title"
      >
        <div className="settings-section-title">
          <CpuIcon aria-hidden="true" />
          <div>
            <h2 id="whisper-model-title">Whisper 모델</h2>
            <p>
              전사에 쓸 모델을 고르고 내려받습니다. 클수록 정확하지만 느리고
              메모리를 더 씁니다.
            </p>
          </div>
        </div>
        <div className="settings-group">
          <div className="settings-row">
            <div>
              <strong>사용할 모델</strong>
              <span
                className="settings-destination-path"
                title={view.selected_model.file_display}
              >
                {view.selected_model.label} · {view.selected_model.file_display}
              </span>
            </div>
            <div className="settings-control">
              {view.selected_model.ready ? (
                <Badge variant="secondary">준비됨</Badge>
              ) : (
                <Badge variant="outline">
                  {view.selected_model.source === "custom"
                    ? "파일 없음"
                    : "다운로드 필요"}
                </Badge>
              )}
            </div>
          </div>
          <div
            role="radiogroup"
            aria-label="Whisper 모델"
            className="model-list"
          >
            {view.models.map((model) => (
              <ModelRow
                key={model.id}
                model={model}
                selected={
                  view.selected_model.source === "catalog" &&
                  view.selected_model.id === model.id
                }
                download={view.download}
                selectPending={pending.has("model")}
                downloadPending={pending.has("download")}
                onSelect={() => void persistence.selectModel(model.id)}
                onDownload={() => void persistence.download(model.id)}
                onCancel={() => void persistence.cancelDownload()}
              />
            ))}
          </div>
          <div className="settings-row">
            <div>
              <strong>모델 파일 직접 선택</strong>
              <span>
                {view.selected_model.source === "custom"
                  ? "직접 고른 ggml 모델 파일을 사용하고 있습니다."
                  : "이미 가지고 있는 whisper.cpp ggml 모델(.bin)을 사용합니다."}
              </span>
            </div>
            <div className="settings-control">
              {pending.has("model") ? (
                <Spinner aria-label="모델 선택 중" />
              ) : null}
              <Button
                variant="outline"
                disabled={pending.has("model")}
                onClick={() => void persistence.chooseCustomModel()}
              >
                <FileUpIcon data-icon="inline-start" />
                파일 선택…
              </Button>
            </div>
          </div>
          <FolderRow
            label="모델 폴더"
            description="내려받은 모델을 저장하고 찾는 폴더입니다."
            display={view.models_directory_display}
            isDefault={view.models_directory_is_default}
            {...folderActions("models")}
            locked={view.download !== null}
          />
        </div>
      </section>

      <DictionarySection
        enabled={view.dictionary_enabled}
        entries={view.dictionary}
        pending={pending.has("dictionary")}
        onSave={persistence.saveDictionary}
      />

      <section
        className="settings-section"
        aria-labelledby="whisper-options-title"
      >
        <div className="settings-section-title">
          <SlidersHorizontalIcon aria-hidden="true" />
          <div>
            <h2 id="whisper-options-title">Whisper 옵션</h2>
            <p>
              언어와 디코딩 방식을 조정합니다. 바꾼 값은 다음 녹음부터
              적용됩니다.
            </p>
          </div>
        </div>
        <div className="settings-group">
          <TextChoiceRow
            id="whisper-language"
            label="언어"
            description="자동 감지는 녹음 앞부분으로 언어를 판단합니다."
            value={view.whisper.language}
            choices={languageChoices}
            pending={optionsPending}
            onChange={(language) => setWhisper({ language })}
          />
          <SettingRow
            id="whisper-translate"
            label="영어로 번역"
            description="받아쓰는 대신 영어 번역문을 저장합니다."
            checked={view.whisper.translate}
            pending={optionsPending}
            onChange={(translate) => setWhisper({ translate })}
          />
          <TextChoiceRow
            id="whisper-sampling"
            label="디코딩 방식"
            description="빔 서치가 대체로 정확하고, 그리디가 더 빠릅니다."
            value={view.whisper.sampling}
            choices={samplingChoices}
            pending={optionsPending}
            onChange={(sampling) =>
              setWhisper({
                sampling: sampling === "greedy" ? "greedy" : "beam_search",
              })
            }
          />
          {view.whisper.sampling === "beam_search" ? (
            <SettingChoiceRow
              id="whisper-beam-size"
              label="빔 크기"
              description="동시에 살펴볼 후보 수입니다."
              value={view.whisper.beam_size}
              choices={decoderChoices}
              pending={optionsPending}
              onChange={(beamSize) => setWhisper({ beam_size: beamSize })}
            />
          ) : (
            <SettingChoiceRow
              id="whisper-best-of"
              label="후보 수 (best of)"
              description="온도를 높여 다시 시도할 때 비교할 후보 수입니다."
              value={view.whisper.best_of}
              choices={decoderChoices}
              pending={optionsPending}
              onChange={(bestOf) => setWhisper({ best_of: bestOf })}
            />
          )}
          <NumberRow
            id="whisper-temperature"
            label="온도"
            description="0이면 가장 확실한 결과만 고릅니다 (0–1)."
            value={view.whisper.temperature}
            min={0}
            max={1}
            step={0.1}
            pending={optionsPending}
            onCommit={(temperature) => setWhisper({ temperature })}
          />
          <NumberRow
            id="whisper-temperature-increment"
            label="온도 증가폭"
            description="결과가 불확실할 때 온도를 이만큼 올려 다시 시도합니다. 0이면 다시 시도하지 않습니다."
            value={view.whisper.temperature_increment}
            min={0}
            max={1}
            step={0.1}
            pending={optionsPending}
            onCommit={(increment) =>
              setWhisper({ temperature_increment: increment })
            }
          />
        </div>
      </section>

      <section
        className="settings-section"
        aria-labelledby="whisper-advanced-title"
      >
        <details className="settings-advanced">
          <summary>
            <span id="whisper-advanced-title">고급 옵션</span>
          </summary>
          <div className="settings-group">
            <SettingChoiceRow
              id="whisper-threads"
              label="CPU 스레드"
              description="자동은 최대 4개를 씁니다."
              value={view.whisper.threads}
              choices={threadChoices}
              pending={optionsPending}
              onChange={(threads) => setWhisper({ threads })}
            />
            <SettingRow
              id="whisper-gpu"
              label="GPU(Metal) 사용"
              description="Apple Silicon GPU로 전사해 훨씬 빠릅니다."
              checked={view.whisper.use_gpu}
              pending={optionsPending}
              onChange={(useGpu) => setWhisper({ use_gpu: useGpu })}
            />
            <SettingRow
              id="whisper-flash-attention"
              label="Flash Attention"
              description="GPU 메모리를 덜 쓰고 더 빠르게 계산합니다."
              checked={view.whisper.flash_attention}
              pending={optionsPending}
              onChange={(flash) => setWhisper({ flash_attention: flash })}
            />
            <SettingRow
              id="whisper-context"
              label="이전 문맥 이어 쓰기"
              description="앞 구간의 문장을 참고해 표기가 일관되지만, 오류가 반복될 수 있습니다."
              checked={view.whisper.condition_on_previous_text}
              pending={optionsPending}
              onChange={(context) =>
                setWhisper({ condition_on_previous_text: context })
              }
            />
            <NumberRow
              id="whisper-no-speech"
              label="무음 판정 임계값"
              description="이 값보다 무음일 확률이 높은 구간은 건너뜁니다 (0–1)."
              value={view.whisper.no_speech_threshold}
              min={0}
              max={1}
              step={0.05}
              pending={optionsPending}
              onCommit={(threshold) =>
                setWhisper({ no_speech_threshold: threshold })
              }
            />
            <NumberRow
              id="whisper-entropy"
              label="엔트로피 임계값"
              description="결과가 이보다 반복적이면 온도를 올려 다시 시도합니다 (0–10)."
              value={view.whisper.entropy_threshold}
              min={0}
              max={10}
              step={0.1}
              pending={optionsPending}
              onCommit={(threshold) =>
                setWhisper({ entropy_threshold: threshold })
              }
            />
            <NumberRow
              id="whisper-logprob"
              label="로그 확률 임계값"
              description="평균 로그 확률이 이보다 낮으면 다시 시도합니다 (−10–0)."
              value={view.whisper.logprob_threshold}
              min={-10}
              max={0}
              step={0.1}
              pending={optionsPending}
              onCommit={(threshold) =>
                setWhisper({ logprob_threshold: threshold })
              }
            />
            <NumberRow
              id="whisper-max-length"
              label="최대 세그먼트 길이"
              description="한 자막 구간의 최대 글자 수입니다. 0이면 모델에 맡깁니다."
              value={view.whisper.max_segment_length}
              min={0}
              max={1000}
              step={1}
              pending={optionsPending}
              onCommit={(length) => setWhisper({ max_segment_length: length })}
            />
            <SettingRow
              id="whisper-split-on-word"
              label="단어 단위로 나누기"
              description="최대 길이로 자를 때 단어 중간을 끊지 않습니다."
              checked={view.whisper.split_on_word}
              pending={optionsPending}
              onChange={(split) => setWhisper({ split_on_word: split })}
            />
            <SettingRow
              id="whisper-suppress-blank"
              label="빈 출력 억제"
              description="구간 시작에 빈 문장이 나오지 않게 합니다."
              checked={view.whisper.suppress_blank}
              pending={optionsPending}
              onChange={(suppress) => setWhisper({ suppress_blank: suppress })}
            />
            <SettingRow
              id="whisper-suppress-non-speech"
              label="비음성 토큰 억제"
              description="음악 기호나 효과음 표기를 줄입니다."
              checked={view.whisper.suppress_non_speech}
              pending={optionsPending}
              onChange={(suppress) =>
                setWhisper({ suppress_non_speech: suppress })
              }
            />
            <div className="settings-shortcuts">
              <Button
                variant="outline"
                disabled={optionsPending}
                onClick={() =>
                  void persistence.saveOptions({
                    whisper: { ...defaultWhisperOptions },
                  })
                }
              >
                <RotateCcwIcon data-icon="inline-start" />
                Whisper 옵션 기본값으로
              </Button>
            </div>
          </div>
        </details>
      </section>
    </div>
  );
}

function FolderRow({
  label,
  description,
  display,
  isDefault,
  pending,
  locked = false,
  onChoose,
  onReset,
  onOpen,
}: {
  label: string;
  description: string;
  display: string;
  isDefault: boolean;
  pending: boolean;
  /** A model download writes into this folder, so it cannot change until it ends. */
  locked?: boolean;
  onChoose: () => void;
  onReset: () => void;
  onOpen: () => void;
}) {
  return (
    <div className="settings-row settings-destination-row">
      <div>
        <strong>{label}</strong>
        <span className="settings-destination-path" title={display}>
          {display}
        </span>
        <span>{description}</span>
      </div>
      <div className="settings-control folder-actions">
        {pending ? <Spinner aria-label={`${label} 처리 중`} /> : null}
        <Button
          variant="ghost"
          size="icon"
          aria-label={`${label} 열기`}
          disabled={pending}
          onClick={onOpen}
        >
          <FolderOpenIcon />
        </Button>
        {isDefault ? null : (
          <Button
            variant="ghost"
            disabled={pending || locked}
            onClick={onReset}
          >
            기본값
          </Button>
        )}
        <Button
          variant="outline"
          aria-label={`${label} 변경`}
          disabled={pending || locked}
          onClick={onChoose}
        >
          변경…
        </Button>
      </div>
    </div>
  );
}

function FormatsRow({
  formats,
  pending,
  onChange,
}: {
  formats: TranscriptFormats;
  pending: boolean;
  onChange: (formats: TranscriptFormats) => void;
}) {
  const selected = formatChoices
    .filter((choice) => formats[choice.key])
    .map((choice) => choice.key);
  return (
    <div className="settings-row">
      <div>
        <strong>저장 형식</strong>
        <span>
          하나 이상 고르세요. 이미 있는 전사 파일은 덮어쓰지 않습니다.
        </span>
      </div>
      <div className="settings-control">
        {pending ? <Spinner aria-label="저장 형식 저장 중" /> : null}
        <ToggleGroup
          type="multiple"
          variant="outline"
          size="sm"
          aria-label="저장 형식"
          className="format-toggles"
          value={selected}
          disabled={pending}
          onValueChange={(keys) => {
            if (keys.length === 0) return;
            onChange({
              txt: keys.includes("txt"),
              srt: keys.includes("srt"),
              vtt: keys.includes("vtt"),
              json: keys.includes("json"),
            });
          }}
        >
          {formatChoices.map((choice) => (
            <ToggleGroupItem
              key={choice.key}
              value={choice.key}
              title={choice.description}
              aria-label={`${choice.label}: ${choice.description}`}
            >
              {choice.label}
            </ToggleGroupItem>
          ))}
        </ToggleGroup>
      </div>
    </div>
  );
}

function JobRow({
  view,
  pending,
  onRun,
  onCancel,
}: {
  view: TranscriptionSnapshot;
  pending: boolean;
  onRun: () => void;
  onCancel: () => void;
}) {
  const { job } = view;
  const transcribing = job.phase === "transcribing";
  const lastCompleted = formatTime(job.last_completed_at);
  const remaining = transcribing
    ? Math.max(job.queued_count - 1, 0)
    : job.queued_count;
  return (
    <div className="settings-row transcription-job" aria-live="polite">
      <div>
        <strong>{jobSummary(view)}</strong>
        <span>
          {[
            remaining > 0 && transcribing ? `대기 ${remaining}개` : null,
            `완료 ${job.completed_count}개`,
            job.failed_count > 0 ? `실패 ${job.failed_count}개` : null,
            lastCompleted ? `마지막 완료 ${lastCompleted}` : null,
          ]
            .filter(Boolean)
            .join(" · ")}
        </span>
        {transcribing ? (
          <Progress
            value={job.percent}
            aria-label={`전사 진행률 ${job.percent}%`}
          />
        ) : null}
      </div>
      <div className="settings-control">
        {pending ? <Spinner aria-label="전사 요청 중" /> : null}
        {transcribing ? (
          <Button variant="outline" disabled={pending} onClick={onCancel}>
            <SquareIcon data-icon="inline-start" />
            중단
          </Button>
        ) : (
          <Button
            variant="outline"
            disabled={pending || !view.enabled}
            onClick={onRun}
          >
            <PlayIcon data-icon="inline-start" />
            지금 전사
          </Button>
        )}
      </div>
    </div>
  );
}

function ModelRow({
  model,
  selected,
  download,
  selectPending,
  downloadPending,
  onSelect,
  onDownload,
  onCancel,
}: {
  model: WhisperModel;
  selected: boolean;
  download: TranscriptionSnapshot["download"];
  selectPending: boolean;
  downloadPending: boolean;
  onSelect: () => void;
  onDownload: () => void;
  onCancel: () => void;
}) {
  const downloading = download?.model_id === model.id;
  const percent = downloading
    ? Math.min(
        100,
        Math.floor((download.received_bytes / download.total_bytes) * 100),
      )
    : 0;
  const inputId = `whisper-model-${model.id}`;
  return (
    <div
      className="settings-row model-row"
      data-selected={selected || undefined}
    >
      <label htmlFor={inputId} className="model-label">
        <input
          id={inputId}
          type="radio"
          name="whisper-model"
          aria-label={`${model.label}${model.id === defaultModelId ? " (기본값)" : ""}`}
          checked={selected}
          disabled={selectPending}
          onChange={onSelect}
        />
        <span className="model-text">
          <strong>
            {model.label}
            {model.id === defaultModelId ? " (기본값)" : ""}
          </strong>
          <span>
            {[
              formatBytes(model.byte_count),
              model.english_only ? "영어 전용" : "다국어",
              model.quantized ? "양자화" : null,
            ]
              .filter(Boolean)
              .join(" · ")}
          </span>
        </span>
      </label>
      <div className="settings-control">
        {model.installed ? (
          <Badge variant="secondary">설치됨</Badge>
        ) : downloading ? (
          <>
            <Progress
              className="model-progress"
              value={percent}
              aria-label={`${model.label} 다운로드 ${percent}%`}
            />
            <span className="model-percent" aria-hidden="true">
              {percent}%
            </span>
            <Button
              variant="ghost"
              aria-label={`${model.label} 다운로드 중단`}
              disabled={downloadPending}
              onClick={onCancel}
            >
              중단
            </Button>
          </>
        ) : (
          <Button
            variant="outline"
            aria-label={`${model.label} 다운로드`}
            disabled={download !== null || downloadPending}
            onClick={onDownload}
          >
            <DownloadIcon data-icon="inline-start" />
            다운로드
          </Button>
        )}
      </div>
    </div>
  );
}

/**
 * A settings row whose value is one of a few strings. Like `SettingChoiceRow`, a stored value
 * the list does not offer is shown as an extra option instead of being misreported.
 */
function TextChoiceRow({
  id,
  label,
  description,
  value,
  choices,
  pending,
  onChange,
}: {
  id: string;
  label: string;
  description: string;
  value: string;
  choices: ReadonlyArray<{ value: string; label: string }>;
  pending: boolean;
  onChange: (value: string) => void;
}) {
  const options = choices.some((choice) => choice.value === value)
    ? choices
    : [...choices, { value, label: value }];
  return (
    <div className="settings-row">
      <label htmlFor={id}>
        <strong>{label}</strong>
        <span>{description}</span>
      </label>
      <div className="settings-control">
        {pending ? <Spinner aria-label={`${label} 저장 중`} /> : null}
        <select
          id={id}
          className="settings-choice"
          aria-label={label}
          value={value}
          disabled={pending}
          onChange={(event) => onChange(event.currentTarget.value)}
        >
          {options.map((choice) => (
            <option key={choice.value} value={choice.value}>
              {choice.label}
            </option>
          ))}
        </select>
      </div>
    </div>
  );
}

/**
 * A number the user types. It is saved when the field loses focus or Enter is pressed, and only
 * when it is inside the range Rust accepts; anything else puts the stored value back.
 */
function NumberRow({
  id,
  label,
  description,
  value,
  min,
  max,
  step,
  pending,
  onCommit,
}: {
  id: string;
  label: string;
  description: string;
  value: number;
  min: number;
  max: number;
  step: number;
  pending: boolean;
  onCommit: (value: number) => void;
}) {
  const [draft, setDraft] = useState(String(value));
  const [editing, setEditing] = useState(false);

  useEffect(() => {
    if (!editing) setDraft(String(value));
  }, [editing, value]);

  function commit() {
    setEditing(false);
    const parsed = Number(draft);
    if (
      draft.trim() === "" ||
      !Number.isFinite(parsed) ||
      parsed < min ||
      parsed > max
    ) {
      setDraft(String(value));
      return;
    }
    const rounded =
      step >= 1 ? Math.round(parsed) : Math.round(parsed * 100) / 100;
    setDraft(String(rounded));
    if (rounded !== value) onCommit(rounded);
  }

  return (
    <div className="settings-row">
      <label htmlFor={id}>
        <strong>{label}</strong>
        <span>{description}</span>
      </label>
      <div className="settings-control">
        {pending ? <Spinner aria-label={`${label} 저장 중`} /> : null}
        <input
          id={id}
          type="number"
          aria-label={label}
          inputMode="decimal"
          className="settings-number"
          min={min}
          max={max}
          step={step}
          value={draft}
          disabled={pending}
          onFocus={() => setEditing(true)}
          onChange={(event) => setDraft(event.currentTarget.value)}
          onBlur={commit}
          onKeyDown={(event) => {
            if (event.key === "Enter") event.currentTarget.blur();
          }}
        />
      </div>
    </div>
  );
}
