import {
  commit as commitHistory,
  initHistory,
  NO_MARKS,
  pendingClip,
  redo as redoHistory,
  undo as undoHistory,
  type Clip,
  type History,
  type Marks,
} from "./clip";
import { clamp, sameTime } from "./num";
import { clampWindow, panBy, zoomAround, type TimeWindow } from "./timeline";

/**
 * The whole interactive narrowing state.
 *
 * Every transition goes through `editorReducer`, which is what keeps three
 * invariants true by construction:
 *   - `view` always sits inside `history.clip`
 *   - `playhead` always sits inside `history.clip`
 *   - `marks` always sit inside `history.clip`
 */
export interface EditorState {
  readonly history: History;
  readonly marks: Marks;
  /** Keep pins — informational only, never affect the trim or export. */
  readonly keeps: readonly number[];
  readonly view: TimeWindow;
  readonly playhead: number;
}

export type EditorAction =
  | { readonly type: "load"; readonly duration: number }
  | { readonly type: "seek"; readonly time: number }
  | { readonly type: "markIn"; readonly time: number }
  | { readonly type: "markOut"; readonly time: number }
  | { readonly type: "markKeep"; readonly time: number }
  | { readonly type: "clearMarks" }
  | { readonly type: "commit" }
  | { readonly type: "undo" }
  | { readonly type: "redo" }
  | { readonly type: "zoom"; readonly factor: number; readonly focus: number }
  | { readonly type: "zoomToFit" }
  | { readonly type: "zoomToSelection" }
  | { readonly type: "pan"; readonly delta: number };

export function initEditor(duration: number): EditorState {
  const history = initHistory(duration);
  return {
    history,
    marks: NO_MARKS,
    keeps: [],
    view: history.clip,
    playhead: history.clip.start,
  };
}

/** Re-establish the invariants after the clip itself has moved. */
function afterClipChange(
  history: History,
  playhead: number,
  keeps: readonly number[],
): EditorState {
  const clip = history.clip;
  return {
    history,
    // A mark made against the old clip is meaningless against the new one.
    marks: NO_MARKS,
    // A pin outside the new clip can no longer be reached by the playhead to
    // unpin it, so drop it — only pins in the kept region survive.
    keeps: keeps.filter((k) => k >= clip.start && k <= clip.end),
    // Show the new extent in full — this is the "reset the timeline" step.
    view: clip,
    playhead: clamp(playhead, clip.start, clip.end),
  };
}

const inClip = (t: number, clip: Clip): number =>
  Number.isFinite(t) ? clamp(t, clip.start, clip.end) : clip.start;

/**
 * Toggle a keep pin at `time`: drop it if one already sits there, otherwise
 * add it. A second press at the same moment is how a mistaken pin is removed.
 */
function toggleKeep(keeps: readonly number[], time: number): readonly number[] {
  if (!Number.isFinite(time)) return keeps;
  if (keeps.some((k) => sameTime(k, time))) {
    return keeps.filter((k) => !sameTime(k, time));
  }
  return [...keeps, time];
}

/** Leave a little air around the selection so its edges stay grabbable. */
const PADDING = 0.05;

export function editorReducer(
  state: EditorState,
  action: EditorAction,
): EditorState {
  const clip = state.history.clip;

  switch (action.type) {
    case "load":
      return initEditor(action.duration);

    case "seek":
      return { ...state, playhead: inClip(action.time, clip) };

    case "markIn":
      return { ...state, marks: { ...state.marks, inPoint: inClip(action.time, clip) } };

    case "markOut":
      return { ...state, marks: { ...state.marks, outPoint: inClip(action.time, clip) } };

    case "markKeep":
      return {
        ...state,
        keeps: toggleKeep(state.keeps, inClip(action.time, clip)),
      };

    case "clearMarks":
      return { ...state, marks: NO_MARKS };

    case "commit": {
      const next = commitHistory(state.history, pendingClip(clip, state.marks));
      if (next === state.history) return { ...state, marks: NO_MARKS };
      return afterClipChange(next, state.playhead, state.keeps);
    }

    case "undo": {
      const next = undoHistory(state.history);
      return next === state.history ? state : afterClipChange(next, state.playhead, state.keeps);
    }

    case "redo": {
      const next = redoHistory(state.history);
      return next === state.history ? state : afterClipChange(next, state.playhead, state.keeps);
    }

    case "zoom":
      return {
        ...state,
        view: zoomAround(state.view, action.focus, action.factor, clip),
      };

    case "zoomToFit":
      return { ...state, view: clip };

    case "zoomToSelection": {
      const target = pendingClip(clip, state.marks);
      const air = Math.max((target.end - target.start) * PADDING, 0.01);
      return {
        ...state,
        view: clampWindow(
          { start: target.start - air, end: target.end + air },
          clip,
        ),
      };
    }

    case "pan":
      return { ...state, view: panBy(state.view, action.delta, clip) };
  }
}
