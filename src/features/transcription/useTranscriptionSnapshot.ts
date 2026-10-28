import { useCallback, useEffect, useRef, useState } from "react";
import {
  getTranscriptionSnapshot,
  listenForTranscriptionSnapshots,
} from "./client";
import type { TranscriptionSnapshot } from "./contracts";

function messageFrom(error: unknown) {
  if (typeof error === "object" && error !== null && "message_code" in error) {
    return String(error.message_code);
  }
  return "snapshot_unavailable";
}

/** Loads the transcription view and follows Rust's updates, keeping only the newest revision. */
export function useTranscriptionSnapshot() {
  const [snapshot, setSnapshot] = useState<TranscriptionSnapshot | null>(null);
  const [errorCode, setErrorCode] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const mounted = useRef(true);

  const accept = useCallback((candidate: TranscriptionSnapshot) => {
    if (!mounted.current) return;
    setSnapshot((current) =>
      current && current.revision >= candidate.revision ? current : candidate,
    );
    setErrorCode(null);
    setLoading(false);
  }, []);

  const refresh = useCallback(async () => {
    try {
      accept(await getTranscriptionSnapshot());
    } catch (error) {
      if (!mounted.current) return;
      setErrorCode(messageFrom(error));
      setLoading(false);
    }
  }, [accept]);

  useEffect(() => {
    mounted.current = true;
    let unlisten: (() => void) | undefined;
    void listenForTranscriptionSnapshots(accept)
      .then((stop) => {
        if (!mounted.current) {
          stop();
          return;
        }
        unlisten = stop;
        void refresh();
      })
      .catch(() => void refresh());
    return () => {
      mounted.current = false;
      unlisten?.();
    };
  }, [accept, refresh]);

  return { snapshot, loading, errorCode, refresh };
}
