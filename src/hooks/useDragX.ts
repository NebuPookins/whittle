import { useCallback, useRef } from "react";

/**
 * Turn a pointerdown into a horizontal drag that keeps tracking outside the
 * element, and fires once immediately so a plain click also registers.
 */
export function useDragX(
  onDrag: (clientX: number) => void,
): (event: React.PointerEvent) => void {
  // Held in a ref so the listeners always call the latest closure without
  // needing to be torn down and re-attached on every render.
  const latest = useRef(onDrag);
  latest.current = onDrag;

  return useCallback((event: React.PointerEvent) => {
    if (event.button !== 0) return;
    event.preventDefault();
    event.stopPropagation();

    latest.current(event.clientX);

    const move = (e: PointerEvent) => latest.current(e.clientX);
    const stop = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", stop);
      window.removeEventListener("pointercancel", stop);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", stop);
    window.addEventListener("pointercancel", stop);
  }, []);
}
