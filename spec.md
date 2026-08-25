# Whittle — App Spec

## Overview

A desktop tool for quickly narrowing a video file down to a target timespan through an iterative, binary-search-like workflow: preview, mark a cut point, discard everything before/after it, zoom in, repeat — until the exact segment is isolated.

Target platform: Linux desktop. Built with **Tauri** (Rust shell + system `ffmpeg` binary) and a **TypeScript/React** frontend.

## Goals

- Fast, lossless (stream-copy) trimming — no re-encoding during the interactive narrowing loop.
- Timeline scrubber with click-to-seek.
- "Discard before this point" / "discard after this point" actions that commit a trim and produce a new working file.
- Timeline zoom, so the user can narrow in on smaller and smaller windows as they close in on the target segment.
- Simple, single-purpose UI — not a general video editor.

## Non-goals

- Frame-accurate cutting (cuts snap to the nearest keyframe; see "Cut precision" below).
- Multi-track editing, transitions, effects, audio mixing.
- Cross-platform polish beyond "runs on Linux" (Windows/Mac not a target, though Tauri may make it possible later).

## Tech stack

- **Shell / backend**: Tauri (Rust). Rust code is thin — mostly spawning `ffmpeg` as a subprocess and parsing output.
- **Frontend**: React + TypeScript.
- **Video processing**: system-installed `ffmpeg` / `ffprobe` binaries, invoked via subprocess. Not bundled — assume the user has ffmpeg installed, but check for it on startup and show an error if missing.
- **Video preview**: native HTML `<video>` element pointed at the current working file (via Tauri's asset protocol / a local file URL).

## Core workflow

1. User opens a video file (file picker or drag-and-drop).
2. App loads metadata via `ffprobe` (duration, resolution, codec, keyframe list) and shows the video in a `<video>` preview element with a timeline below it.
3. User scrubs/clicks on the timeline to seek. The `<video>` element's `currentTime` is the source of truth for playback position.
4. User clicks **"Discard before"** or **"Discard after"** at the current playhead position. This does *not* immediately touch the file — it sets an in-memory `inPoint` / `outPoint` marker on the timeline.
5. User can adjust `inPoint`/`outPoint` freely before committing, and can **zoom** the timeline (see below) to click more precisely within a narrowed range.
6. User clicks **"Commit trim"**: the app runs `ffmpeg -ss <inPoint> -to <outPoint> -i <current file> -c copy <new temp file>`, then swaps the working file to the new output and resets the timeline to the new (shorter) duration. This becomes the new base for the next round of narrowing.
7. Repeat steps 3–6, zooming and narrowing, until satisfied.
8. **Export**: user clicks "Save As" to copy the current working file to a final destination chosen by the user.

Undo: keep a stack of previous working-file paths (temp files from each commit) so the user can step back one or more commits.

## Cut precision

`-c copy` trims are stream-copy (no re-encode) and therefore near-instant regardless of file length, which is what makes the interactive loop usable. The tradeoff: cuts can only land on keyframes, not arbitrary frames.

- On file load, fetch the keyframe list via `ffprobe -select_streams v -show_entries frame=pts_time,key_frame -of csv` (or equivalent) and store it.
- When the user picks a cut point, the UI should show the nearest keyframe as the actual commit point — either by snapping the marker visually, or showing "commit will cut at 00:01:22.800 (nearest keyframe)" near the button.
- Do not attempt frame-accurate re-encoding. If the user later wants sub-keyframe precision, that's a possible v2 feature (re-encode just the boundary GOP and concat), explicitly out of scope for v1.

## Timeline component

Custom-built (canvas or absolutely-positioned divs), not a library. Responsibilities:

- Render a horizontal bar representing `[visibleStart, visibleEnd]` of the current working file's duration.
- Click anywhere on the bar → seek `<video>.currentTime` to the corresponding time.
- Show a playhead marker synced to `<video>.currentTime` (via `timeupdate` event).
- Show `inPoint`/`outPoint` markers once set (before commit).
- **Zoom controls**: buttons or scroll-wheel to narrow `[visibleStart, visibleEnd]` around the current playhead. Zoom is pure UI state — a mapping from pixel position to time within the current visible window — and does not touch the underlying file.
- Optionally render keyframe tick marks when zoomed in enough to be useful.

## Rust (Tauri) side

Minimal — mostly command wrappers:

- `probe_file(path) -> VideoMetadata` — shells out to `ffprobe`, returns duration, resolution, codec, keyframe timestamps as JSON.
- `trim_file(input_path, start, end) -> output_path` — shells out to `ffmpeg -ss start -to end -i input -c copy output`, writes to a temp dir, returns the new path. Stream stderr progress back to the frontend via Tauri events if useful (though `-c copy` trims should be fast enough not to need a progress bar).
- `check_ffmpeg_available() -> bool` — run on startup; show a setup-instructions screen if ffmpeg/ffprobe aren't found on `$PATH`.
- `export_file(source_path, dest_path)` — plain file copy to the user's chosen save location.

Use Tauri's file-system and shell plugins (`@tauri-apps/plugin-shell` or direct `std::process::Command`) — no need for anything fancier.

## Frontend state (rough shape)

```ts
interface AppState {
  originalFilePath: string | null;
  workingFilePath: string | null;   // current file after N commits
  fileHistory: string[];            // stack of prior working files, for undo
  duration: number;                 // duration of workingFilePath, seconds
  keyframes: number[];              // seconds
  playheadTime: number;
  inPoint: number | null;
  outPoint: number | null;
  visibleWindow: [number, number];  // for zoom
}
```

## Open questions / decisions for the LLM implementing this

- File format for temp working files: keep original container/codec (safest for `-c copy`) — don't transcode.
- Where to put temp files: a per-session temp dir (e.g. via `std::env::temp_dir()`), cleaned up on app exit or explicitly by the user.
- Should "Commit trim" be reversible without re-running ffmpeg? Since each commit is a full copy operation, undo can just mean "point workingFilePath back at the previous entry in fileHistory" rather than re-running anything.
- Keep the UI to a single view — no multi-clip project management, no timeline of multiple video tracks.
