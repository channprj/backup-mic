import { useEffect, useId, useMemo, useState } from "react";
import { PlusIcon, TestTube2Icon, Trash2Icon } from "lucide-react";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { Spinner } from "@/components/ui/spinner";
import { Switch } from "@/components/ui/switch";
import type { BackupRule, BackupRuleDraft, RuleTestResult } from "./contracts";

export interface RuleEditorProps {
  rule: BackupRule | null;
  initialDraft?: BackupRuleDraft;
  artifactFormat: "wav" | "m4a";
  connectedVolumeNames: string[];
  busy: boolean;
  onCancel: () => void;
  onSave: (draft: BackupRuleDraft) => Promise<void>;
  onTest: (draft: BackupRuleDraft) => Promise<RuleTestResult>;
}

type PatternKey =
  | "required_path_globs"
  | "backup_file_globs"
  | "session_directory_globs";

const patternLabels: Record<PatternKey, string> = {
  required_path_globs: "필수 경로 glob",
  backup_file_globs: "백업 파일 glob",
  session_directory_globs: "세션 폴더 glob",
};

const dateFolderLayouts: Array<{
  value: BackupRuleDraft["date_folder_layout"];
  label: string;
}> = [
  { value: "year_month_day", label: "YYYY/MM/DD/" },
  { value: "year_month", label: "YYYY/MM/" },
  { value: "compact_date", label: "YYMMDD/" },
];

function newDraft(): BackupRuleDraft {
  return {
    id: null,
    name: "",
    archive_directory_name: "",
    enabled: true,
    volume_name_glob: "*",
    required_path_globs: [],
    backup_file_globs: ["*.WAV"],
    session_directory_globs: [],
    filename_prefix: "",
    filename_suffix: "",
    date_folder_layout: "year_month",
  };
}

function draftFromRule(rule: BackupRule): BackupRuleDraft {
  return {
    id: rule.id,
    name: rule.name,
    archive_directory_name: rule.archive_directory_name,
    enabled: rule.enabled,
    volume_name_glob: rule.volume_name_glob,
    required_path_globs: [...rule.required_path_globs],
    backup_file_globs: [...rule.backup_file_globs],
    session_directory_globs: [...rule.session_directory_globs],
    filename_prefix: rule.filename_prefix,
    filename_suffix: rule.filename_suffix,
    date_folder_layout: rule.date_folder_layout,
  };
}

function pad2(value: number) {
  return String(value).padStart(2, "0");
}

export function formatRulePreview(
  draft: BackupRuleDraft,
  djiProfile: boolean,
  artifactFormat: "wav" | "m4a",
  date = new Date(),
) {
  const year = String(date.getFullYear());
  const month = pad2(date.getMonth() + 1);
  const day = pad2(date.getDate());
  const yymmdd = `${year.slice(-2)}${month}${day}`;
  const yyyymmdd = `${year}${month}${day}`;
  const source = djiProfile
    ? `TX01_MIC001_${yyyymmdd}_120000.WAV`
    : "ZOOM0001.WAV";
  const sourceStem = djiProfile ? `T01_MIC001_${yyyymmdd}_120000` : "ZOOM0001";
  const folders = {
    year_month_day: `${year}/${month}/${day}`,
    year_month: `${year}/${month}`,
    compact_date: yymmdd,
  }[draft.date_folder_layout];
  const directory = draft.archive_directory_name.trim() || "녹음기";
  const fileName = `${yymmdd}-${draft.filename_prefix}${sourceStem}${draft.filename_suffix}.${artifactFormat}`;

  return { source, result: `${directory}/${folders}/${fileName}` };
}

function initialValue(rule: BackupRule | null, initialDraft?: BackupRuleDraft) {
  if (initialDraft) {
    return {
      ...initialDraft,
      required_path_globs: [...initialDraft.required_path_globs],
      backup_file_globs: [...initialDraft.backup_file_globs],
      session_directory_globs: [...initialDraft.session_directory_globs],
    };
  }
  return rule ? draftFromRule(rule) : newDraft();
}

