import { Spinner } from "@/components/ui/spinner";
import { Switch } from "@/components/ui/switch";

/**
 * The presets the settings window offers.
 *
 * Rust accepts a whole range, so these are suggestions rather than the validation boundary —
 * which is why `SettingChoiceRow` has to cope with a stored value that is not in the list.
 */
export const freeSpaceReserveChoices = [5, 10, 20, 50, 100] as const;

export const rescanIntervalChoices = [
  { value: 5, label: "5초" },
  { value: 15, label: "15초" },
  { value: 30, label: "30초" },
  { value: 60, label: "1분" },
  { value: 300, label: "5분" },
] as const;

/**
 * A settings row whose value is one of a few numbers rather than on/off.
 *
 * A value the app is storing but does not offer — the accepted range is wider than the preset
 * list, and an older build may have written something else — is shown as an extra option, so the
 * control never silently misreports what is in effect.
 */
export function SettingChoiceRow({
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
  value: number;
  choices: ReadonlyArray<{ value: number; label: string }>;
  pending: boolean;
  onChange: (value: number) => void;
}) {
  const options = choices.some((choice) => choice.value === value)
    ? choices
    : [...choices, { value, label: String(value) }].sort(
        (left, right) => left.value - right.value,
      );
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
          onChange={(event) => onChange(Number(event.currentTarget.value))}
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

export function SettingRow({
  id,
  label,
  description,
  checked,
  pending,
  onChange,
}: {
  id: string;
  label: string;
  description: string;
  checked: boolean;
  pending: boolean;
  onChange: (enabled: boolean) => void;
}) {
  return (
    <div className="settings-row">
      <label htmlFor={id}>
        <strong>{label}</strong>
        <span>{description}</span>
      </label>
      <div className="settings-control">
        {pending ? <Spinner aria-label={`${label} 저장 중`} /> : null}
        <Switch
          id={id}
          aria-label={label}
          checked={checked}
          disabled={pending}
          onCheckedChange={onChange}
        />
      </div>
    </div>
  );
}
