import { useCallback, useEffect, useRef, useState } from "react";
import { getAppSnapshot, listenForSnapshots } from "./client";
import type { AppSnapshot } from "./contracts";

function messageFrom(error: unknown) {
  if (typeof error === "object" && error !== null && "message_code" in error) {
    return String(error.message_code);
  }
  return "snapshot_unavailable";
}

export function useBackupSnapshot() {
  const [snapshot, setSnapshot] = useState<AppSnapshot | null>(null);
  const [errorCode, setErrorCode] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const mounted = useRef(true);

  const accept = useCallback((candidate: AppSnapshot) => {
    if (!mounted.current) return;
    setSnapshot((current) =>
      current && current.revision >= candidate.revision ? current : candidate,
    );
    setErrorCode(null);
    setLoading(false);
  }, []);

  const refresh = useCallback(async () => {
    try {
      accept(await getAppSnapshot());
    } catch (error) {
      if (!mounted.current) return;
      setErrorCode(messageFrom(error));
      setLoading(false);
    }
  }, [accept]);

  useEffect(() => {
    mounted.current = true;
    let unlisten: (() => void) | undefined;
    void listenForSnapshots(accept)
      .then((stop) => {
        if (!mounted.current) {
          stop();
          return;
        }
        unlisten = stop;
        void refresh();
      })
      .catch(() => void refresh());

    const onFocus = () => void refresh();
    const onVisibility = () => {
      if (document.visibilityState === "visible") void refresh();
    };
    window.addEventListener("focus", onFocus);
    document.addEventListener("visibilitychange", onVisibility);
    return () => {
      mounted.current = false;
      unlisten?.();
      window.removeEventListener("focus", onFocus);
      document.removeEventListener("visibilitychange", onVisibility);
    };
  }, [accept, refresh]);

  return { snapshot, loading, errorCode, refresh };
}
