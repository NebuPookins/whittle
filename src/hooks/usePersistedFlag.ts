import { useCallback, useState } from "react";

/** Per-viewer convenience only: storage can be blocked or empty. */
function load(key: string, initial: boolean): boolean {
  try {
    const stored = window.localStorage.getItem(key);
    return stored === null ? initial : stored === "1";
  } catch {
    return initial;
  }
}

function save(key: string, value: boolean): void {
  try {
    window.localStorage.setItem(key, value ? "1" : "0");
  } catch {
    // Not persisting the flag is harmless.
  }
}

/** A boolean that survives restarts, falling back to `initial` when it can't. */
export function usePersistedFlag(
  key: string,
  initial: boolean,
): readonly [boolean, (next: boolean) => void] {
  const [value, setValue] = useState(() => load(key, initial));
  const set = useCallback(
    (next: boolean) => {
      setValue(next);
      save(key, next);
    },
    [key],
  );
  return [value, set];
}
