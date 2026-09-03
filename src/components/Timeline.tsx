import { useCallback, useEffect, useRef } from "react";

import type { Clip, Marks } from "../core/clip";
import { keyframesInRange } from "../core/keyframes";
import { clamp } from "../core/num";
import { formatTickLabel, formatSpan, formatTimecode } from "../core/timecode";
import {
  chooseTickStep,
  generateTicks,
  span,
  timeToX,
  xToTime,
  type TimeWindow,
} from "../core/timeline";
import { useDragX } from "../hooks/useDragX";
import { useElementWidth } from "../hooks/useElementWidth";

/** Mirrors the palette in styles.css; canvas needs literal colours. */
const C = {
  base: "#1c232e",
  discarded: "#12171e",
  selection: "rgba(245, 166, 35, 0.13)",
  grid: "#232c38",
  rulerTick: "#3a4655",
  rulerText: "#6d7b8a",
  keyframe: "#3f4d5e",
  effective: "#f5a623",
  clipBand: "rgba(245, 166, 35, 0.55)",
  clipBandEdge: "#f5a623",
  viewBox: "rgba(88, 166, 255, 0.85)",
  playhead: "#58a6ff",
  keep: "#3fb950",
} as const;

const RULER_H = 20;
const KF_BAND = 13;
const ZOOM_STEP = 1.18;

const crisp = (x: number): number => Math.round(x) + 0.5;

function drawMain(
  ctx: CanvasRenderingContext2D,
  w: number,
  h: number,
  view: TimeWindow,
  pending: Clip,
  keyframes: readonly number[],
  effectiveStart: number | null,
): void {
  ctx.clearRect(0, 0, w, h);
  const x = (t: number) => timeToX(t, view, w);
  const bodyTop = RULER_H;
  const bodyH = h - RULER_H;

  // Selected range stays lit; everything a commit would throw away goes dark.
  const sx = clamp(x(pending.start), 0, w);
  const ex = clamp(x(pending.end), 0, w);
  ctx.fillStyle = C.discarded;
  ctx.fillRect(0, bodyTop, w, bodyH);
  ctx.fillStyle = C.base;
  ctx.fillRect(sx, bodyTop, ex - sx, bodyH);
  ctx.fillStyle = C.selection;
  ctx.fillRect(sx, bodyTop, ex - sx, bodyH);

  // Ruler.
  const step = chooseTickStep(span(view), w);
  const ticks = generateTicks(view, step);
  ctx.font = '10px ui-monospace, "SF Mono", Menlo, monospace';
  ctx.textBaseline = "top";
  for (const t of ticks) {
    const tx = crisp(x(t));
    ctx.fillStyle = C.grid;
    ctx.fillRect(tx, bodyTop, 1, bodyH);
    ctx.fillStyle = C.rulerTick;
    ctx.fillRect(tx, RULER_H - 5, 1, 5);

    const label = formatTickLabel(t, step);
    if (tx + 4 + ctx.measureText(label).width < w) {
      ctx.fillStyle = C.rulerText;
      ctx.fillText(label, tx + 4, 4);
    }
  }

  // Keyframe ticks, but only while they still read as distinct marks.
  const kfs = keyframesInRange(keyframes, view.start, view.end);
  if (kfs.length > 0 && kfs.length <= w / 3) {
    ctx.fillStyle = C.keyframe;
    for (const t of kfs) ctx.fillRect(crisp(x(t)), h - KF_BAND, 1, KF_BAND);
  }

  // Where a stream copy will really begin.
  if (effectiveStart !== null) {
    const ex2 = crisp(x(effectiveStart));
    ctx.save();
    ctx.setLineDash([3, 3]);
    ctx.strokeStyle = C.effective;
    ctx.lineWidth = 1;
    ctx.beginPath();
    ctx.moveTo(ex2, bodyTop);
    ctx.lineTo(ex2, h);
    ctx.stroke();
    ctx.restore();
  }
}

