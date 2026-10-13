import type { Clip, EffectiveCut, Marks } from "../core/clip";
import type { ExportMode } from "../core/exportMode";
import { formatSpan, formatTimecode } from "../core/timecode";
import type { FrameAccurateSupport } from "../types";

export interface ActionHandlers {
  readonly markIn: () => void;
  readonly markOut: () => void;
  readonly markKeep: () => void;
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
  /** The mode Save As will really use — the toggle, unless the file overrides it. */
  readonly mode: ExportMode;
  readonly frameAccurate: FrameAccurateSupport;
  readonly onFrameAccurateChange: (next: boolean) => void;
  /** Estimated seconds a frame-accurate export re-encodes; null until keyframes are known. */
  readonly reencode: number | null;
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
  mode,
  reencode,
  scanning,
  keyframesKnown,
  scanError,
}: {
  cut: EffectiveCut;
  mode: ExportMode;
  reencode: number | null;
  scanning: boolean;
  keyframesKnown: boolean;
  scanError: string | null;
}) {
  if (mode === "frameAccurate") {
    // No snapping to explain: the clip starts on the frame that was marked. All
    // that is left to say is how much of it has to be re-encoded to get there.
    return (
      <span className="note">
        Export cuts at <code>{formatTimecode(cut.start)}</code> — frame-accurate
        {reencode === null
          ? ""
          : reencode > 0
            ? ` (re-encodes ~${formatSpan(reencode)})`
            : " (on keyframes — nothing re-encoded)"}
      </span>
    );
  }
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
  return (
    <span className="note note--warn">
      Export cuts at <code>{formatTimecode(cut.start)}</code> — previous
      keyframe, {formatSpan(Math.abs(cut.drift))} earlier
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
  mode,
  frameAccurate,
  onFrameAccurateChange,
  reencode,
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
        <button
          onClick={actions.markKeep}
          title="Pin the playhead as a moment worth keeping — press again to unpin (K)"
        >
          Mark keep
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
          mode={mode}
          reencode={reencode}
          scanning={scanning}
          keyframesKnown={keyframesKnown}
          scanError={scanError}
        />
        <div className="actions__spacer" />
        <label
          className="toggle"
          title={
            frameAccurate.status === "supported"
              ? "Start on the exact frame instead of the previous keyframe. Re-encodes the few seconds before the first keyframe (and after the last); the rest is copied untouched."
              : `${frameAccurate.reason} Saves cut on keyframes.`
          }
        >
          <input
            type="checkbox"
            checked={mode === "frameAccurate"}
            disabled={frameAccurate.status === "unsupported" || exporting}
            onChange={(e) => onFrameAccurateChange(e.currentTarget.checked)}
          />
          Frame-accurate
        </label>
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
