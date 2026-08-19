import { useCallback, useEffect, useRef, useState } from "react";
import type { BackupActions } from "./client";
import type { AppSnapshot, TrashProposalSummary } from "./contracts";
import { errorCopy } from "./format";

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
  const messageCode = "operation_failed";
  return { messageCode, ...errorCopy(messageCode) };
}

function isEditableTarget(target: EventTarget | null) {
  return (
    target instanceof HTMLElement &&
    (target.matches("input, textarea, select") || target.isContentEditable)
  );
}

/**
 * Runs one popover command at a time and holds the Trash proposal.
 *
 * Serialising on a ref rather than the `pending` state matters: a second click can arrive before
 * React re-renders, and the guard has to see the first one. The proposal is dropped as soon as
 * Rust stops reporting the source as ready, or when it expires, so the confirm dialog can never
 * outlive the authority behind it.
 */
export function usePopoverActions({
  snapshot,
  actions,
  active,
  setupComplete,
}: {
  snapshot: AppSnapshot;
  actions: BackupActions;
  active: boolean;
  setupComplete: boolean;
}) {
  const [pending, setPending] = useState<string | null>(null);
  const [cancelRequested, setCancelRequested] = useState(false);
  const [proposal, setProposal] = useState<TrashProposalSummary | null>(null);
  const [actionError, setActionError] = useState<ActionError | null>(null);
  const pendingRef = useRef<string | null>(null);

  useEffect(() => {
    if (!active) setCancelRequested(false);
  }, [active]);

  useEffect(() => {
    if (!proposal || pending === "confirm-trash") return;
    const source = snapshot.sources.find(({ source_id }) => source_id === proposal.source_id);
    if (!source?.deletion_ready) {
      setProposal(null);
    }
  }, [pending, proposal, snapshot]);

  useEffect(() => {
    if (!proposal) return;
    const remaining = Date.parse(proposal.expires_at) - Date.now();
    if (!Number.isFinite(remaining) || remaining <= 0) {
      setProposal(null);
      return;
    }
    const timeout = window.setTimeout(() => setProposal(null), remaining);
    return () => window.clearTimeout(timeout);
  }, [proposal]);

  const run = useCallback(async (key: string, operation: () => Promise<unknown>) => {
    if (pendingRef.current) return;
    pendingRef.current = key;
    setPending(key);
    setActionError(null);
    try {
      await operation();
    } catch (error) {
      setActionError(commandError(error));
      throw error;
    } finally {
      pendingRef.current = null;
      setPending(null);
    }
  }, []);

  /** Runs a command whose failure the inline error already explains. */
  const runQuietly = useCallback(
    (key: string, operation: () => Promise<unknown>) => {
      void run(key, operation).catch(() => undefined);
    },
    [run],
  );

  const refreshAndBackup = useCallback(async () => {
    try {
      await run("backup", actions.backupNow);
    } catch {
      // The inline error already describes the retry path.
    }
  }, [actions.backupNow, run]);

  const cancelCurrentBackup = useCallback(async () => {
    if (cancelRequested || pendingRef.current) return;
    setCancelRequested(true);
    try {
      await run("cancel", actions.cancelBackup);
    } catch {
      setCancelRequested(false);
    }
  }, [actions.cancelBackup, cancelRequested, run]);

  useEffect(() => {
    function handleKeyDown(event: KeyboardEvent) {
      if (!event.metaKey || event.key.toLowerCase() !== "r" || isEditableTarget(event.target)) {
        return;
      }
      event.preventDefault();
      if (!setupComplete || active || pendingRef.current) return;
      void refreshAndBackup();
    }

    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [active, refreshAndBackup, setupComplete]);

  const prepare = useCallback(
    async (sourceId: string) => {
      try {
        await run(`prepare-${sourceId}`, async () => {
          setProposal(await actions.prepareTrash(sourceId));
        });
      } catch {
        // The inline error already describes the retry path.
      }
    },
    [actions, run],
  );

  const confirm = useCallback(
    async (proposalId: string) => {
      try {
        await run("confirm-trash", () => actions.confirmTrash(proposalId));
        setProposal(null);
      } catch {
        // Keep the confirmation open; Rust has already refused unsafe Trash movement.
      }
    },
    [actions, run],
  );

  const dismissProposal = useCallback(() => {
    if (pendingRef.current === "confirm-trash") return;
    setProposal(null);
    setActionError(null);
  }, []);

  return {
    pending,
    busy: pending !== null,
    cancelRequested,
    proposal,
    actionError,
    run,
    runQuietly,
    refreshAndBackup,
    cancelCurrentBackup,
    prepare,
    confirm,
    dismissProposal,
  };
}
