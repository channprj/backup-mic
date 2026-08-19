import { useEffect, useState } from "react";
import type { BackupActions } from "./client";
import type {
  AppSnapshot,
  BackupRule,
  BackupRuleDraft,
} from "./contracts";
import { errorCopy } from "./format";

type Settings = AppSnapshot["settings"];
type SettingKey = keyof Settings;

export interface ActionError {
  messageCode: string;
  title: string;
  detail: string;
}

function commandError(error: unknown): ActionError {
  if (typeof error === "object" && error !== null && "message_code" in error) {
    const messageCode = String(error.message_code);
    return { messageCode, ...errorCopy(messageCode) };
  }
  const messageCode = "setting_save_failed";
  return { messageCode, ...errorCopy(messageCode) };
}

/**
 * Owns the settings window's view of Rust state and every write it can make.
 *
 * A control moves optimistically so it feels immediate, but the value it settles on always comes
 * from the snapshot Rust returned — and a failure puts the previous value back. `viewSnapshot`
 * only ever moves forward by revision, so a slow reply cannot overwrite newer state that arrived
 * from the snapshot event in the meantime.
 */
export function useSettingsPersistence(
  snapshot: AppSnapshot,
  actions: BackupActions,
) {
  const [viewSnapshot, setViewSnapshot] = useState(snapshot);
  const [pending, setPending] = useState<Set<SettingKey>>(new Set());
  const [destinationPending, setDestinationPending] = useState(false);
  const [destinationStatus, setDestinationStatus] = useState<string | null>(null);
  const [actionError, setActionError] = useState<ActionError | null>(null);
  const [busyRuleId, setBusyRuleId] = useState<string | null>(null);

  useEffect(() => {
    setViewSnapshot((current) =>
      snapshot.revision > current.revision ? snapshot : current,
    );
  }, [snapshot]);

  function markPending(key: SettingKey, value: boolean) {
    setPending((current) => {
      const next = new Set(current);
      if (value) next.add(key);
      else next.delete(key);
      return next;
    });
  }

  async function persistSetting<Key extends SettingKey>(
    key: Key,
    value: Settings[Key],
    operation: () => Promise<AppSnapshot>,
  ) {
    if (pending.has(key)) return;
    const previous = viewSnapshot.settings[key];
    setActionError(null);
    markPending(key, true);
    setViewSnapshot((current) => ({
      ...current,
      settings: { ...current.settings, [key]: value },
    }));
    try {
      const persisted = await operation();
      setViewSnapshot((current) => {
        if (persisted.revision < current.revision) return current;
        return {
          ...persisted,
          artifact_format:
            key === "m4a_conversion"
              ? persisted.artifact_format
              : current.artifact_format,
          retirement_mode:
            key === "automatic_trash"
              ? persisted.retirement_mode
              : current.retirement_mode,
          settings: {
            ...current.settings,
            [key]: persisted.settings[key],
          },
        };
      });
    } catch (error) {
      setViewSnapshot((current) => ({
        ...current,
        settings: { ...current.settings, [key]: previous },
      }));
      setActionError({
        ...commandError(error),
        title: "설정을 저장하지 못했습니다",
      });
    } finally {
      markPending(key, false);
    }
  }

  async function chooseDestination() {
    if (destinationPending) return;
    const previousDisplay = viewSnapshot.destination_display;
    setDestinationPending(true);
    setDestinationStatus(null);
    setActionError(null);
    try {
      const persisted = await actions.chooseDestination();
      setViewSnapshot((current) => ({
        ...persisted,
        artifact_format: current.artifact_format,
        retirement_mode: current.retirement_mode,
        settings: current.settings,
      }));
      if (
        persisted.destination_display &&
        persisted.destination_display !== previousDisplay
      ) {
        setDestinationStatus("백업 폴더가 변경되었습니다");
      }
    } catch (error) {
      setActionError({
        ...commandError(error),
        title: "백업 폴더를 변경하지 못했습니다",
      });
    } finally {
      setDestinationPending(false);
    }
  }

  /** Runs one rule mutation at a time, reporting failure under `title`. */
  async function mutateRule(
    busyId: string,
    title: string,
    operation: () => Promise<AppSnapshot>,
  ): Promise<boolean> {
    if (busyRuleId !== null) return false;
    setBusyRuleId(busyId);
    setActionError(null);
    try {
      setViewSnapshot(await operation());
      return true;
    } catch (error) {
      setActionError({ ...commandError(error), title });
      return false;
    } finally {
      setBusyRuleId(null);
    }
  }

  function saveRule(draft: BackupRuleDraft) {
    return mutateRule(draft.id ?? "new-rule", "규칙을 저장하지 못했습니다", () =>
      actions.saveBackupRule(draft),
    );
  }

  function archiveRule(ruleId: string) {
    return mutateRule(ruleId, "규칙을 보관하지 못했습니다", () =>
      actions.archiveBackupRule(ruleId),
    );
  }

  function restoreDjiRule() {
    const djiRuleId =
      viewSnapshot.backup_rules.find((rule: BackupRule) => rule.is_dji_preset)?.id ??
      "dji-preset";
    return mutateRule(djiRuleId, "DJI 기본 규칙을 복원하지 못했습니다", () =>
      actions.restoreDjiRule(),
    );
  }

  return {
    viewSnapshot,
    pending,
    destinationPending,
    destinationStatus,
    actionError,
    busyRuleId,
    setActionError,
    persistSetting,
    chooseDestination,
    saveRule,
    archiveRule,
    restoreDjiRule,
  };
}
