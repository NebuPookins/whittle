import { useEffect, useState, type RefObject } from "react";

/** Track an element's rendered width, so canvases can be sized in CSS pixels. */
export function useElementWidth(ref: RefObject<HTMLElement | null>): number {
  const [width, setWidth] = useState(0);

  useEffect(() => {
    const el = ref.current;
    if (el === null) return;

    setWidth(el.getBoundingClientRect().width);
    const observer = new ResizeObserver((entries) => {
      const [entry] = entries;
      if (entry !== undefined) setWidth(entry.contentRect.width);
    });
    observer.observe(el);
    return () => observer.disconnect();
  }, [ref]);

  return width;
}
