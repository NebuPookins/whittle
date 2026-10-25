# Notes for Claude

- `markIn` / `markOut` / `markKeep` in `src/core/editor.ts` are low-level actions: dragging a mark handle on the Timeline dispatches `markIn`/`markOut` on every mouse move. Put button-level behaviour (auto-zoom, playhead probe jumps, "skip while playing") in App's `actions` handlers, as a sequence of dispatches, not inside those reducer cases. Otherwise the side effects fire during handle drags too.
