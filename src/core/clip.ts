import { keyframeAtOrAfter, keyframeAtOrBefore } from "./keyframes";
import type { ExportMode } from "./exportMode";
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

/** What ffmpeg will really produce, once the start is snapped to a keyframe if need be. */
export interface EffectiveCut {
  readonly start: number;
  readonly end: number;
  /** Snapped start minus requested start; never positive, since snapping only moves earlier. */
  readonly drift: number;
  readonly snapped: boolean;
}

const exactCut = (clip: Clip): EffectiveCut => ({
  start: clip.start,
  end: clip.end,
  drift: 0,
  snapped: false,
});

/**
 * Roughly how many seconds of footage a frame-accurate export re-encodes: the
 * stretch before the first keyframe in the clip, and the one after the last.
 * Everything between those keyframes is copied untouched. A clip with no two
 * keyframes inside it is re-encoded whole.
 *
 * An estimate: it follows `smartcut::plan_cut`, including its half-frame slack
 * (a keyframe within half a frame of an edge counts as on it), but the backend
 * re-encodes the whole range when the copied part turns out to open with
 * leading pictures (open-GOP sources), which only the exported file shows.
 */
export function reencodeSpan(
  clip: Clip,
  keyframes: readonly number[],
  frameRate: number | null,
): number {
  const half = frameRate !== null && frameRate > 0 ? 0.5 / frameRate : 0;
  const first = keyframeAtOrAfter(keyframes, clip.start - half);
  const last = keyframeAtOrBefore(keyframes, clip.end + half);
  if (
    first === null ||
    last === null ||
    last - first < half ||
    first >= clip.end - half
  ) {
    return clip.end - clip.start;
  }
  const head = first - clip.start;
  const tail = clip.end - last;
  return (head >= half ? head : 0) + (tail >= half ? tail : 0);
}

/**
 * Resolve a clip to the range the export will really deliver.
 *
 * A frame-accurate export cuts exactly where the clip was marked. A stream copy
 * can only start on a keyframe, so its start is snapped back to the last
 * keyframe at or before the in-point — never forward, which would drop footage
 * the user asked to keep. With no keyframes known yet, or none before the
 * in-point, the clip passes through unchanged, so the UI can say "still
 * scanning" rather than promise a cut point it cannot honour.
 */
export function effectiveCut(
  clip: Clip,
  keyframes: readonly number[],
  mode: ExportMode,
): EffectiveCut {
  const exact = exactCut(clip);
  if (mode === "frameAccurate") return exact;

  const kf = keyframeAtOrBefore(keyframes, clip.start);
  if (kf === null) return exact;
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

/**
 * Where the playhead lands to probe what can be cut next, like one step of a
 * binary search: the midpoint of the longer end stretch — from the selection's
 * start to its first interior keep pin, or from its last interior pin to its
 * end — since that is where the biggest cut could still be hiding. A tie goes
 * to the front. Pins outside the selection, or on its edges, are ignored.
 */
export function probePoint(selection: Clip, keeps: readonly number[]): number {
  const inside = keeps.filter(
    (k) =>
      k > selection.start &&
      k < selection.end &&
      !sameTime(k, selection.start) &&
      !sameTime(k, selection.end),
  );
  const first = Math.min(selection.end, ...inside);
  const last = Math.max(selection.start, ...inside);
  return first - selection.start >= selection.end - last
    ? (selection.start + first) / 2
    : (last + selection.end) / 2;
}
