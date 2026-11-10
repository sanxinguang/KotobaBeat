import { describe, expect, it } from "vitest";
import { paletteFromPixels } from "./palette";

describe("cover palette extraction", () => {
  it("selects colorful dominant and contrasting tones", () => {
    const pixels = new Uint8ClampedArray([
      185, 52, 92, 255,
      185, 52, 92, 255,
      185, 52, 92, 255,
      32, 82, 176, 255,
      32, 82, 176, 255
    ]);

    const palette = paletteFromPixels(pixels);

    expect(palette.primary).toMatch(/^\d+ \d+ \d+$/);
    expect(palette.secondary).not.toBe(palette.primary);
    expect(palette.deep).not.toBe(palette.highlight);
  });
});
