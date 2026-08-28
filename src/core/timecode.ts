const pad = (n: number, width: number): string =>
  String(Math.floor(n)).padStart(width, "0");

const toMillis = (seconds: number): number =>
  Number.isFinite(seconds) ? Math.max(0, Math.round(seconds * 1000)) : 0;

/** `HH:MM:SS.mmm`. Non-finite and negative inputs render as zero. */
export function formatTimecode(seconds: number): string {
  const ms = toMillis(seconds);
  return (
    `${pad(ms / 3_600_000, 2)}:` +
    `${pad((ms / 60_000) % 60, 2)}:` +
    `${pad((ms / 1000) % 60, 2)}.` +
    `${pad(ms % 1000, 3)}`
  );
}

/** A compact duration for headers: `4.20s`, `1:23.4`, `1:02:03`. */
export function formatSpan(seconds: number): string {
  const t = Number.isFinite(seconds) ? Math.max(0, seconds) : 0;
  if (t < 10) return `${t.toFixed(2)}s`;
  if (t < 60) return `${t.toFixed(1)}s`;
  const h = Math.floor(t / 3600);
  const m = Math.floor(t / 60) % 60;
  const s = Math.floor(t) % 60;
  return h > 0 ? `${h}:${pad(m, 2)}:${pad(s, 2)}` : `${m}:${pad(s, 2)}`;
}

/**
 * A ruler label carrying only as much precision as `step` warrants, so a
 * zoomed-out timeline reads `12:30` and a zoomed-in one reads `12:30.250`.
 */
export function formatTickLabel(seconds: number, step: number): string {
  const decimals = step >= 1 ? 0 : step >= 0.1 ? 1 : step >= 0.01 ? 2 : 3;
  const scale = 10 ** decimals;
  // Round first, so 59.9999 becomes 1:00 rather than 0:60.
  const t = Math.round(Math.max(0, seconds) * scale) / scale;
  const h = Math.floor(t / 3600);
  const m = Math.floor(t / 60) % 60;
  const s = t % 60;
  const sStr = s.toFixed(decimals).padStart(decimals > 0 ? decimals + 3 : 2, "0");
  return h > 0 ? `${h}:${pad(m, 2)}:${sStr}` : `${m}:${sStr}`;
}

/**
 * Accept `SS`, `MM:SS`, `HH:MM:SS`, each optionally fractional.
 * Returns `null` for anything it cannot read.
 */
export function parseTimecode(text: string): number | null {
  const parts = text.trim().split(":");
  if (parts.length === 0 || parts.length > 3) return null;

  const total = parts.reduce<number | null>((acc, part) => {
    if (acc === null) return null;
    if (!/^\d+(\.\d+)?$|^\.\d+$/.test(part)) return null;
    return acc * 60 + Number(part);
  }, 0);

  return total !== null && Number.isFinite(total) ? total : null;
}

const UNITS = ["B", "KB", "MB", "GB", "TB"] as const;

const scaleBytes = (v: number, i: number): readonly [number, string] =>
  v >= 1024 && i < UNITS.length - 1
    ? scaleBytes(v / 1024, i + 1)
    : [v, UNITS[i] ?? "B"];

export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes < 0) return "—";
  const [v, unit] = scaleBytes(bytes, 0);
  if (unit === "B") return `${Math.round(v)} B`;
  return `${v < 10 ? v.toFixed(2) : v.toFixed(1)} ${unit}`;
}
