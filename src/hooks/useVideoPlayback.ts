import { useCallback, useEffect, useRef, useState } from "react";

import type { Clip } from "../core/clip";
import { errInfo, failureFromCode, failureFromPlayRejection, type MediaFailure } from "../core/media";
import { logUi } from "../ipc";

/**
 * The media element's state machine, as reported to the log.
 *
 * `timeupdate` and `progress` are deliberately absent — they fire constantly
 * and would bury the transitions that actually explain a failure.
 */
const MEDIA_EVENTS = [
  "loadstart",
  "durationchange",
  "loadedmetadata",
  "loadeddata",
  "canplay",
  "canplaythrough",
  "playing",
  "waiting",
  "seeking",
  "seeked",
  "stalled",
  "suspend",
  "abort",
  "emptied",
  "ended",
  "error",
] as const;

/** Below one frame at 25fps — close enough that we must not fight playback. */
const SYNC_EPSILON = 0.04;
const FALLBACK_FPS = 25;

export interface VideoPlayback {
  /** Attach to the `<video>` element. */
  readonly attach: (el: HTMLVideoElement | null) => void;
  readonly playing: boolean;
  readonly toggle: () => void;
  /** Move by whole frames; negative steps back. */
  readonly step: (frames: number) => void;
  /** Why the preview is blank, or `null` while it is fine. */
  readonly failure: MediaFailure | null;
  /** Loudness from 0 (silent) to 1 (full). */
  readonly volume: number;
  /** Whether the element is muted, independent of `volume`. */
  readonly muted: boolean;
  readonly setVolume: (v: number) => void;
  readonly toggleMute: () => void;
  /** Playback multiplier; 1 is real time. */
  readonly rate: number;
  readonly setRate: (r: number) => void;
}

/**
 * Bind a `<video>` element to the editor's playhead.
 *
 * The element's `currentTime` is the source of truth *during playback*, which
 * is followed with rAF rather than `timeupdate` (that only fires ~4×/second
 * and would make the playhead visibly lag). Outside playback the playhead is
 * the user's intent and is pushed into the element, deliberately *not* read
 * back from `seeked` — the browser lands on the nearest decodable frame, and
 * letting that nudge the playhead would blunt the precision of a mark.
 *
 * Playback is confined to `clip`: the discarded parts of the file are not
 * merely hidden on the timeline, they are unreachable.
 */