function drawOverview(
  ctx: CanvasRenderingContext2D,
  w: number,
  h: number,
  duration: number,
  clip: Clip,
  view: TimeWindow,
  keeps: readonly number[],
  playhead: number,
): void {
  ctx.clearRect(0, 0, w, h);
  const full: TimeWindow = { start: 0, end: duration };
  const x = (t: number) => timeToX(t, full, w);

  ctx.fillStyle = C.discarded;
  ctx.fillRect(0, 0, w, h);

  // The stretch still in play.
  const cs = clamp(x(clip.start), 0, w);
  const ce = clamp(x(clip.end), 0, w);
  ctx.fillStyle = C.clipBand;
  ctx.fillRect(cs, 0, Math.max(ce - cs, 1), h);

  // What the zoomed timeline below is showing.
  if (span(view) < span(clip) - 1e-9) {
    const vs = clamp(x(view.start), 0, w);
    const ve = clamp(x(view.end), 0, w);
    ctx.strokeStyle = C.viewBox;
    ctx.lineWidth = 1;
    ctx.strokeRect(crisp(vs), 0.5, Math.max(ve - vs, 1), h - 1);
  }

  // Keep pins across the whole file — a glance at where the interesting
  // moments sit is the whole reason to mark them.
  ctx.fillStyle = C.keep;
  for (const t of keeps) ctx.fillRect(crisp(x(t)), 0, 1, 5);

  ctx.fillStyle = C.playhead;
  ctx.fillRect(crisp(x(playhead)), 0, 1, h);
}

/** Size the backing store for the display's pixel ratio, then draw. */
function useCanvas(
  width: number,
  height: number,
  paint: (ctx: CanvasRenderingContext2D, w: number, h: number) => void,
): React.RefObject<HTMLCanvasElement | null> {
  const ref = useRef<HTMLCanvasElement>(null);

  useEffect(() => {
    const canvas = ref.current;
    if (canvas === null || width <= 0) return;
    const dpr = window.devicePixelRatio || 1;
    canvas.width = Math.round(width * dpr);
    canvas.height = Math.round(height * dpr);
    const ctx = canvas.getContext("2d");
    if (ctx === null) return;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    paint(ctx, width, height);
  });

  return ref;
}

interface Props {
  readonly duration: number;
  readonly clip: Clip;
  readonly view: TimeWindow;
  readonly marks: Marks;
  readonly keeps: readonly number[];
  readonly pending: Clip;
  readonly playhead: number;
  readonly keyframes: readonly number[];
  readonly effectiveStart: number | null;
  readonly onSeek: (t: number) => void;
  readonly onMarkIn: (t: number) => void;
  readonly onMarkOut: (t: number) => void;
  readonly onZoom: (factor: number, focus: number) => void;
  readonly onPan: (deltaSeconds: number) => void;
}

