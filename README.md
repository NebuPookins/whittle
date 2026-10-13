# Whittle

A desktop tool for narrowing a video down to a target timespan through an
iterative, binary-search-like workflow: preview, mark a cut point, discard
everything before or after it, zoom in, repeat — until the exact segment is
isolated.

Tauri v2 (Rust shell) + React/TypeScript, driving the system `ffmpeg`.

## Requirements

`ffmpeg` and `ffprobe` on your `$PATH`. They are not bundled; Whittle checks on
startup and shows install instructions if either is missing.

```
sudo pacman -S ffmpeg     # Arch
sudo apt install ffmpeg   # Debian/Ubuntu
sudo dnf install ffmpeg   # Fedora
```

Build-time: Rust ≥ 1.77, Node, pnpm, and `webkit2gtk-4.1`.

## Running

```bash
pnpm install
pnpm app          # dev, with hot reload
pnpm app:build    # release bundle (deb + AppImage)
```

## How it narrows

Whittle keeps the original file untouched and tracks the target range purely in
memory. **Discard before / after** set in and out marks; **Commit trim** narrows
the working range to them, which becomes the base for the next round. Undo and
redo step through that history.

Because narrowing is bookkeeping rather than file I/O, every step is instant and
free regardless of whether the source is 40 MB or 40 GB, and undo never re-runs
ffmpeg. A single `ffmpeg` stream copy runs when you **Save As**.

> This is the one deliberate departure from `spec.md`, which had each commit
> shell out to ffmpeg and swap in a new temp file. Since `-c copy` still copies
> every byte of the kept range, that cost gigabytes of writes per commit, several
> times per session, to reach the same result.

### Cut precision

