import type { Clip, WaypointDirection } from "../core/clip";
import { describeFailure } from "../core/media";
import { adjacentPlaybackRate, PLAYBACK_RATES } from "../core/playbackRate";
import { formatSpan, formatTimecode } from "../core/timecode";
import type { VideoPlayback } from "../hooks/useVideoPlayback";
import type { VideoMetadata } from "../types";

interface Props {
  readonly src: string;
  readonly meta: VideoMetadata;
  readonly clip: Clip;
  readonly playhead: number;
  readonly playback: VideoPlayback;
  readonly onJump: (direction: WaypointDirection) => void;
}

export function VideoStage({
  src,
  meta,
  clip,
  playhead,
  playback,
  onJump,
}: Props) {
  const {
    attach,
    toggle,
    step,
    failure,
    playing,
    volume,
    muted,
    setVolume,
    toggleMute,
    rate,
    setRate,
  } = playback;
  const hasAudio = meta.audioCodec !== null;
  const muteLabel = muted ? "Unmute" : "Mute";
  const slowerRate = adjacentPlaybackRate(rate, "slower");
  const fasterRate = adjacentPlaybackRate(rate, "faster");

  return (
    <>
      <div className="video-wrap">
        {/* Streamed from disk over Tauri's asset protocol. */}
        <video
          ref={attach}
          src={src}
          preload="auto"
          onClick={toggle}
        />
        {failure !== null && (
          <div className="video-error" role="alert">
            <b>Preview unavailable</b>
            <span>{describeFailure(failure)}</span>
          </div>
        )}
        <div className="video-badge">
          {meta.width}×{meta.height} · {meta.videoCodec}
          {hasAudio ? ` + ${meta.audioCodec}` : " · no audio"}
        </div>
      </div>

      <div className="transport">
        <button
          className="icon"
          onClick={() => onJump("back")}
          title="Jump to previous waypoint (Home)"
          aria-label="Jump to previous waypoint"
        >
          ⏮
        </button>
        <button
          className="icon"
          onClick={() => step(-1)}
          title="Back one frame (←, Shift+← for ten)"
          aria-label="Back one frame"
        >
          ◀|
        </button>
        <button
          className="icon"
          onClick={toggle}
          title="Play / pause (Space)"
          aria-label={playing ? "Pause" : "Play"}
        >
          {playing ? "❙❙" : "▶"}
        </button>
        <button
          className="icon"
          onClick={() => step(1)}
          title="Forward one frame (→, Shift+→ for ten)"
          aria-label="Forward one frame"
        >
          |▶
        </button>
        <button
          className="icon"
          onClick={() => onJump("forward")}
          title="Jump to next waypoint (End)"
          aria-label="Jump to next waypoint"
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

        <div className="transport__spacer" />

        <div className="transport__volume">
          <button
            className="icon"
            onClick={toggleMute}
            disabled={!hasAudio}
            title={hasAudio ? muteLabel : "No audio track"}
            aria-label={hasAudio ? muteLabel : "No audio track"}
            aria-pressed={hasAudio ? muted : undefined}
          >
            {muted ? "🔇" : "🔊"}
          </button>
          <input
            type="range"
            min={0}
            max={100}
            step={1}
            value={Math.round(volume * 100)}
            onChange={(e) => setVolume(Number(e.target.value) / 100)}
            disabled={!hasAudio}
            aria-label="Volume"
            title={hasAudio ? "Volume" : "No audio track"}
          />
        </div>

        <div className="transport__speed">
          <button
            className="icon"
            onClick={() => slowerRate !== null && setRate(slowerRate)}
            disabled={slowerRate === null}
            title="Decrease playback speed"
            aria-label="Decrease playback speed"
          >
            −
          </button>
          <select
            value={String(rate)}
            onChange={(e) => {
              const selected = PLAYBACK_RATES.find(
                (option) => String(option) === e.target.value,
              );
              if (selected !== undefined) setRate(selected);
            }}
            aria-label="Playback speed"
            title="Playback speed"
          >
            {PLAYBACK_RATES.map((speed) => (
              <option key={speed} value={String(speed)}>
                {speed}×
              </option>
            ))}
          </select>
          <button
            className="icon"
            onClick={() => fasterRate !== null && setRate(fasterRate)}
            disabled={fasterRate === null}
            title="Increase playback speed"
            aria-label="Increase playback speed"
          >
            +
          </button>
        </div>
      </div>
    </>
  );
}
