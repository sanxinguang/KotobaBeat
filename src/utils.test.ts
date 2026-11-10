import { describe, expect, it } from "vitest";
import type { LyricLine, MediaState } from "./types";
import { anchorPlayback, extrapolateAnchoredPosition, extrapolatePosition, findActiveLine } from "./utils";

const lines: LyricLine[] = [
  { id: "a", startMs: 0, endMs: 1000, text: "a", words: [] },
  { id: "b", startMs: 1000, endMs: 2000, text: "b", words: [] },
  { id: "c", startMs: 2000, endMs: 3000, text: "c", words: [] }
];

describe("lyric synchronization", () => {
  it("selects the latest elapsed line with an offset", () => {
    expect(findActiveLine(lines, 1600, 500)?.id).toBe("b");
    expect(findActiveLine(lines, 2400, 500)?.id).toBe("b");
  });

  it("extrapolates only while playing and clamps to duration", () => {
    const media = {
      positionMs: 9000,
      durationMs: 10_000,
      sampledAtMs: 1000,
      playbackRate: 1,
      status: "playing"
    } as MediaState;
    expect(extrapolatePosition(media, 3000)).toBe(10_000);
    expect(extrapolatePosition({ ...media, status: "paused" }, 3000)).toBe(9000);
  });

  it("does not highlight timed lyrics during the instrumental intro", () => {
    expect(findActiveLine(lines.slice(1), 500)).toBeNull();
    expect(findActiveLine(lines, 0, 300)).toBeNull();
    expect(findActiveLine([], 500)).toBeNull();
    expect(findActiveLine([{ ...lines[0], startMs: null }], 500)?.id).toBe("a");
  });

  it("keeps duplicate lines distinct and selects the last simultaneous timestamp", () => {
    expect(findActiveLine([lines[0], { ...lines[0], id: "repeat" }], 100)?.id).toBe("repeat");
  });

  it("uses a monotonic anchor for playback, supports speed and replaces the anchor after a seek", () => {
    const media = { positionMs: 1_000, durationMs: 20_000, sampledAtMs: 100_000, playbackRate: 2, status: "playing" } as MediaState;
    const anchor = anchorPlayback(media, 100_100, 50);
    expect(extrapolateAnchoredPosition(anchor, 550)).toBe(2_200);
    // Subsequent extrapolation has no wall-clock input, so NTP/manual clock jumps cannot affect it.
    expect(extrapolateAnchoredPosition(anchor, 550)).toBe(2_200);
    const seek = anchorPlayback({ ...media, positionMs: 10_000, sampledAtMs: 90_000 }, 90_000, 600);
    expect(extrapolateAnchoredPosition(seek, 800)).toBe(10_400);
    const paused = anchorPlayback({ ...media, status: "paused" }, 100_100, 50);
    expect(extrapolateAnchoredPosition(paused, 800)).toBe(1_000);
  });

  it("clamps malformed positions and tolerates missing rates without NaN propagation", () => {
    const media = { positionMs: -200, durationMs: 0, sampledAtMs: 1_000, playbackRate: NaN, status: "playing" } as MediaState;
    expect(extrapolatePosition(media, 1_500)).toBe(500);
    expect(extrapolatePosition({ ...media, positionMs: NaN }, NaN)).toBe(0);
    expect(extrapolateAnchoredPosition(anchorPlayback(null, 0, 0), 5_000)).toBe(0);
  });
});
