import { useEffect, useRef, useState } from "react";
import type { UnlistenFn } from "@tauri-apps/api/event";

import { mergeSorted } from "../core/keyframes";
import {
  cancelKeyframeScan,
  onKeyframeBatch,
  onKeyframeDone,
  startKeyframeScan,
} from "../ipc";
import type { VideoMetadata } from "../types";

export interface KeyframeScan {
  readonly keyframes: readonly number[];
  readonly scanning: boolean;
  readonly error: string | null;
}

const IDLE: KeyframeScan = { keyframes: [], scanning: false, error: null };

let nextScanId = 1;

/**
 * Stream the file's keyframe list in the background.
 *
 * Results arrive in batches so a long file shows progress instead of freezing,
 * and opening another file supersedes the previous scan rather than letting two
 * ffprobe processes race.
 */
export function useKeyframeScan(meta: VideoMetadata | null): KeyframeScan {
  const [scan, setScan] = useState<KeyframeScan>(IDLE);
  const currentId = useRef(0);

  useEffect(() => {
    if (meta === null) {
      setScan(IDLE);
      return;
    }

    const scanId = nextScanId++;
    currentId.current = scanId;
    setScan({ keyframes: [], scanning: true, error: null });

    let disposed = false;
    const offs: UnlistenFn[] = [];

    const begin = async (): Promise<void> => {
      // Subscribe before starting, or the first batches can be missed.
      const offBatch = await onKeyframeBatch((e) => {
        if (e.scanId !== currentId.current) return;
        setScan((prev) => ({
          ...prev,
          keyframes: mergeSorted(prev.keyframes, e.times),
        }));
      });
      const offDone = await onKeyframeDone((e) => {
        if (e.scanId !== currentId.current || e.cancelled) return;
        setScan((prev) => ({ ...prev, scanning: false, error: e.error }));
      });

      if (disposed) {
        offBatch();
        offDone();
        return;
      }
      offs.push(offBatch, offDone);
      await startKeyframeScan(meta.path, scanId, meta.startTime);
    };

    begin().catch((e: unknown) => {
      if (disposed) return;
      setScan({ keyframes: [], scanning: false, error: String(e) });
    });

    return () => {
      disposed = true;
      for (const off of offs) off();
      void cancelKeyframeScan(scanId).catch(() => {
        /* the window is going away; nothing useful to do */
      });
    };
  }, [meta]);

  return scan;
}
