import { useCallback, useEffect, useMemo, useReducer, useState } from "react";

import { ActionBar, type ActionHandlers } from "./components/ActionBar";
import { EmptyState } from "./components/EmptyState";
import { SetupScreen } from "./components/SetupScreen";
import { StatusBar } from "./components/StatusBar";
import { Timeline } from "./components/Timeline";
import { Toasts, type Toast } from "./components/Toasts";
import { VideoStage } from "./components/VideoStage";
import {
  canRedo,
  canUndo,
  effectiveCut,
  hasPendingChange,
  nearestWaypoint,
  pendingClip,
  reencodeSpan,
  type WaypointDirection,
} from "./core/clip";
import { editorReducer, initEditor } from "./core/editor";
import { exportModeFor } from "./core/exportMode";
import { basename, splitExtension, suggestExportPath } from "./core/paths";
import { formatBytes, formatSpan } from "./core/timecode";
import { useKeyframeScan } from "./hooks/useKeyframeScan";
import { usePersistedFlag } from "./hooks/usePersistedFlag";
import { useVideoPlayback } from "./hooks/useVideoPlayback";
import {
  checkTools,
  exportTrim,
  logUi,
  onDragDrop,
  pickDestination,
  pickVideo,
  previewUrl,
  probeFile,
  toAppError,
} from "./ipc";
import type { ToolStatus, VideoMetadata } from "./types";

const ZOOM_IN = 1 / 1.6;
const ZOOM_OUT = 1.6;

/**
 * How far the written file may differ from the selected range before it is
 * worth mentioning.
 *
 * Both a floor and a proportion, because a container's timestamp origin adds a
 * small fixed offset — Matroska reports roughly 0.2s more than it holds — and a
 * bare absolute threshold would flag every single mkv export as suspicious.
 */
const DRIFT_FLOOR = 0.5;
const DRIFT_FRACTION = 0.03;

let nextToastId = 1;