function componentError(value: string, required: boolean) {
  if (required && value.trim().length === 0) return "필수 입력입니다";
  if (!required && value.length === 0) return null;
  if (value.trim().length === 0) return "공백만 입력할 수 없습니다";
  if (Array.from(value).length > 64) return "64자 이하로 입력해 주세요";
  if (value.startsWith(".")) return "점으로 시작할 수 없습니다";
  if (/[\\/\0-\x1f\x7f]/u.test(value))
    return "경로 구분자는 사용할 수 없습니다";
  return null;
}

function validGlob(value: string) {
  if (
    value.trim().length === 0 ||
    Array.from(value).length > 256 ||
    /[\0-\x1f\x7f]/u.test(value)
  ) {
    return false;
  }
  const stack: string[] = [];
  let escaped = false;
  for (const character of value) {
    if (escaped) {
      escaped = false;
      continue;
    }
    if (character === "\\") {
      escaped = true;
    } else if (character === "[" || character === "{") {
      stack.push(character);
    } else if (character === "]") {
      if (stack.pop() !== "[") return false;
    } else if (character === "}") {
      if (stack.pop() !== "{") return false;
    }
  }
  return !escaped && stack.length === 0;
}

function validateDraft(draft: BackupRuleDraft) {
  const errors: Record<string, string> = {};
  const fields: Array<[keyof BackupRuleDraft, boolean]> = [
    ["name", true],
    ["archive_directory_name", true],
    ["filename_prefix", false],
    ["filename_suffix", false],
  ];
  for (const [key, required] of fields) {
    const error = componentError(String(draft[key]), required);
    if (error) errors[key] = error;
  }
  if (!validGlob(draft.volume_name_glob)) {
    errors.volume_name_glob = "올바른 glob 패턴을 입력해 주세요";
  }
  for (const key of Object.keys(patternLabels) as PatternKey[]) {
    const patterns = draft[key];
    if (patterns.length > 32)
      errors[key] = "패턴은 종류별로 32개까지 추가할 수 있습니다";
    if (key === "backup_file_globs" && patterns.length === 0) {
      errors[key] = "백업 파일 glob을 하나 이상 추가해 주세요";
    } else if (patterns.some((pattern) => !validGlob(pattern))) {
      errors[key] = "올바른 glob 패턴을 입력해 주세요";
    }
  }
  return errors;
}

