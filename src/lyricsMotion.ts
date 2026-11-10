export function lyricsScrollTarget(
  itemTop: number,
  itemHeight: number,
  viewportHeight: number,
  scrollHeight: number
) {
  const focusPoint = viewportHeight * 0.46;
  const target = itemTop + itemHeight / 2 - focusPoint;
  return Math.max(0, Math.min(target, Math.max(0, scrollHeight - viewportHeight)));
}

export function lyricsScrollDuration(distance: number) {
  return Math.min(820, Math.max(680, 680 + Math.abs(distance) * 0.12));
}

export function easeLyricsScroll(progress: number) {
  const clamped = Math.max(0, Math.min(1, progress));
  return 1 - Math.pow(1 - clamped, 3);
}