By default a stream copy is used, and those can only *start* on a keyframe, so
the in-point is snapped back to the last keyframe at or before it — never
forward, since starting late would lose footage you marked to keep. Whittle
shows exactly where the cut will land before you commit to it ("Export cuts at
00:01:22.800 — previous keyframe, 0.14s earlier"), and draws it on the timeline
as a dashed line.

### Frame-accurate export

Tick **Frame-accurate** next to Save As to start on exactly the frame you
marked instead. The cut note then reads "Export cuts at 00:01:22.940 —
frame-accurate (re-encodes ~1.1s)", and the dashed line goes away, since there is
nothing left to snap. It is a smart cut: only the stretch before the first
keyframe in the range (the *head*) and after the last (the *tail*) are
re-encoded, with the source's own codec family, profile, pixel format, colour
tags, frame rate and aspect ratio at CRF 18; everything between is still a
stream copy. Audio, subtitles and extra tracks are copied for exactly the
selected range.

It supports H.264 and HEVC only, progressive and unrotated, and only when the
installed ffmpeg has the matching encoder (libx264 / libx265, with the source's
pixel format among its supported ones; checked once per run with
`ffmpeg -encoders` and `-h encoder=…`). If that query itself fails, the file is
treated as supported and the export reports ffmpeg's own error. For anything
else the toggle is disabled, with the backend's reason as its tooltip, and the
backend refuses it too. The estimate in the cut note is just that: a source
with open GOPs (see below) is re-encoded whole.

The pipeline, one ffmpeg per step, all intermediates MPEG-TS in a hidden directory beside the destination
that is removed on success and failure alike (a crash or kill cannot clean up
after itself, so the next frame-accurate export into the same folder removes
any `.whittle-cut-<pid>-…` directory whose process is gone):

```
head   decode [S, K1), re-encode            -> head.ts
body   -seek2any 1 -ss K1 -to <bound> -c copy -bsf:v *_mp4toannexb -> body.ts
tail   decode [K2, E), re-encode            -> tail.ts
join   concat demuxer over head, body, tail  (video)
       + the source's other streams, -ss S -to E -c copy
       -> the destination, container chosen by its extension
```

What was measured, and is why it is built this way (see the doc comments in
`src-tauri/src/smartcut.rs` and the tests in `tests/ffmpeg_roundtrip.rs`):

- **A stream copy's out-point is not exact.** With B-frames, copying to 7.0s
  from a keyframe at 3.2s gave 116 frames where 114 fit, in mp4 and mkv; ending
  on a keyframe still gave two extra. Input `-to` compares decode timestamps,
  which trail display order by the reorder delay. So the body is stopped just
  short of its closing keyframe's dts and the last partial GOP is re-encoded.
  (This also means the plain keyframe-copy export runs a couple of frames long
  at the end on B-frame sources; that is left as it was.)
- **Parameter sets must travel in-band.** The head's SPS/PPS differ from the
  source's whenever the source was encoded with other settings, and an mp4 or
  mkv muxer writes one decoder configuration from the first packets. Joining as
  MPEG-TS with Annex-B bitstreams keeps each piece's own parameter sets at its
  keyframes, and the final mux keeps them in the samples.
- **Open GOPs cannot be spliced.** x265 defaults to them. Leading pictures at a
  copied keyframe reference footage that is not there (`Could not find ref with
  POC`), so when the copied body opens with them the whole range is re-encoded.
  Leading pictures at the *end* keyframe are not in the body at all, so the tail
  starts after the body's last measured frame, and is encoded whenever that
  leaves a gap before the out-point, even when the out-point sits exactly on
  that keyframe (otherwise those pictures were lost: 191 frames where 192 fit).
- **Frames are chosen by raw timestamp, midway between frames.** Every
  alternative misbehaved: on a transport stream (no index) ffmpeg's input
  `-ss` seeks to the keyframe *nearest* the target, not the one before it, and
  its timeline can be off from the container's `start_time` by up to 0.7 of a
  frame; output-side `-ss` gave different frame counts for `0.001` and
  `-0.001`; `-fps_mode cfr` duplicated the first frame when the offset hit half
  a frame; and Matroska's millisecond timestamps (33, 33, 34ms) made x264 emit
  duplicate timestamps. So the re-encodes use `-copyts` and a `trim` filter on
  the raw timestamps, then `setpts` renumbers onto an exact grid.
- **Keyframe lookups need a wide window on index-less containers.** ffprobe's
  `-read_intervals` is meant to open on the keyframe before the interval, but
  an MPEG-TS has no index and the seek is a byte estimate. Measured on a
  15 minute 160x120 TS (`-g 50`, `-output_ts_offset 1440.5`): a one-second
  window ending at the cut opened on a plain packet *after* the time asked for
  and showed no keyframe in 7 of 14 probes (the old code then silently decoded
  from 0.0, the wrong grid phase); windows starting at the time asked for
  always found the right first keyframe in the probes made. So the search
  widens (1s, 4s, 16s… back to the start of the file) until it finds a keyframe
  (last-keyframe lookups) or until the first packet read is decoded no later
  than the time asked for (first-keyframe lookups), and fails loudly rather
  than falling back to 0.0.
- **Audio needs `-copypriorss 0`.** Otherwise a transport stream's audio began
  0.27s early, and an mp4's 85ms early.
- mkv output starts its video ~0.06s in (the B-frame reorder delay, as for any
  mkv), and an MPEG-TS output starts about a second in; both are muxer
  conventions, not offsets in the cut.

Constant frame rate is assumed over the cut: times are snapped to the nearest
frame, so a variable-rate source is cut to within half a frame.

## Keyboard

| Key | Action |
| --- | --- |
| <kbd>Space</kbd> | Play / pause |
| <kbd>←</kbd> <kbd>→</kbd> | Step one frame (<kbd>Shift</kbd> for ten) |
| <kbd>Home</kbd> <kbd>End</kbd> | Jump to start / end of range |
| <kbd>I</kbd> / <kbd>O</kbd> | Discard before / after the playhead |
| <kbd>K</kbd> | Mark keep — pin the playhead as a moment worth keeping |
| <kbd>Enter</kbd> | Commit trim |
| <kbd>Esc</kbd> | Clear marks |
| <kbd>+</kbd> <kbd>−</kbd> <kbd>F</kbd> | Zoom in / out / fit |
| <kbd>Ctrl</kbd>+<kbd>Z</kbd> / <kbd>Shift</kbd>+<kbd>Ctrl</kbd>+<kbd>Z</kbd> | Undo / redo |
| <kbd>Ctrl</kbd>+<kbd>O</kbd> / <kbd>Ctrl</kbd>+<kbd>S</kbd> | Open / Save As |

Scroll to zoom the timeline, <kbd>Shift</kbd>+scroll to pan, drag the markers to
adjust.

## A note on `-seek2any`

The trim command is:

```
ffmpeg -seek2any 1 -ss <keyframe> -to <end> -i <in> -map 0 -c copy <out>
```

(That is the keyframe-copy export; the frame-accurate one reuses the same
`-seek2any`/input-side `-ss`/`-to` for its copied middle.)

Two of those choices are load-bearing and were established by measurement, not
assumption:

- **`-ss`/`-to` before `-i`.** As *input* options, `-to` is absolute on the input
  timeline. Moving it after `-i` makes it relative to the output instead, which
  silently yields a clip of the wrong length (asking for 3.2s–6.4s gave 6.47s).
- **`-seek2any 1`.** Seeking into a Matroska file whose video uses B-frames
  otherwise lands a full GOP early and silently prepends unwanted footage:
  3.2s–6.4s yielded 4.9s starting at 1.6s, with no discard flag to hide it. mp4
  and B-frame-free mkv are unaffected. This is safe only because the in-point is
  snapped to a real keyframe first.

`-ss`/`-to` are 0-based, measured from the container's `start_time`, which ffmpeg
adds itself. An earlier version added it back by hand, which on a transport
stream with `start_time` 1441.9s sought 1441.9s past the end and wrote an empty
file; it is no longer added.

`-avoid_negative_ts make_zero`, a common addition here, was measured to make
things *worse* (it shifts video by the audio priming offset) and is deliberately
absent.

Matroska keeps a small non-zero timestamp origin, so an mkv output reports
~0.2s more duration than the footage it actually contains. The content is
correct — `tests/ffmpeg_roundtrip.rs` asserts on video extent rather than the
container's reported duration for exactly this reason.

## Why preview streams over loopback HTTP

The obvious way to show the file is Tauri's asset protocol via `convertFileSrc`,
but on Linux WebKitGTK cannot source media from *any* custom URI scheme: the
asset protocol and a bespoke `whittle://` scheme both route through
`webkit_web_context_register_uri_scheme`, and its GStreamer media pipeline never
loads audio/video from one (WebKit bug #146351). A `file://` URL is blocked by
the webview for the same reason a page cannot reach arbitrary local files.
Plain HTTP is what GStreamer sources natively, so `preview.rs` runs a tiny
loopback HTTP server bound to `127.0.0.1` and hands the `<video>` element an
`http://127.0.0.1:<port>/<token>` URL.

It streams bodies in bounded chunks — the file may be tens of gigabytes, so no
response is ever built in full in memory — and addresses the file by an opaque
token, so the webview can only ever fetch the file that is currently open, and
a new file gets a new URL rather than a cached one. Binding to `127.0.0.1`
means it is not reachable from another machine.

### The Content-Type is accurate, but advisory

Over HTTP GStreamer type-sniffs the bytes itself, so the `Content-Type` header
no longer decides whether the file plays — it is advisory. `preview.rs` still
sends the accurate type (`mime_for`) rather than guessing from magic bytes, so
an H.264 Matroska file is labelled `video/x-matroska`, not sniffed as
`video/webm`.

Preview failure is not export failure, and the UI says so: trims are stream
copies performed by ffmpeg, which never decodes, so a file the webview cannot
display is still cut correctly.

## Diagnostics

Each launch writes a fresh log to `/tmp/whittle.log` (override with
`$WHITTLE_LOG`), and the path is printed to stderr on startup. Both sides write
to it, so a preview failure reads as one sequence: the container and MIME type
when the file was opened, the `<video>` element's state transitions, every
range request that reached the server and what it answered, and any CSP refusal
— which is otherwise invisible, since the webview blocks those before the Rust
side ever hears about them.

## Layout

```
src/core/        pure logic — timeline math, keyframe lookup, clip history,
                 the editor reducer. No React, fully unit-tested.
src/components/  presentational; the timeline is a canvas + DOM markers.
src/hooks/       side effects — video sync, streaming keyframe scan, sizing.
src-tauri/src/   ffmpeg/ffprobe wrappers. Argument building and output parsing
                 are pure functions, tested separately from the subprocess work.
```

## Tests

```bash
pnpm test                      # 144 frontend tests
cd src-tauri && cargo test     # 65 unit + 15 integration
```

The integration tests encode fixtures and run real trims through ffmpeg,
verifying the extracted range in mp4, mkv and mov, and that frame-accurate cuts
(H.264 and HEVC, open and closed GOP, mp4, mkv and a transport stream with a
non-zero start, multiple audio tracks) hold exactly the requested frames,
compared pixel for pixel against the source, with audio in step and no decode
errors. They skip themselves if
ffmpeg is absent. Removing `-seek2any` from `trim_args` makes them fail — the
regression they exist to catch is real.
