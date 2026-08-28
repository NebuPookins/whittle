import type { Clip, EffectiveCut, Marks } from "../core/clip";
import { formatSpan, formatTimecode } from "../core/timecode";

export interface ActionHandlers {
  readonly markIn: () => void;
  readonly markOut: () => void;
  readonly commit: () => void;
  readonly clearMarks: () => void;
  readonly undo: () => void;
  readonly redo: () => void;
  readonly zoomIn: () => void;
  readonly zoomOut: () => void;
  readonly fit: () => void;
  readonly frameSelection: () => void;
  readonly exportClip: () => void;
}

interface Props {
  readonly clip: Clip;
  readonly marks: Marks;
  readonly pending: Clip;
  readonly cut: EffectiveCut;
  readonly scanning: boolean;
  readonly keyframesKnown: boolean;
  readonly scanError: string | null;
  readonly hasPending: boolean;
  readonly canUndo: boolean;
  readonly canRedo: boolean;
  readonly exporting: boolean;
  readonly actions: ActionHandlers;
}

/**
 * Describe what an export will actually produce.
 *
 * A stream copy can only start on a keyframe, so the requested in-point and
 * the real one can differ. Saying so plainly beats silently shifting the cut.
 */
function CutNote({
  cut,
  scanning,
  keyframesKnown,
  scanError,
}: {
  cut: EffectiveCut;
  scanning: boolean;
  keyframesKnown: boolean;
  scanError: string | null;
}) {
  if (scanning) {
    return (
      <span className="note status__spin">
        Scanning keyframes — the exact cut point is not settled yet
      </span>
    );
  }
  if (scanError !== null) {
    return (
      <span className="note note--warn">
        Keyframe scan failed — export will cut exactly where marked, which may
        glitch the opening frames
      </span>
    );
  }
  if (!keyframesKnown) {
    return (
      <span className="note note--warn">
        No keyframes found — export will cut exactly where marked, which may
        glitch the opening frames
      </span>
    );
  }
  if (!cut.snapped) {
    return (
      <span className="note">
        Export cuts at <code>{formatTimecode(cut.start)}</code> — already a
        keyframe
      </span>
    );
  }
  const earlier = cut.drift < 0;
  return (
    <span className="note note--warn">
      Export cuts at <code>{formatTimecode(cut.start)}</code> — nearest
      keyframe, {formatSpan(Math.abs(cut.drift))} {earlier ? "earlier" : "later"}
    </span>
  );
}

export function ActionBar({
  clip,
  marks,
  pending,
  cut,
  scanning,
  keyframesKnown,
  scanError,
  hasPending,
  canUndo,
  canRedo,
  exporting,
  actions,
}: Props) {
  const hasMarks = marks.inPoint !== null || marks.outPoint !== null;

  return (
    <>
      <div className="actions">
        <button
          onClick={actions.markIn}
          title="Mark the playhead as the new start, discarding everything before it (I)"
        >
          Discard before
        </button>
        <button
          onClick={actions.markOut}
          title="Mark the playhead as the new end, discarding everything after it (O)"
        >
          Discard after
        </button>

        <div className="actions__sep" />

        <button
          className="primary"
          onClick={actions.commit}
          disabled={!hasPending}
          title="Narrow the working range to the marked selection (Enter)"
        >
          Commit trim
        </button>
        <button
          className="ghost"
          onClick={actions.clearMarks}
          disabled={!hasMarks}
          title="Clear the in and out marks (Esc)"
        >
          Clear marks
        </button>

        <div className="actions__sep" />

        <button onClick={actions.undo} disabled={!canUndo} title="Undo (Ctrl+Z)">
          Undo
        </button>
        <button
          onClick={actions.redo}
          disabled={!canRedo}
          title="Redo (Ctrl+Shift+Z)"
        >
          Redo
        </button>

        <div className="actions__spacer" />

        {hasPending && (
          <span className="note">
            selection {formatSpan(pending.end - pending.start)} of{" "}
            {formatSpan(clip.end - clip.start)}
          </span>
        )}

        <div className="actions__sep" />

        <button className="icon" onClick={actions.zoomOut} title="Zoom out (−)">
          −
        </button>
        <button className="icon" onClick={actions.zoomIn} title="Zoom in (+)">
          +
        </button>
        <button onClick={actions.fit} title="Fit the whole range (F)">
          Fit
        </button>
        <button
          onClick={actions.frameSelection}
          disabled={!hasPending}
          title="Zoom to the marked selection"
        >
          Selection
        </button>
      </div>

      <div className="actions">
        <CutNote
          cut={cut}
          scanning={scanning}
          keyframesKnown={keyframesKnown}
          scanError={scanError}
        />
        <div className="actions__spacer" />
        <button
          className="primary"
          onClick={actions.exportClip}
          disabled={exporting}
          title="Write the current range to a new file (Ctrl+S)"
        >
          {exporting ? "Saving…" : "Save As…"}
        </button>
      </div>
    </>
  );
}
