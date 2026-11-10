import type { LyricLine, MediaState } from "./types";

export function extrapolatePosition(media: MediaState | null | undefined, now: number) {
  if (!media) return 0;
  const position = finiteNonnegative(media.positionMs);
  const duration = finiteNonnegative(media.durationMs) || Number.MAX_SAFE_INTEGER;
  const elapsed =
    media.status === "playing" && Number.isFinite(now) && Number.isFinite(media.sampledAtMs)
      ? Math.max(0, now - media.sampledAtMs) * playbackRate(media.playbackRate)
      : 0;
  return Math.min(duration, position + elapsed);
}

const finiteNonnegative = (value: number) => Number.isFinite(value) ? Math.max(0, value) : 0;
const playbackRate = (value: number) => Number.isFinite(value) && value > 0 ? value : 1;

export interface PlaybackAnchor {
  position: number;
  duration: number;
  rate: number;
  playing: boolean;
  monotonicAt: number;
}

export function anchorPlayback(
  media: MediaState | null | undefined,
  wallNow: number,
  monotonicNow: number
): PlaybackAnchor {
  return {
    position: extrapolatePosition(media, wallNow),
    duration: media ? finiteNonnegative(media.durationMs) || Number.MAX_SAFE_INTEGER : 0,
    rate: playbackRate(media?.playbackRate ?? 1),
    playing: media?.status === "playing",
    monotonicAt: monotonicNow
  };
}

/** Between backend samples, wall-clock changes must not make lyrics jump. */
export function extrapolateAnchoredPosition(anchor: PlaybackAnchor, monotonicNow: number): number {
  const elapsed = anchor.playing ? finiteNonnegative(monotonicNow - anchor.monotonicAt) * anchor.rate : 0;
  return Math.min(anchor.duration, anchor.position + elapsed);
}

export function findActiveLine(
  lines: LyricLine[],
  positionMs: number,
  offsetMs = 0
): LyricLine | null {
  if (!lines.length) return null;
  const adjusted = positionMs - offsetMs;
  let timed = false;
  for (let index = lines.length - 1; index >= 0; index -= 1) {
    const line = lines[index];
    if (line.startMs == null) continue;
    timed = true;
    if (line.startMs <= adjusted) return line;
  }
  // A timed song's instrumental intro should not highlight the first lyric early.
  return timed ? null : lines[0];
}
