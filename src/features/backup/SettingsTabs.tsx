import { useRef, type KeyboardEvent, type ReactNode } from "react";

export interface SettingsTab<Value extends string> {
  value: Value;
  label: string;
  icon: ReactNode;
}

export function settingsTabId(value: string) {
  return `settings-tab-${value}`;
}

export function settingsPanelId(value: string) {
  return `settings-panel-${value}`;
}

/**
 * The settings window's tab strip.
 *
 * Follows the WAI-ARIA tabs pattern: only the selected tab is in the Tab order, and the arrow,
 * Home, and End keys move between tabs and select them.
 */
export function SettingsTabs<Value extends string>({
  label,
  tabs,
  value,
  onValueChange,
}: {
  label: string;
  tabs: ReadonlyArray<SettingsTab<Value>>;
  value: Value;
  onValueChange: (value: Value) => void;
}) {
  const buttons = useRef<Array<HTMLButtonElement | null>>([]);

  function select(index: number) {
    const tab = tabs[(index + tabs.length) % tabs.length];
    onValueChange(tab.value);
    buttons.current[(index + tabs.length) % tabs.length]?.focus();
  }

  function onKeyDown(event: KeyboardEvent<HTMLButtonElement>, index: number) {
    const target = {
      ArrowRight: index + 1,
      ArrowLeft: index - 1,
      Home: 0,
      End: tabs.length - 1,
    }[event.key];
    if (target === undefined) return;
    event.preventDefault();
    select(target);
  }

  return (
    <div role="tablist" aria-label={label} className="settings-tabs">
      {tabs.map((tab, index) => {
        const selected = tab.value === value;
        return (
          <button
            key={tab.value}
            ref={(element) => {
              buttons.current[index] = element;
            }}
            type="button"
            role="tab"
            id={settingsTabId(tab.value)}
            aria-selected={selected}
            aria-controls={settingsPanelId(tab.value)}
            tabIndex={selected ? 0 : -1}
            onClick={() => onValueChange(tab.value)}
            onKeyDown={(event) => onKeyDown(event, index)}
          >
            {tab.icon}
            {tab.label}
          </button>
        );
      })}
    </div>
  );
}
