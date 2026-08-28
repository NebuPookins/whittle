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
