import { useState } from "react";
import { BookTextIcon, FilePenLineIcon, PlusIcon, XIcon } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Spinner } from "@/components/ui/spinner";
import { Switch } from "@/components/ui/switch";
import { SettingRow } from "../backup/SettingsControls";
import type { DictionaryEntry } from "./contracts";
import {
  DictionaryTextError,
  formatDictionaryText,
  nextDictionaryEntryId,
  parseDictionaryAliases,
  parseDictionaryText,
} from "./dictionary";

type Editor =
  | { kind: "list" }
  | {
      kind: "entry";
      id: string | null;
      term: string;
      aliases: string;
      enabled: boolean;
    }
  | { kind: "text"; text: string; error: string | null };

/**
 * The transcription dictionary, edited the way WakeNote edits its shared Dictionary: entries
 * show as chips that open an editor, and the whole list can be edited as text in the same
 * `Canonical term = aliases` format.
 */
export function DictionarySection({
  enabled,
  entries,
  pending,
  onSave,
}: {
  enabled: boolean;
  entries: DictionaryEntry[];
  pending: boolean;
  onSave: (enabled: boolean, entries: DictionaryEntry[]) => Promise<boolean>;
}) {
  const [editor, setEditor] = useState<Editor>({ kind: "list" });

  async function save(next: DictionaryEntry[]) {
    if (await onSave(enabled, next)) setEditor({ kind: "list" });
  }

  function saveEntry() {
    if (editor.kind !== "entry") return;
    const term = editor.term.trim();
    if (!term) return;
    const entry: DictionaryEntry = {
      id: editor.id ?? nextDictionaryEntryId(entries),
      term,
      aliases: parseDictionaryAliases(editor.aliases, term),
      enabled: editor.enabled,
    };
    void save(
      editor.id
        ? entries.map((current) => (current.id === editor.id ? entry : current))
        : [...entries, entry],
    );
  }

  function applyText() {
    if (editor.kind !== "text") return;
    try {
      void save(parseDictionaryText(editor.text, entries));
    } catch (error) {
      setEditor({
        ...editor,
        error:
          error instanceof DictionaryTextError
            ? error.message
            : "사전을 읽지 못했습니다",
      });
    }
  }

  return (
    <section className="settings-section" aria-labelledby="dictionary-title">
      <div className="settings-section-title">
        <BookTextIcon aria-hidden="true" />
        <div>
          <h2 id="dictionary-title">사전</h2>
          <p>
            자주 나오는 이름과 용어를 정식 표기로 맞춥니다. WakeNote 사전과 같은
            형식입니다.
          </p>
        </div>
      </div>
      <div className="settings-group">
        <SettingRow
          id="dictionary-enabled"
          label="사전 사용"
          description="정식 표기를 Whisper에 힌트로 주고, 별칭은 저장하기 전에 정식 표기로 바꿉니다."
          checked={enabled}
          pending={pending && editor.kind === "list"}
          onChange={(next) => void onSave(next, entries)}
        />

        {editor.kind === "list" ? (
          <div className="dictionary-body">
            <div className="dictionary-toolbar">
              <Button
                variant="outline"
                disabled={pending}
                onClick={() =>
                  setEditor({
                    kind: "entry",
                    id: null,
                    term: "",
                    aliases: "",
                    enabled: true,
                  })
                }
              >
                <PlusIcon data-icon="inline-start" />
                단어 추가
              </Button>
              <Button
                variant="outline"
                disabled={pending}
                onClick={() =>
                  setEditor({
                    kind: "text",
                    text: formatDictionaryText(entries),
                    error: null,
                  })
                }
              >
                <FilePenLineIcon data-icon="inline-start" />
                텍스트로 편집
              </Button>
              <span className="dictionary-count">{entries.length}개</span>
            </div>
            {entries.length > 0 ? (
              <ul className="dictionary-chips" aria-label="사전 단어">
                {entries.map((entry) => (
                  <li
                    key={entry.id}
                    className="dictionary-chip"
                    data-disabled={!entry.enabled || undefined}
                  >
                    <button
                      type="button"
                      className="dictionary-chip-label"
                      aria-label={`${entry.term} 편집`}
                      title={
                        entry.aliases.length
                          ? `별칭: ${entry.aliases.join(", ")}`
                          : undefined
                      }
                      disabled={pending}
                      onClick={() =>
                        setEditor({
                          kind: "entry",
                          id: entry.id,
                          term: entry.term,
                          aliases: entry.aliases.join(", "),
                          enabled: entry.enabled,
                        })
                      }
                    >
                      {entry.term}
                      {entry.aliases.length ? (
                        <span className="dictionary-chip-aliases">
                          +{entry.aliases.length}
                        </span>
                      ) : null}
                    </button>
                    <button
                      type="button"
                      className="dictionary-chip-delete"
                      aria-label={`${entry.term} 삭제`}
                      disabled={pending}
                      onClick={() =>
                        void onSave(
                          enabled,
                          entries.filter(
                            (candidate) => candidate.id !== entry.id,
                          ),
                        )
                      }
                    >
                      <XIcon aria-hidden="true" />
                    </button>
                  </li>
                ))}
              </ul>
            ) : (
              <p className="dictionary-empty">아직 등록한 단어가 없습니다.</p>
            )}
          </div>
        ) : null}

        {editor.kind === "entry" ? (
          <div
            className="dictionary-editor"
            role="group"
            aria-label={editor.id ? "사전 단어 편집" : "사전 단어 추가"}
          >
            <label htmlFor="dictionary-term">
              <strong>정식 표기</strong>
              <span>전사 결과에 남길 최종 표기입니다.</span>
            </label>
            <input
              id="dictionary-term"
              className="settings-text-input"
              aria-label="정식 표기"
              placeholder="Backup Mic"
              value={editor.term}
              disabled={pending}
              autoFocus
              onChange={(event) =>
                setEditor({ ...editor, term: event.currentTarget.value })
              }
            />
            <label htmlFor="dictionary-aliases">
              <strong>별칭</strong>
              <span>
                잘못 들릴 수 있는 표기를 쉼표로 구분해 적습니다. 적은 별칭만
                바꿉니다.
              </span>
            </label>
            <input
              id="dictionary-aliases"
              className="settings-text-input"
              aria-label="별칭"
              placeholder="백업 마이크, backup mike"
              value={editor.aliases}
              disabled={pending}
              onChange={(event) =>
                setEditor({ ...editor, aliases: event.currentTarget.value })
              }
            />
            <div className="dictionary-editor-switch">
              <label htmlFor="dictionary-entry-enabled">
                <strong>사용</strong>
                <span>
                  끄면 단어는 남겨 두고 새 전사에는 적용하지 않습니다.
                </span>
              </label>
              <Switch
                id="dictionary-entry-enabled"
                aria-label="이 단어 사용"
                checked={editor.enabled}
                disabled={pending}
                onCheckedChange={(next) =>
                  setEditor({ ...editor, enabled: next })
                }
              />
            </div>
            <EditorActions
              pending={pending}
              saveLabel="저장"
              saveDisabled={!editor.term.trim()}
              onCancel={() => setEditor({ kind: "list" })}
              onSave={saveEntry}
            />
          </div>
        ) : null}

        {editor.kind === "text" ? (
          <div
            className="dictionary-editor"
            role="group"
            aria-label="사전 텍스트 편집"
          >
            <label htmlFor="dictionary-text">
              <strong>텍스트로 편집</strong>
              <span>
                한 줄에 하나씩 <code>정식 표기 = 별칭, 별칭</code> 형식으로
                적습니다. 빈 줄과 <code>#</code>으로 시작하는 줄은 건너뜁니다.
              </span>
            </label>
            <textarea
              id="dictionary-text"
              className="settings-textarea dictionary-textarea"
              aria-label="사전 텍스트"
              aria-invalid={editor.error ? true : undefined}
              aria-describedby={
                editor.error ? "dictionary-text-error" : undefined
              }
              spellCheck={false}
              rows={10}
              value={editor.text}
              disabled={pending}
              onChange={(event) =>
                setEditor({
                  ...editor,
                  text: event.currentTarget.value,
                  error: null,
                })
              }
            />
            {editor.error ? (
              <p
                id="dictionary-text-error"
                className="dictionary-error"
                role="alert"
              >
                {editor.error}
              </p>
            ) : null}
            <pre className="dictionary-example" aria-label="입력 예시">
              <code>{`# 정식 표기 = 별칭, 별칭
Backup Mic = 백업 마이크, backup mike
DJI Mic Mini = 디제이아이 마이크 미니
WakeNote`}</code>
            </pre>
            <EditorActions
              pending={pending}
              saveLabel="적용"
              saveDisabled={false}
              onCancel={() => setEditor({ kind: "list" })}
              onSave={applyText}
            />
          </div>
        ) : null}
      </div>
    </section>
  );
}

function EditorActions({
  pending,
  saveLabel,
  saveDisabled,
  onCancel,
  onSave,
}: {
  pending: boolean;
  saveLabel: string;
  saveDisabled: boolean;
  onCancel: () => void;
  onSave: () => void;
}) {
  return (
    <div className="dictionary-editor-actions">
      {pending ? <Spinner aria-label="사전 저장 중" /> : null}
      <Button variant="ghost" disabled={pending} onClick={onCancel}>
        취소
      </Button>
      <Button disabled={pending || saveDisabled} onClick={onSave}>
        {saveLabel}
      </Button>
    </div>
  );
}
