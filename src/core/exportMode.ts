import type { FrameAccurateSupport } from "../types";

/**
 * How Save As honours the in-point. Mirrors `trim::ExportMode`.
 *
 * - `keyframeCopy`: a pure stream copy. It can only start on a keyframe, so the
 *   in-point is snapped to one first (see `effectiveCut`).
 * - `frameAccurate`: a smart cut. The partial GOPs at each end are re-encoded
 *   and everything between is copied, so the clip starts and ends on the
 *   exact frames chosen.
 */
export type ExportMode = "keyframeCopy" | "frameAccurate";

/**
 * The mode a Save As will really use. The toggle is a wish; a file the backend
 * cannot cut that way overrides it, so the two can never disagree.
 */
export const exportModeFor = (
  wantsFrameAccurate: boolean,
  support: FrameAccurateSupport,
): ExportMode =>
  wantsFrameAccurate && support.status === "supported"
    ? "frameAccurate"
    : "keyframeCopy";
