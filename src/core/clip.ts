import { keyframeAtOrBefore, nearestKeyframe } from "./keyframes";
import { clamp, sameTime } from "./num";
import type { TimeWindow } from "./timeline";

/** The stretch of the original file currently under consideration. */
export type Clip = TimeWindow;

/** Cut points the user has marked but not yet committed. */
export interface Marks {
  readonly inPoint: number | null;
  readonly outPoint: number | null;
}

export const NO_MARKS: Marks = { inPoint: null, outPoint: null };

/** Narrowing history. Committing is pure bookkeeping — no file is written. */
export interface History {
  readonly clip: Clip;
  readonly past: readonly Clip[];
  readonly future: readonly Clip[];
}

/** Refuse to narrow below this, or the clip stops being previewable. */
export const MIN_CLIP = 0.02;

export function initHistory(duration: number): History {
  const end = Number.isFinite(duration) && duration > 0 ? duration : 0;
  return { clip: { start: 0, end }, past: [], future: [] };
}

export const clipsEqual = (a: Clip, b: Clip): boolean =>
  sameTime(a.start, b.start) && sameTime(a.end, b.end);

/**
 * The clip a commit would produce: marks clamped into the current clip.
 *
 * An unset mark leaves that edge alone, and a range too small to be useful
 * leaves the clip untouched rather than producing something unplayable.
 */
export function pendingClip(clip: Clip, marks: Marks): Clip {
  const a = clamp(marks.inPoint ?? clip.start, clip.start, clip.end);
  const b = clamp(marks.outPoint ?? clip.end, clip.start, clip.end);
  const start = Math.min(a, b);
  const end = Math.max(a, b);
  return end - start >= MIN_CLIP ? { start, end } : clip;
}

export const hasPendingChange = (clip: Clip, marks: Marks): boolean =>
  !clipsEqual(clip, pendingClip(clip, marks));

export function commit(history: History, next: Clip): History {
  if (clipsEqual(history.clip, next)) return history;
  return { clip: next, past: [...history.past, history.clip], future: [] };
}

export function undo(history: History): History {
  const previous = history.past[history.past.length - 1];
  if (previous === undefined) return history;
  return {
    clip: previous,
    past: history.past.slice(0, -1),
    future: [history.clip, ...history.future],
  };
}

export function redo(history: History): History {
  const [next, ...rest] = history.future;
  if (next === undefined) return history;
  return { clip: next, past: [...history.past, history.clip], future: rest };
}

export const canUndo = (h: History): boolean => h.past.length > 0;
export const canRedo = (h: History): boolean => h.future.length > 0;

/** What ffmpeg will really produce, once the start is snapped to a keyframe. */
export interface EffectiveCut {
  readonly start: number;
  readonly end: number;
  /** Snapped start minus requested start. Negative means it starts earlier. */
  readonly drift: number;
  readonly snapped: boolean;
}

/**
 * Resolve a clip to the range a stream copy can actually deliver.
 *
 * With no keyframes known yet the clip passes through unchanged, so the UI can
 * say "still scanning" rather than promise a cut point it cannot honour.
 */
export function effectiveCut(
  clip: Clip,
  keyframes: readonly number[],
): EffectiveCut {
  const exact: EffectiveCut = {
    start: clip.start,
    end: clip.end,
    drift: 0,
    snapped: false,
  };

  const kf = nearestKeyframe(keyframes, clip.start);
  if (kf === null || kf >= clip.end) {
    // `nearestKeyframe` prefers the closer keyframe, which can sit past the
    // end of a sub-GOP clip — snapping there would cut to nothing. Fall back to
    // the last keyframe at or before the in-point, still a valid start.
    const before = keyframeAtOrBefore(keyframes, clip.start);
    if (before === null) return exact;
    return {
      start: before,
      end: clip.end,
      drift: before - clip.start,
      snapped: !sameTime(before, clip.start),
    };
  }

  return {
    start: kf,
    end: clip.end,
    drift: kf - clip.start,
    snapped: !sameTime(kf, clip.start),
  };
}

/** Which way the transport's skip buttons travel. */
export type WaypointDirection = "back" | "forward";

/**
 * Where a skip button lands: the nearest waypoint strictly on the requested
 * side of `from`, or null when there is none (already at that edge).
 *
 * Waypoints are the moments worth hopping between while narrowing — the clip's
 * own edges, the in/out marks, and the keep pins. Marks and pins are clamped
 * into the clip and the edges bound it, so every waypoint sits inside the clip.
 */
export function nearestWaypoint(
  clip: Clip,
  marks: Marks,
  keeps: readonly number[],
  from: number,
  direction: WaypointDirection,
): number | null {
  const back = direction === "back";
  // Skip anything at (or past) the playhead: the "nearest" in a direction is
  // the first strictly-before / strictly-after waypoint, not the current spot.
  const eligible = [
    clip.start,
    clip.end,
    marks.inPoint,
    marks.outPoint,
    ...keeps,
  ].filter((t): t is number => t !== null && (back ? t < from : t > from));

  if (eligible.length === 0) return null;
  return back ? Math.max(...eligible) : Math.min(...eligible);
}
