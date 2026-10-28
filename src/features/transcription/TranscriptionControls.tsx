import { useEffect, useState } from "react";
import { Spinner } from "@/components/ui/spinner";

/**
 * A settings row whose value is one of a few strings. Like `SettingChoiceRow`, a stored value
 * the list does not offer is shown as an extra option instead of being misreported.
 */
export function TextChoiceRow({
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
 * A short text the user types, such as an address or a model name. It is saved when the field
 * loses focus or Enter is pressed, and only when `isValid` accepts it; anything else puts the
 * stored value back.
 */
export function TextInputRow({
  id,
  label,
  description,
  value,
  placeholder,
  pending,
  isValid,
  onCommit,
}: {
  id: string;
  label: string;
  description: string;
  value: string;
  placeholder?: string;
  pending: boolean;
  isValid: (value: string) => boolean;
  onCommit: (value: string) => void;
}) {
  const [draft, setDraft] = useState(value);
  const [editing, setEditing] = useState(false);

  useEffect(() => {
    if (!editing) setDraft(value);
  }, [editing, value]);

  function commit() {
    setEditing(false);
    const next = draft.trim();
    if (!isValid(next)) {
      setDraft(value);
      return;
    }
    setDraft(next);
    if (next !== value) onCommit(next);
  }

  return (
    <div className="settings-row settings-text-row">
      <label htmlFor={id}>
        <strong>{label}</strong>
        <span>{description}</span>
      </label>
      <div className="settings-control">
        {pending ? <Spinner aria-label={`${label} 저장 중`} /> : null}
        <input
          id={id}
          className="settings-text-input"
          aria-label={label}
          spellCheck={false}
          autoCapitalize="off"
          autoCorrect="off"
          placeholder={placeholder}
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
