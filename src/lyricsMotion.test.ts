import { describe, expect, it } from "vitest";
import {
  easeLyricsScroll,
  lyricsScrollDuration,
  lyricsScrollTarget
} from "./lyricsMotion";

describe("lyrics motion", () => {
  it("places the active line slightly above the visual center", () => {
    expect(lyricsScrollTarget(800, 80, 600, 1800)).toBe(564);
  });

  it("clamps the target at both ends", () => {
    expect(lyricsScrollTarget(10, 40, 600, 1800)).toBe(0);
    expect(lyricsScrollTarget(1750, 80, 600, 1800)).toBe(1200);
  });

  it("uses a fast, non-bouncing Apple-like ease", () => {
    expect(easeLyricsScroll(0)).toBe(0);
    expect(easeLyricsScroll(0.5)).toBeCloseTo(0.875);
    expect(easeLyricsScroll(1)).toBe(1);
    expect(lyricsScrollDuration(2_000)).toBe(820);
  });
});
