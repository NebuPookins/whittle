import type { Clip } from "../core/clip";
import { describeFailure } from "../core/media";
import { formatSpan, formatTimecode } from "../core/timecode";
import type { VideoPlayback } from "../hooks/useVideoPlayback";
import type { VideoMetadata } from "../types";

interface Props {
  readonly src: string;
  readonly meta: VideoMetadata;
  readonly clip: Clip;
  readonly playhead: number;
  readonly playback: VideoPlayback;
  readonly onJump: (t: number) => void;
}

export function VideoStage({
  src,
  meta,
  clip,
  playhead,
  playback,
  onJump,
}: Props) {
  return (
    <>
      <div className="video-wrap">
        {/* Streamed from disk over Tauri's asset protocol. */}
        <video
          ref={playback.attach}
          src={src}
          preload="auto"
          onClick={playback.toggle}
        />
        {playback.failure !== null && (
          <div className="video-error" role="alert">
            <b>Preview unavailable</b>
            <span>{describeFailure(playback.failure)}</span>
          </div>
        )}
        <div className="video-badge">
          {meta.width}×{meta.height} · {meta.videoCodec}
          {meta.audioCodec !== null ? ` + ${meta.audioCodec}` : " · no audio"}
        </div>
      </div>

      <div className="transport">
        <button
          className="icon"
          onClick={() => onJump(clip.start)}
          title="Jump to start of range (Home)"
          aria-label="Jump to start of range"
        >
          ⏮
        </button>
        <button
          className="icon"
          onClick={() => playback.step(-1)}
          title="Back one frame (←, Shift+← for ten)"
          aria-label="Back one frame"
        >
          ◀|
        </button>
        <button
          className="icon"
          onClick={playback.toggle}
          title="Play / pause (Space)"
          aria-label={playback.playing ? "Pause" : "Play"}
        >
          {playback.playing ? "❙❙" : "▶"}
        </button>
        <button
          className="icon"
          onClick={() => playback.step(1)}
          title="Forward one frame (→, Shift+→ for ten)"
          aria-label="Forward one frame"
        >
          |▶
        </button>
        <button
          className="icon"
          onClick={() => onJump(clip.end)}
          title="Jump to end of range (End)"
          aria-label="Jump to end of range"
        >
          ⏭
        </button>

        <div className="transport__clock" style={{ marginLeft: 6 }}>
          <b>{formatTimecode(playhead)}</b>
          <span>
            {" "}
            · range {formatTimecode(clip.start)}–{formatTimecode(clip.end)} (
            {formatSpan(clip.end - clip.start)})
          </span>
        </div>
      </div>
    </>
  );
}