export function useVideoPlayback(
  clip: Clip,
  playhead: number,
  frameRate: number | null,
  onPosition: (t: number) => void,
): VideoPlayback {
  const [element, setElement] = useState<HTMLVideoElement | null>(null);
  const [playing, setPlaying] = useState(false);
  const [failure, setFailure] = useState<MediaFailure | null>(null);
  const [volume, setVolume] = useState(1);
  const [muted, setMuted] = useState(false);
  const [rate, setRate] = useState(1);

  // Latest values for the rAF loop and callbacks, without re-subscribing.
  const clipRef = useRef(clip);
  clipRef.current = clip;
  const playheadRef = useRef(playhead);
  playheadRef.current = playhead;
  const positionRef = useRef(onPosition);
  positionRef.current = onPosition;

  // Push external seeks into the element.
  useEffect(() => {
    if (element === null) return;
    if (Math.abs(element.currentTime - playhead) > SYNC_EPSILON) {
      element.currentTime = playhead;
    }
  }, [element, playhead]);

  // Apply volume and speed to the element. `defaultPlaybackRate` is set in
  // step with `playbackRate` because the browser resets the rate to it
  // whenever a new source loads; without it the chosen speed would silently
  // fall back to 1× on the next file.
  useEffect(() => {
    if (element === null) return;
    element.volume = volume;
    element.muted = muted;
    element.playbackRate = rate;
    element.defaultPlaybackRate = rate;
  }, [element, volume, muted, rate]);

  // Trace the element's state machine into the log file.
  useEffect(() => {
    if (element === null) return;

    const describe = (type: string): string => {
      const err =
        element.error === null
          ? "none"
          : `${element.error.code}(${element.error.message || "no message"})`;
      return (
        `${type} readyState=${element.readyState} networkState=${element.networkState}` +
        ` error=${err} currentTime=${element.currentTime.toFixed(3)}` +
        ` currentSrc=${element.currentSrc === "" ? "(none)" : element.currentSrc}`
      );
    };

    const detach = MEDIA_EVENTS.map((type) => {
      const handler = () => logUi("video", describe(type));
      element.addEventListener(type, handler);
      return () => element.removeEventListener(type, handler);
    });
    return () => detach.forEach((off) => off());
  }, [element]);

  // A blank preview must say why. Without this the element fails silently and
  // an asset-protocol refusal is indistinguishable from a decode failure.
  useEffect(() => {
    if (element === null) return;

    const report = () => {
      const err = element.error;
      setFailure(err === null ? { kind: "unreadable" } : failureFromCode(err.code));
    };
    // The element may already have failed before this effect attached.
    if (element.error !== null) report();

    const clear = () => setFailure(null);
    element.addEventListener("error", report);
    element.addEventListener("loadstart", clear);
    element.addEventListener("loadeddata", clear);
    return () => {
      element.removeEventListener("error", report);
      element.removeEventListener("loadstart", clear);
      element.removeEventListener("loadeddata", clear);
    };
  }, [element]);

  useEffect(() => {
    if (element === null) return;
    const onPlay = () => setPlaying(true);
    const onStop = () => setPlaying(false);
    element.addEventListener("play", onPlay);
    element.addEventListener("pause", onStop);
    element.addEventListener("ended", onStop);
    return () => {
      element.removeEventListener("play", onPlay);
      element.removeEventListener("pause", onStop);
      element.removeEventListener("ended", onStop);
    };
  }, [element]);

  // Follow playback, and stop at the end of the clip.
  useEffect(() => {
    if (element === null || !playing) return;

    let frame = 0;
    const tick = () => {
      const { start, end } = clipRef.current;
      if (element.currentTime >= end) {
        element.pause();
        element.currentTime = end;
        positionRef.current(end);
      } else if (element.currentTime < start - SYNC_EPSILON) {
        element.currentTime = start;
        positionRef.current(start);
      } else {
        positionRef.current(element.currentTime);
      }
      frame = requestAnimationFrame(tick);
    };
    frame = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(frame);
  }, [element, playing]);

  const toggle = useCallback(() => {
    if (element === null) return;
    if (!element.paused) {
      element.pause();
      return;
    }
    // Replay from the top rather than sitting stuck on the last frame.
    const { start, end } = clipRef.current;
    if (playheadRef.current >= end - 1e-3) {
      element.currentTime = start;
      positionRef.current(start);
    }
    logUi(
      "video",
      `play() called at ${element.currentTime.toFixed(3)}s readyState=${element.readyState}`,
    );
    void element.play().then(
      () => logUi("video", "play() resolved"),
      (reason: unknown) => {
        const { name, detail } = errInfo(reason);
        logUi("video", `play() rejected: ${name}: ${detail}`);
        const refusal = failureFromPlayRejection(reason);
        if (refusal !== null) setFailure(refusal);
      },
    );
  }, [element]);

  const step = useCallback(
    (frames: number) => {
      element?.pause();
      const fps = frameRate !== null && frameRate > 0 ? frameRate : FALLBACK_FPS;
      positionRef.current(playheadRef.current + frames / fps);
    },
    [element, frameRate],
  );

  const changeVolume = (v: number) => {
    setVolume(v);
    // Raising the volume is an implicit unmute; a slider that cannot be heard
    // until a second control is touched would be baffling.
    if (v > 0) setMuted(false);
  };

  const toggleMute = () => setMuted((m) => !m);

  return {
    attach: setElement,
    playing,
    toggle,
    step,
    failure,
    volume,
    muted,
    setVolume: changeVolume,
    toggleMute,
    rate,
    setRate,
  };
}