export default function App() {
  const [tools, setTools] = useState<ToolStatus | null>(null);
  const [meta, setMeta] = useState<VideoMetadata | null>(null);
  const [opening, setOpening] = useState(false);
  const [exporting, setExporting] = useState(false);
  const [dragOver, setDragOver] = useState(false);
  const [toasts, setToasts] = useState<readonly Toast[]>([]);
  const [editor, dispatch] = useReducer(editorReducer, initEditor(0));
  const [wantsFrameAccurate, setWantsFrameAccurate] = usePersistedFlag(
    "whittle.frameAccurate",
    false,
  );

  const scan = useKeyframeScan(meta);

  const clip = editor.history.clip;
  const pending = useMemo(() => pendingClip(clip, editor.marks), [clip, editor.marks]);
  // Save As writes what is highlighted, so the cut note describes that range.
  // A failed scan leaves a partial keyframe list; snap against it and the
  // reported cut point would be wrong, so treat it as unknown instead.
  const keyframes = useMemo(
    () => (scan.error === null ? scan.keyframes : []),
    [scan.keyframes, scan.error],
  );
  // The toggle is a wish; the backend decides whether it can be granted.
  const mode =
    meta === null
      ? "keyframeCopy"
      : exportModeFor(wantsFrameAccurate, meta.frameAccurate);
  const cut = useMemo(
    () => effectiveCut(pending, keyframes, mode),
    [pending, keyframes, mode],
  );
  const frameRate = meta?.frameRate ?? null;
  const reencode = useMemo(
    () =>
      mode === "frameAccurate" && !scan.scanning && keyframes.length > 0
        ? reencodeSpan(pending, keyframes, frameRate)
        : null,
    [mode, pending, keyframes, scan.scanning, frameRate],
  );
  const src = useMemo(
    () => (meta === null ? null : previewUrl(meta.previewBaseUrl, meta.previewToken)),
    [meta],
  );

  const dismiss = useCallback((id: number) => {
    setToasts((ts) => ts.filter((t) => t.id !== id));
  }, []);

  const notify = useCallback(
    (tone: Toast["tone"], title: string, body: string) => {
      const id = nextToastId++;
      setToasts((ts) => [...ts, { id, tone, title, body }]);
      if (tone === "ok") {
        window.setTimeout(() => dismiss(id), 7000);
      }
    },
    [dismiss],
  );

  const seek = useCallback((time: number) => dispatch({ type: "seek", time }), []);

  const jump = useCallback(
    (direction: WaypointDirection) => {
      const target = nearestWaypoint(
        clip,
        editor.marks,
        editor.keeps,
        editor.playhead,
        direction,
      );
      if (target !== null) seek(target);
    },
    [clip, editor.marks, editor.keeps, editor.playhead, seek],
  );

  const playback = useVideoPlayback(
    clip,
    editor.playhead,
    meta?.frameRate ?? null,
    seek,
  );

  /* ---------- diagnostics ---------- */

  // A CSP refusal never reaches the Rust side: the request is stopped inside
  // the webview, so the protocol handler simply never hears about it. Without
  // this the log would show a src being set and then nothing at all.
  useEffect(() => {
    logUi("ui", `webview userAgent=${navigator.userAgent}`);

    const onViolation = (e: SecurityPolicyViolationEvent) => {
      logUi(
        "csp",
        `blocked ${e.blockedURI} — violated ${e.violatedDirective} (effective: ${e.effectiveDirective})`,
      );
    };
    document.addEventListener("securitypolicyviolation", onViolation);
    return () => document.removeEventListener("securitypolicyviolation", onViolation);
  }, []);

  useEffect(() => {
    if (meta === null || src === null) return;
    logUi("ui", `<video> src set to ${src}`);
  }, [meta, src]);

  /* ---------- tool detection ---------- */

  const detectTools = useCallback(async () => {
    setTools(null);
    try {
      setTools(await checkTools());
    } catch {
      setTools({ ffmpeg: null, ffprobe: null });
    }
  }, []);

  useEffect(() => {
    void detectTools();
  }, [detectTools]);

  /* ---------- opening files ---------- */

  const load = useCallback(
    async (path: string) => {
      setOpening(true);
      try {
        const probed = await probeFile(path);
        setMeta(probed);
        dispatch({ type: "load", duration: probed.duration });
      } catch (e) {
        notify("error", "Could not open that file", toAppError(e).message);
      } finally {
        setOpening(false);
      }
    },
    [notify],
  );

  const browse = useCallback(async () => {
    try {
      const path = await pickVideo();
      if (path !== null) await load(path);
    } catch (e) {
      notify("error", "Could not open the file picker", toAppError(e).message);
    }
  }, [load, notify]);

  useEffect(() => {
    let stop: (() => void) | null = null;
    let disposed = false;

    void onDragDrop((state) => {
      if (state.kind === "hover") {
        setDragOver(true);
        return;
      }
      setDragOver(false);
      if (state.kind === "drop" && state.path !== null) void load(state.path);
    }).then((off) => {
      if (disposed) off();
      else stop = off;
    });

    return () => {
      disposed = true;
      stop?.();
    };
  }, [load]);

  /* ---------- export ---------- */

  const exportClip = useCallback(async () => {
    if (meta === null || exporting) return;

    const { ext } = splitExtension(meta.path);
    try {
      const dest = await pickDestination(
        suggestExportPath(meta.path, cut.start, cut.end),
        ext,
      );
      if (dest === null) return;

      setExporting(true);
      const report = await exportTrim(
        meta.path,
        cut.start,
        cut.end,
        dest,
        mode,
      );
      const drift = report.actualDuration - report.requestedDuration;
      const tolerance = Math.max(
        DRIFT_FLOOR,
        report.requestedDuration * DRIFT_FRACTION,
      );
      const note =
        Math.abs(drift) > tolerance
          ? ` — ${formatSpan(Math.abs(drift))} ${drift > 0 ? "longer" : "shorter"} than the selected range`
          : "";

      notify(
        "ok",
        "Saved",
        `${basename(report.destPath)} — ${formatSpan(report.actualDuration)}, ${formatBytes(report.sizeBytes)}${note}`,
      );
    } catch (e) {
      notify("error", "Export failed", toAppError(e).message);
    } finally {
      setExporting(false);
    }
  }, [meta, exporting, cut, mode, notify]);

  /* ---------- keyboard ---------- */

  const actions: ActionHandlers = useMemo(
    () => ({
      markIn: () => dispatch({ type: "markIn", time: editor.playhead }),
      markOut: () => dispatch({ type: "markOut", time: editor.playhead }),
      markKeep: () => dispatch({ type: "markKeep", time: editor.playhead }),
      commit: () => dispatch({ type: "commit" }),
      clearMarks: () => dispatch({ type: "clearMarks" }),
      undo: () => dispatch({ type: "undo" }),
      redo: () => dispatch({ type: "redo" }),
      zoomIn: () => dispatch({ type: "zoom", factor: ZOOM_IN, focus: editor.playhead }),
      zoomOut: () => dispatch({ type: "zoom", factor: ZOOM_OUT, focus: editor.playhead }),
      fit: () => dispatch({ type: "zoomToFit" }),
      frameSelection: () => dispatch({ type: "zoomToSelection" }),
      exportClip: () => void exportClip(),
    }),
    [editor.playhead, exportClip],
  );

  useEffect(() => {
    if (meta === null) return;

    const onKey = (e: KeyboardEvent) => {
      const key = e.key.toLowerCase();

      if (e.ctrlKey || e.metaKey) {
        if (key === "z") {
          e.preventDefault();
          if (e.shiftKey) actions.redo();
          else actions.undo();
        } else if (key === "y") {
          e.preventDefault();
          actions.redo();
        } else if (key === "s") {
          e.preventDefault();
          actions.exportClip();
        } else if (key === "o") {
          e.preventDefault();
          void browse();
        }
        return;
      }

      if (
        e.target instanceof HTMLInputElement ||
        e.target instanceof HTMLSelectElement ||
        e.target instanceof HTMLTextAreaElement
      ) {
        return;
      }

      switch (e.key) {
        case " ":
          e.preventDefault();
          playback.toggle();
          break;
        case "ArrowLeft":
          e.preventDefault();
          playback.step(e.shiftKey ? -10 : -1);
          break;
        case "ArrowRight":
          e.preventDefault();
          playback.step(e.shiftKey ? 10 : 1);
          break;
        case "Home":
          jump("back");
          break;
        case "End":
          jump("forward");
          break;
        case "Enter":
          // Without this, the browser also synthesises a click on the still-
          // focused button, so Enter would commit *and* fire that button.
          e.preventDefault();
          actions.commit();
          break;
        case "Escape":
          actions.clearMarks();
          break;
        default:
          switch (key) {
            case "i":
              actions.markIn();
              break;
            case "o":
              actions.markOut();
              break;
            case "k":
              actions.markKeep();
              break;
            case "f":
              actions.fit();
              break;
            case "+":
            case "=":
              actions.zoomIn();
              break;
            case "-":
            case "_":
              actions.zoomOut();
              break;
          }
      }
    };

    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [meta, actions, playback, jump, browse]);

  /* ---------- render ---------- */

  if (tools === null) {
    return (
      <div className="app">
        <div className="center">
          <span className="note status__spin">Looking for ffmpeg…</span>
        </div>
      </div>
    );
  }

  if (tools.ffmpeg === null || tools.ffprobe === null) {
    return (
      <div className="app">
        <SetupScreen tools={tools} onRetry={() => void detectTools()} />
      </div>
    );
  }

  return (
    <div className="app">
      <div className="topbar">
        <span className="topbar__title">Whittle</span>
        <button onClick={() => void browse()} title="Open a video (Ctrl+O)">
          Open…
        </button>
        <span className="topbar__file" title={meta?.path ?? ""}>
          {meta === null ? "no file" : basename(meta.path)}
        </span>
        <div className="topbar__spacer" />
        {meta !== null && (
          <span className="note">
            {canUndo(editor.history) ? `${editor.history.past.length} trims` : "untrimmed"}
          </span>
        )}
      </div>

      <div className="stage">
        {meta === null || src === null ? (
          <EmptyState dragOver={dragOver} onOpen={() => void browse()} />
        ) : (
          <VideoStage
            src={src}
            meta={meta}
            clip={clip}
            playhead={editor.playhead}
            playback={playback}
            onJump={jump}
          />
        )}
      </div>

      {meta !== null && (
        <div className="bottom">
          <Timeline
            duration={meta.duration}
            clip={clip}
            view={editor.view}
            marks={editor.marks}
            keeps={editor.keeps}
            pending={pending}
            playhead={editor.playhead}
            keyframes={scan.keyframes}
            effectiveStart={cut.snapped ? cut.start : null}
            onSeek={seek}
            onMarkIn={(time) => dispatch({ type: "markIn", time })}
            onMarkOut={(time) => dispatch({ type: "markOut", time })}
            onZoom={(factor, focus) => dispatch({ type: "zoom", factor, focus })}
            onPan={(delta) => dispatch({ type: "pan", delta })}
          />
          <ActionBar
            clip={clip}
            marks={editor.marks}
            pending={pending}
            cut={cut}
            scanning={scan.scanning}
            keyframesKnown={scan.keyframes.length > 0}
            scanError={scan.error}
            hasPending={hasPendingChange(clip, editor.marks)}
            canUndo={canUndo(editor.history)}
            canRedo={canRedo(editor.history)}
            exporting={exporting}
            mode={mode}
            frameAccurate={meta.frameAccurate}
            onFrameAccurateChange={setWantsFrameAccurate}
            reencode={reencode}
            actions={actions}
          />
          <StatusBar meta={meta} scan={scan} />
        </div>
      )}

      <Toasts toasts={toasts} onDismiss={dismiss} />
      {(exporting || opening) && (
        <div className="busy">
          <span className="status__spin" />
          {exporting ? "Writing the trimmed file…" : "Reading the file…"}
        </div>
      )}
    </div>
  );
}
