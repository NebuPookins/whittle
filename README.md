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

Stream copies can only *start* on a keyframe, so the in-point is snapped to the
nearest one. Whittle shows exactly where the cut will land before you commit to
it ("Export cuts at 00:01:22.800 — nearest keyframe, 0.14s earlier"), and draws
it on the timeline as a dashed line. Ties snap to the *earlier* keyframe, since
starting early keeps footage and starting late loses it.

Sub-keyframe precision would require re-encoding the boundary GOP. Out of scope.

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
pnpm test                      # 119 frontend tests
cd src-tauri && cargo test     # 43 unit + 5 integration
```

The integration tests encode fixtures and run real trims through ffmpeg,
verifying the extracted range in mp4, mkv and mov. They skip themselves if
ffmpeg is absent. Removing `-seek2any` from `trim_args` makes them fail — the
regression they exist to catch is real.
