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
const STARTING: KeyframeScan = { keyframes: [], scanning: true, error: null };

/** A scan result, tagged with the file it describes. */
interface Owned {
  readonly meta: VideoMetadata | null;
  readonly scan: KeyframeScan;
}

let nextScanId = 1;

/**
 * Stream the file's keyframe list in the background.
 *
 * Results arrive in batches so a long file shows progress instead of freezing,
 * and opening another file supersedes the previous scan rather than letting two
 * ffprobe processes race.
 */
export function useKeyframeScan(meta: VideoMetadata | null): KeyframeScan {
  const [owned, setOwned] = useState<Owned>({ meta: null, scan: IDLE });
  const currentId = useRef(0);

  useEffect(() => {
    if (meta === null) {
      setOwned({ meta, scan: IDLE });
      return;
    }

    const scanId = nextScanId++;
    currentId.current = scanId;
    setOwned({ meta, scan: STARTING });
    const setScan = (update: (prev: KeyframeScan) => KeyframeScan): void =>
      setOwned((prev) => ({ meta, scan: update(prev.scan) }));

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
      setScan(() => ({ keyframes: [], scanning: false, error: String(e) }));
    });

    return () => {
      disposed = true;
      for (const off of offs) off();
      void cancelKeyframeScan(scanId).catch(() => {
        /* the window is going away; nothing useful to do */
      });
    };
  }, [meta]);

  // Until the effect catches up with a newly opened file, the state still
  // describes the previous one; its keyframes must not be snapped against.
  if (owned.meta === meta) return owned.scan;
  return meta === null ? IDLE : STARTING;
}
