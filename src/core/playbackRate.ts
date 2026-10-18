/** Selectable playback speeds, slowest first. */
export const PLAYBACK_RATES = [0.25, 0.5, 1, 1.25, 1.5, 2, 3, 4] as const;

export type PlaybackRate = (typeof PLAYBACK_RATES)[number];
export type PlaybackRateDirection = "slower" | "faster";

/**
 * Returns the adjacent selectable speed in `direction`, or `null` at the end
 * of the range. Keeping the navigation tied to the select's values prevents
 * the buttons and menu from drifting apart.
 */
export function adjacentPlaybackRate(
  rate: PlaybackRate,
  direction: PlaybackRateDirection,
): PlaybackRate | null {
  const index = PLAYBACK_RATES.indexOf(rate);
  const adjacent = direction === "slower" ? index - 1 : index + 1;
  return PLAYBACK_RATES[adjacent] ?? null;
}
