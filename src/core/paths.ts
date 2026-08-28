import { formatTimecode } from "./timecode";

const lastSeparator = (path: string): number =>
  Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\"));

export function basename(path: string): string {
  const i = lastSeparator(path);
  return i >= 0 ? path.slice(i + 1) : path;
}

export function dirname(path: string): string {
  const i = lastSeparator(path);
  if (i > 0) return path.slice(0, i);
  return i === 0 ? "/" : "";
}

/** Split a filename's stem from its extension. A dotfile has no extension. */
export function splitExtension(path: string): {
  readonly stem: string;
  readonly ext: string;
} {
  const base = basename(path);
  const dot = base.lastIndexOf(".");
  if (dot <= 0) return { stem: base, ext: "" };
  return { stem: base.slice(0, dot), ext: base.slice(dot + 1) };
}

/** `00:01:23.500` -> `00-01-23-500`, dropping a zero millisecond part. */
const stamp = (t: number): string => {
  const [clock = "", millis = ""] = formatTimecode(t).split(".");
  const base = clock.replace(/:/g, "-");
  return millis === "000" ? base : `${base}-${millis}`;
};

/**
 * A destination filename that records the range it came from.
 *
 * The extension is preserved deliberately: a stream copy has to stay in the
 * source container.
 */
export function suggestExportName(
  sourcePath: string,
  start: number,
  end: number,
): string {
  const { stem, ext } = splitExtension(sourcePath);
  const tag = `${stamp(start)} to ${stamp(end)}`;
  return ext ? `${stem} (${tag}).${ext}` : `${stem} (${tag})`;
}

export function suggestExportPath(
  sourcePath: string,
  start: number,
  end: number,
): string {
  const dir = dirname(sourcePath);
  const name = suggestExportName(sourcePath, start, end);
  if (dir === "" ) return name;
  return dir === "/" ? `/${name}` : `${dir}/${name}`;
}
