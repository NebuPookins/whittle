/**
 * Why the preview could not play.
 *
 * Preview failure is *not* export failure: the trim is a stream copy performed
 * by ffmpeg, which never decodes the video. A file the webview cannot show can
 * still be cut correctly, so these messages say so rather than implying the
 * file is unusable.
 */
export type MediaFailure =
  | { readonly kind: "aborted" }
  | { readonly kind: "network" }
  | { readonly kind: "decode" }
  | { readonly kind: "unreadable" }
  | { readonly kind: "refused"; readonly detail: string };

/**
 * Map an `HTMLMediaElement.error.code` to a failure.
 *
 * The four codes are fixed by the HTML spec; anything else is treated as
 * "unreadable" rather than trusted, since a bogus code tells us nothing.
 */
export function failureFromCode(code: number): MediaFailure {
  switch (code) {
    case 1:
      return { kind: "aborted" };
    case 2:
      return { kind: "network" };
    case 3:
      return { kind: "decode" };
    default:
      return { kind: "unreadable" };
  }
}

/**
 * Whether `value` carries the two fields an `Error`-like object always has.
 *
 * A `play()` rejection is a `DOMException`, which is not a subclass of `Error`
 * in every engine — WebKit in particular does not extend `Error` — so an
 * `instanceof Error` check would let a DOMException's name and message fall
 * through to the "unknown" fallback. Duck-typing the two fields that both
 * `Error` and `DOMException` carry is the only check that holds everywhere.
 */
function isErrorLike(value: unknown): value is { name: string; message: string } {
  return (
    typeof value === "object" &&
    value !== null &&
    typeof (value as { name?: unknown }).name === "string" &&
    typeof (value as { message?: unknown }).message === "string"
  );
}

/**
 * Read a stable name and message out of an arbitrary thrown value.
 *
 * A rejection is not guaranteed to be an `Error`, so callers that log one and
 * those that classify it must agree on what "name" and "detail" mean. This is
 * the one place that decides.
 */
export function errInfo(reason: unknown): { name: string; detail: string } {
  return isErrorLike(reason)
    ? { name: reason.name, detail: reason.message }
    : { name: "unknown", detail: String(reason) };
}

/**
 * Turn a `play()` rejection into a failure to show, or `null` when there is
 * nothing to report.
 *
 * `AbortError` is not a refusal: the browser rejects an in-flight `play()`
 * whenever a pause or a seek supersedes it — the ordinary rhythm of stopping
 * and scrubbing — so surfacing it would pin a "Preview unavailable" overlay
 * over a video that is, in fact, still playing.
 */
export function failureFromPlayRejection(reason: unknown): MediaFailure | null {
  const { name, detail } = errInfo(reason);
  return name === "AbortError" ? null : { kind: "refused", detail };
}

export function describeFailure(failure: MediaFailure): string {
  switch (failure.kind) {
    case "aborted":
      return "Loading the preview was interrupted.";
    case "network":
      return "Reading the file for preview stopped part-way through.";
    case "decode":
      return "This video stream cannot be decoded for preview. Trimming still works — it copies the stream without decoding it.";
    case "unreadable":
      return "The preview could not read this file. Trimming still works — it reads the file with ffmpeg rather than the built-in player.";
    case "refused":
      return `The player refused to start: ${failure.detail}`;
  }
}