export function RuleEditor({
  rule,
  initialDraft,
  artifactFormat,
  connectedVolumeNames,
  busy,
  onCancel,
  onSave,
  onTest,
}: RuleEditorProps) {
  const [draft, setDraft] = useState(() => initialValue(rule, initialDraft));
  const [errors, setErrors] = useState<Record<string, string>>({});
  const [testResult, setTestResult] = useState<RuleTestResult | null>(null);
  const [testError, setTestError] = useState(false);
  const [testing, setTesting] = useState(false);
  const volumeListId = useId();

  useEffect(() => {
    setDraft(initialValue(rule, initialDraft));
    setErrors({});
    setTestResult(null);
    setTestError(false);
  }, [initialDraft, rule]);

  const preview = useMemo(
    () =>
      formatRulePreview(draft, Boolean(rule?.is_dji_preset), artifactFormat),
    [artifactFormat, draft, rule?.is_dji_preset],
  );

  function updateText(
    key:
      | "name"
      | "archive_directory_name"
      | "volume_name_glob"
      | "filename_prefix"
      | "filename_suffix",
    value: string,
  ) {
    setDraft((current) => {
      if (key === "name") {
        const syncArchive =
          (current.archive_directory_name.length === 0 ||
            current.archive_directory_name === current.name);
        return {
          ...current,
          name: value,
          archive_directory_name: syncArchive
            ? value
            : current.archive_directory_name,
        };
      }
      return { ...current, [key]: value };
    });
  }

  function updatePattern(key: PatternKey, index: number, value: string) {
    setDraft((current) => ({
      ...current,
      [key]: current[key].map((pattern, patternIndex) =>
        patternIndex === index ? value : pattern,
      ),
    }));
  }

  function addPattern(key: PatternKey) {
    setDraft((current) =>
      current[key].length >= 32
        ? current
        : { ...current, [key]: [...current[key], ""] },
    );
  }

  function removePattern(key: PatternKey, index: number) {
    setDraft((current) => ({
      ...current,
      [key]: current[key].filter((_, patternIndex) => patternIndex !== index),
    }));
  }

  function validatedDraft() {
    const nextErrors = validateDraft(draft);
    setErrors(nextErrors);
    return Object.keys(nextErrors).length === 0 ? draft : null;
  }

  async function runTest() {
    const valid = validatedDraft();
    if (!valid || testing || busy) return;
    setTesting(true);
    setTestResult(null);
    setTestError(false);
    try {
      setTestResult(await onTest(valid));
    } catch {
      setTestError(true);
    } finally {
      setTesting(false);
    }
  }

  async function save() {
    const valid = validatedDraft();
    if (!valid || busy) return;
    await onSave(valid);
  }

  return (
    <section className="rule-editor" aria-labelledby="rule-editor-title">
      <div className="rule-editor-heading">
        <div>
          <h3 id="rule-editor-title">
            {rule ? "녹음기 규칙 편집" : "녹음기 규칙 추가"}
          </h3>
          <p>볼륨과 파일을 찾는 glob 규칙을 직접 입력합니다.</p>
        </div>
        <label className="rule-enabled">
          <span>사용</span>
          <Switch
            aria-label="규칙 사용"
            checked={draft.enabled}
            disabled={busy}
            onCheckedChange={(enabled) =>
              setDraft((current) => ({ ...current, enabled }))
            }
          />
        </label>
      </div>

      <div className="rule-editor-grid">
        <RuleInput
          label="규칙 이름"
          value={draft.name}
          error={errors.name}
          disabled={busy}
          onChange={(value) => updateText("name", value)}
        />
        <RuleInput
          label="보관 폴더 이름"
          value={draft.archive_directory_name}
          error={errors.archive_directory_name}
          disabled={busy}
          hint={
            rule?.archive_directory_locked
              ? "새 백업부터 적용됩니다. 검증된 기존 파일은 현재 위치에 유지됩니다."
              : undefined
          }
          onChange={(value) => updateText("archive_directory_name", value)}
        />
        <label className="rule-input">
          <span>날짜 폴더 구조</span>
          <select
            aria-label="날짜 폴더 구조"
            value={draft.date_folder_layout}
            disabled={busy}
            onChange={(event) => {
              const layout = event.currentTarget
                .value as BackupRuleDraft["date_folder_layout"];
              setDraft((current) => ({
                ...current,
                date_folder_layout: layout,
              }));
            }}
          >
            {dateFolderLayouts.map(({ value, label }) => (
              <option key={value} value={value}>
                {label}
              </option>
            ))}
          </select>
        </label>
        <RuleInput
          label="볼륨 이름 glob"
          value={draft.volume_name_glob}
          error={errors.volume_name_glob}
          disabled={busy}
          list={volumeListId}
          onChange={(value) => updateText("volume_name_glob", value)}
        />
        <datalist id={volumeListId}>
          {connectedVolumeNames.map((name) => (
            <option key={name} value={name} />
          ))}
        </datalist>
        <RuleInput
          label="파일명 프리픽스"
          value={draft.filename_prefix}
          error={errors.filename_prefix}
          disabled={busy}
          onChange={(value) => updateText("filename_prefix", value)}
        />
        <RuleInput
          label="파일명 서픽스"
          value={draft.filename_suffix}
          error={errors.filename_suffix}
          disabled={busy}
          onChange={(value) => updateText("filename_suffix", value)}
        />
      </div>

      {(Object.keys(patternLabels) as PatternKey[]).map((key) => (
        <fieldset className="rule-pattern-group" key={key}>
          <legend>{patternLabels[key]}</legend>
          {draft[key].map((pattern, index) => (
            <div className="rule-pattern-row" key={`${key}-${index}`}>
              <label>
                <span className="sr-only">
                  {patternLabels[key]} {index + 1}
                </span>
                <input
                  aria-label={`${patternLabels[key]} ${index + 1}`}
                  value={pattern}
                  disabled={busy}
                  aria-invalid={Boolean(errors[key])}
                  onChange={(event) =>
                    updatePattern(key, index, event.currentTarget.value)
                  }
                />
              </label>
              <Button
                type="button"
                variant="ghost"
                size="icon"
                aria-label={`${patternLabels[key]} ${index + 1} 제거`}
                disabled={busy}
                onClick={() => removePattern(key, index)}
              >
                <Trash2Icon aria-hidden="true" />
              </Button>
            </div>
          ))}
          {errors[key] ? (
            <p className="rule-field-error">{errors[key]}</p>
          ) : null}
          <Button
            type="button"
            variant="outline"
            size="sm"
            disabled={busy || draft[key].length >= 32}
            onClick={() => addPattern(key)}
          >
            <PlusIcon data-icon="inline-start" />
            {patternLabels[key].replace(" glob", "")} 추가
          </Button>
        </fieldset>
      ))}

      <div className="rule-preview" aria-live="polite">
        <span>파일 경로 미리보기</span>
        <div className="rule-preview-flow">
          <div>
            <small>원본 예시</small>
            <code>{preview.source}</code>
          </div>
          <span aria-hidden="true">→</span>
          <div>
            <small>백업 결과</small>
            <code>{preview.result}</code>
          </div>
        </div>
      </div>

      {testResult ? (
        <Alert>
          <AlertTitle>연결된 디스크 테스트 결과</AlertTitle>
          <AlertDescription>
            {testResult.matched_volumes.length > 0 ? (
              <p>
                {testResult.matched_volumes.join(", ")} ·{" "}
                {testResult.matched_file_count}개 파일
              </p>
            ) : (
              <p>현재 일치하는 녹음기가 없습니다.</p>
            )}
            {testResult.conflict_rule_names.length > 0 ? (
              <p>충돌 규칙: {testResult.conflict_rule_names.join(", ")}</p>
            ) : null}
          </AlertDescription>
        </Alert>
      ) : null}

      {testError ? (
        <Alert variant="destructive">
          <AlertTitle>규칙을 테스트하지 못했습니다</AlertTitle>
          <AlertDescription>
            입력한 규칙을 확인한 뒤 다시 시도해 주세요.
          </AlertDescription>
        </Alert>
      ) : null}

      <div className="rule-editor-actions">
        <Button
          type="button"
          variant="outline"
          disabled={busy || testing}
          onClick={onCancel}
        >
          취소
        </Button>
        <Button
          type="button"
          variant="outline"
          disabled={busy || testing}
          onClick={() => void runTest()}
        >
          {testing ? (
            <Spinner data-icon="inline-start" />
          ) : (
            <TestTube2Icon data-icon="inline-start" />
          )}
          연결된 디스크에서 테스트
        </Button>
        <Button
          type="button"
          disabled={busy || testing}
          onClick={() => void save()}
        >
          {busy ? <Spinner data-icon="inline-start" /> : null}
          규칙 저장
        </Button>
      </div>
    </section>
  );
}

function RuleInput({
  label,
  value,
  error,
  disabled,
  hint,
  list,
  onChange,
}: {
  label: string;
  value: string;
  error?: string;
  disabled: boolean;
  hint?: string;
  list?: string;
  onChange: (value: string) => void;
}) {
  return (
    <label className="rule-input">
      <span>{label}</span>
      <input
        aria-label={label}
        value={value}
        list={list}
        disabled={disabled}
        aria-invalid={Boolean(error)}
        onChange={(event) => onChange(event.currentTarget.value)}
      />
      {hint && !error ? <small className="rule-field-hint">{hint}</small> : null}
      {error ? <small className="rule-field-error">{error}</small> : null}
    </label>
  );
}