export function Timeline({
  duration,
  clip,
  view,
  marks,
  keeps,
  pending,
  playhead,
  keyframes,
  effectiveStart,
  onSeek,
  onMarkIn,
  onMarkOut,
  onZoom,
  onPan,
}: Props) {
  const mainRef = useRef<HTMLDivElement>(null);
  const overviewRef = useRef<HTMLDivElement>(null);
  const width = useElementWidth(mainRef);
  const overviewWidth = useElementWidth(overviewRef);

  const mainCanvas = useCanvas(width, 104, (ctx, w, h) =>
    drawMain(ctx, w, h, view, pending, keyframes, effectiveStart),
  );
  const overviewCanvas = useCanvas(overviewWidth, 34, (ctx, w, h) =>
    drawOverview(ctx, w, h, duration, clip, view, keeps, playhead),
  );

  /** Client X -> time, using the track's own box. */
  const timeAt = useCallback(
    (clientX: number, element: HTMLElement | null, window_: TimeWindow): number => {
      if (element === null) return window_.start;
      const rect = element.getBoundingClientRect();
      return xToTime(clientX - rect.left, window_, rect.width);
    },
    [],
  );

  const scrub = useDragX((clientX) =>
    onSeek(timeAt(clientX, mainRef.current, view)),
  );
  const dragIn = useDragX((clientX) =>
    onMarkIn(timeAt(clientX, mainRef.current, view)),
  );
  const dragOut = useDragX((clientX) =>
    onMarkOut(timeAt(clientX, mainRef.current, view)),
  );
  const overviewSeek = useDragX((clientX) =>
    onSeek(timeAt(clientX, overviewRef.current, { start: 0, end: duration })),
  );

  // Registered natively: a passive listener could not preventDefault, and the
  // page would scroll instead of zooming.
  useEffect(() => {
    const el = mainRef.current;
    if (el === null) return;

    const onWheel = (e: WheelEvent) => {
      e.preventDefault();
      const rect = el.getBoundingClientRect();
      const focus = xToTime(e.clientX - rect.left, view, rect.width);
      if (e.shiftKey) {
        onPan((e.deltaY / rect.width) * span(view));
      } else {
        onZoom(e.deltaY > 0 ? ZOOM_STEP : 1 / ZOOM_STEP, focus);
      }
    };

    el.addEventListener("wheel", onWheel, { passive: false });
    return () => el.removeEventListener("wheel", onWheel);
  }, [view, onZoom, onPan]);

  const pct = (t: number): string =>
    `${(clamp(timeToX(t, view, 100), -20, 120)).toFixed(4)}%`;

  const visible = (t: number): boolean => t >= view.start && t <= view.end;

  return (
    <div className="tl">
      <div className="tl__label">
        <span>Whole file</span>
        <em>{formatTimecode(duration)}</em>
      </div>
      <div
        className="track track--overview"
        ref={overviewRef}
        onPointerDown={overviewSeek}
        title="The lit band is the range still in play"
      >
        <canvas ref={overviewCanvas} />
      </div>

      <div className="tl__label" style={{ marginTop: 10 }}>
        <span>Timeline</span>
        <em>
          showing {formatSpan(span(view))} of {formatSpan(span(clip))}
        </em>
      </div>
      <div
        className="track track--main"
        ref={mainRef}
        onPointerDown={scrub}
        role="slider"
        aria-label="Playhead position"
        aria-valuemin={clip.start}
        aria-valuemax={clip.end}
        aria-valuenow={playhead}
        aria-valuetext={formatTimecode(playhead)}
        tabIndex={0}
      >
        <canvas ref={mainCanvas} />

        {marks.inPoint !== null && visible(marks.inPoint) && (
          <div className="marker marker--in" style={{ left: pct(marks.inPoint) }}>
            <div className="marker__line" />
            <div
              className="marker__grip"
              onPointerDown={dragIn}
              title={`In point ${formatTimecode(marks.inPoint)} — drag to adjust`}
            >
              ⟩
            </div>
          </div>
        )}

        {marks.outPoint !== null && visible(marks.outPoint) && (
          <div className="marker marker--out" style={{ left: pct(marks.outPoint) }}>
            <div className="marker__line" />
            <div
              className="marker__grip"
              onPointerDown={dragOut}
              title={`Out point ${formatTimecode(marks.outPoint)} — drag to adjust`}
            >
              ⟨
            </div>
          </div>
        )}

        {keeps.filter(visible).map((t) => (
          <div key={t} className="marker marker--keep" style={{ left: pct(t) }}>
            <div className="marker__line" />
            <div className="marker__flag" />
          </div>
        ))}

        {visible(playhead) && (
          <div className="marker marker--playhead" style={{ left: pct(playhead) }}>
            <div className="marker__line" />
            <div className="marker__grip" onPointerDown={scrub} />
          </div>
        )}
      </div>
    </div>
  );
}
