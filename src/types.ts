/** Mirrors `OpenedFile` (a flattened `probe::VideoMetadata`). Times are 0-based seconds. */
export interface VideoMetadata {
  readonly path: string;
  /** Addresses this file on the loopback preview server. */
  readonly previewToken: number;
  /** Origin of the server streaming the file; append `/{previewToken}`. */
  readonly previewBaseUrl: string;
  readonly duration: number;
  readonly startTime: number;
  readonly width: number;
  readonly height: number;
  readonly videoCodec: string;
  readonly audioCodec: string | null;
  readonly container: string;
  readonly sizeBytes: number;
  readonly frameRate: number | null;
  readonly hasBFrames: boolean;
}

/** Mirrors `ToolStatus`. A `null` version means the tool is not on `$PATH`. */
export interface ToolStatus {
  readonly ffmpeg: string | null;
  readonly ffprobe: string | null;
}

/** Mirrors `trim::ExportReport`. */
export interface ExportReport {
  readonly destPath: string;
  readonly requestedDuration: number;
  readonly actualDuration: number;
  readonly sizeBytes: number;
}

/** Mirrors the `AppError` serialisation. */
export interface AppErrorPayload {
  readonly kind:
    | "toolMissing"
    | "toolFailed"
    | "unparsable"
    | "io"
    | "invalid";
  readonly message: string;
}

export interface KeyframeBatchEvent {
  readonly scanId: number;
  readonly times: readonly number[];
}

export interface KeyframeDoneEvent {
  readonly scanId: number;
  readonly total: number;
  readonly cancelled: boolean;
  readonly error: string | null;
}
