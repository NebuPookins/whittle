#!/usr/bin/env bash
# Entry point for the desktop launcher: rebuild the checkout if anything
# changed, then run it. If the build fails, fall back to the last successful
# release binary so the launcher still works mid-refactor.
set -uo pipefail

cd "$(dirname "$0")/.."

bin=src-tauri/target/release/whittle
log=src-tauri/target/launch-build.log
mkdir -p src-tauri/target

# `vite build` rewrites dist/ with fresh mtimes, and Tauri embeds dist/ into
# the binary, so running it unconditionally forces a full Rust recompile on
# every launch. Only rebuild the frontend when one of its inputs is newer.
frontend_inputs=(src index.html package.json pnpm-lock.yaml vite.config.ts tsconfig.json)

frontend_stale() {
    [[ ! -f dist/index.html ]] ||
        [[ -n "$(find "${frontend_inputs[@]}" -newer dist/index.html -print -quit 2>/dev/null)" ]]
}

build() {
    if frontend_stale; then
        pnpm build || return 1
    fi
    pnpm tauri build --no-bundle --config '{"build":{"beforeBuildCommand":""}}'
}

if ! build >"$log" 2>&1; then
    if [[ -x "$bin" ]]; then
        notify-send -a Whittle "Whittle build failed" \
            "Launching the last successful build. See $PWD/$log" 2>/dev/null || true
    else
        notify-send -a Whittle -u critical "Whittle build failed" \
            "No previous build to fall back to. See $PWD/$log" 2>/dev/null || true
        exit 1
    fi
fi

exec "$bin" "$@"
