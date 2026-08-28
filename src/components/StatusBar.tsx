import { formatBytes } from "../core/timecode";
import type { KeyframeScan } from "../hooks/useKeyframeScan";
import type { VideoMetadata } from "../types";

interface Props {
  readonly meta: VideoMetadata;
  readonly scan: KeyframeScan;
}

export function StatusBar({ meta, scan }: Props) {
  return (
    <div className="status">
      <span>
        <b>{meta.container.split(",")[0]}</b> container
      </span>
      <span>
        <b>{meta.videoCodec}</b>
        {meta.frameRate !== null ? ` @ ${meta.frameRate.toFixed(3)} fps` : ""}
      </span>
      <span>{formatBytes(meta.sizeBytes)}</span>

      {scan.error !== null ? (
        <span style={{ color: "var(--danger)" }}>
          keyframe scan failed — cuts will not be snapped
        </span>
      ) : scan.scanning ? (
        <span className="status__spin">
          scanning keyframes… <b>{scan.keyframes.length}</b> so far
        </span>
      ) : (
        <span>
          <b>{scan.keyframes.length}</b> keyframes
        </span>
      )}

      {meta.hasBFrames && <span>B-frames</span>}
      {meta.startTime !== 0 && (
        <span>timeline rebased from {meta.startTime.toFixed(3)}s</span>
      )}
    </div>
  );
}
