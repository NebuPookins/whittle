import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { open, save } from "@tauri-apps/plugin-dialog";

import type {
  AppErrorPayload,
  ExportReport,
  KeyframeBatchEvent,
  KeyframeDoneEvent,
  ToolStatus,
  VideoMetadata,
} from "./types";

export const VIDEO_EXTENSIONS = [
  "mp4", "mkv", "mov", "webm", "avi", "m4v", "mpg", "mpeg",
  "ts", "m2ts", "mts", "wmv", "flv", "ogv", "3gp",
] as const;

export const checkTools = (): Promise<ToolStatus> => invoke("check_tools");

export const probeFile = (path: string): Promise<VideoMetadata> =>
  invoke("probe_file", { path });

export const startKeyframeScan = (
  path: string,
  scanId: number,
  startTime: number,
): Promise<void> => invoke("start_keyframe_scan", { path, scanId, startTime });

export const cancelKeyframeScan = (scanId: number): Promise<void> =>
  invoke("cancel_keyframe_scan", { scanId });

export const exportTrim = (
  input: string,
  start: number,
  end: number,
  startTime: number,
  dest: string,
): Promise<ExportReport> =>
  invoke("export_trim", { input, start, end, startTime, dest });

/**
 * A URL the `<video>` element can stream, served by the loopback server in
 * `preview.rs`. It has to be HTTP: WebKitGTK's GStreamer pipeline cannot source
 * media from a custom URI scheme, so the bytes travel as plain HTTP instead.
 */
export const previewUrl = (baseUrl: string, token: number): string =>
  `${baseUrl}/${token}`;

/**
 * Write a line into the app's log file, alongside the Rust side's own entries.
 *
 * Fire-and-forget on purpose: logging must never break, delay, or reorder the
 * behaviour it is there to observe.
 */
export const logUi = (scope: string, message: string): void => {
  void invoke("log_ui", { scope, message }).catch(() => {});
};

export const onKeyframeBatch = (
  handler: (e: KeyframeBatchEvent) => void,
): Promise<UnlistenFn> =>
  listen<KeyframeBatchEvent>("keyframes-batch", (e) => handler(e.payload));

export const onKeyframeDone = (
  handler: (e: KeyframeDoneEvent) => void,
): Promise<UnlistenFn> =>
  listen<KeyframeDoneEvent>("keyframes-done", (e) => handler(e.payload));

export type DropState =
  | { readonly kind: "hover" }
  | { readonly kind: "leave" }
  | { readonly kind: "drop"; readonly path: string | null };

export const onDragDrop = (
  handler: (state: DropState) => void,
): Promise<UnlistenFn> =>
  getCurrentWebview().onDragDropEvent((event) => {
    const payload = event.payload;
    switch (payload.type) {
      case "enter":
      case "over":
        handler({ kind: "hover" });
        return;
      case "leave":
        handler({ kind: "leave" });
        return;
      case "drop":
        handler({ kind: "drop", path: payload.paths[0] ?? null });
        return;
    }
  });

export async function pickVideo(): Promise<string | null> {
  const chosen = await open({
    multiple: false,
    directory: false,
    title: "Open a video",
    filters: [{ name: "Video", extensions: [...VIDEO_EXTENSIONS] }],
  });
  return typeof chosen === "string" ? chosen : null;
}

/**
 * Ask where to write the trim.
 *
 * The filter is pinned to the source's own extension: a stream copy has to
 * stay in the source container.
 */
export async function pickDestination(
  defaultPath: string,
  extension: string,
): Promise<string | null> {
  const chosen = await save({
    defaultPath,
    title: "Save trimmed video",
    filters: extension
      ? [{ name: extension.toUpperCase(), extensions: [extension] }]
      : [],
  });
  return chosen ?? null;
}

const isAppError = (e: unknown): e is AppErrorPayload =>
  typeof e === "object" &&
  e !== null &&
  typeof (e as { kind?: unknown }).kind === "string" &&
  typeof (e as { message?: unknown }).message === "string";

/** Normalise anything a rejected `invoke` might throw into a displayable error. */
export function toAppError(e: unknown): AppErrorPayload {
  if (isAppError(e)) return e;
  if (e instanceof Error) return { kind: "io", message: e.message };
  return { kind: "io", message: String(e) };
}
