export interface CoverPalette {
  primary: string;
  secondary: string;
  deep: string;
  highlight: string;
}

export const fallbackPalette: CoverPalette = {
  primary: "151 91 119",
  secondary: "64 86 137",
  deep: "16 18 28",
  highlight: "226 148 171"
};

type Rgb = [number, number, number];

interface Bucket {
  count: number;
  red: number;
  green: number;
  blue: number;
  score: number;
}

const asCssRgb = ([red, green, blue]: Rgb) =>
  `${Math.round(red)} ${Math.round(green)} ${Math.round(blue)}`;

const mix = (left: Rgb, right: Rgb, amount: number): Rgb => [
  left[0] * (1 - amount) + right[0] * amount,
  left[1] * (1 - amount) + right[1] * amount,
  left[2] * (1 - amount) + right[2] * amount
];

const distance = (left: Rgb, right: Rgb) =>
  Math.hypot(left[0] - right[0], left[1] - right[1], left[2] - right[2]);

export function paletteFromPixels(pixels: Uint8ClampedArray): CoverPalette {
  const buckets = new Map<number, Bucket>();
  for (let index = 0; index + 3 < pixels.length; index += 4) {
    const alpha = pixels[index + 3];
    if (alpha < 160) continue;
    const red = pixels[index];
    const green = pixels[index + 1];
    const blue = pixels[index + 2];
    const maximum = Math.max(red, green, blue);
    const minimum = Math.min(red, green, blue);
    const luminance = (red * 0.2126 + green * 0.7152 + blue * 0.0722) / 255;
    if (luminance < 0.035 || luminance > 0.97) continue;

    const key = (red >> 4) << 8 | (green >> 4) << 4 | (blue >> 4);
    const bucket = buckets.get(key) ?? {
      count: 0,
      red: 0,
      green: 0,
      blue: 0,
      score: 0
    };
    bucket.count += 1;
    bucket.red += red;
    bucket.green += green;
    bucket.blue += blue;
    const saturation = (maximum - minimum) / Math.max(1, maximum);
    bucket.score += 0.42 + saturation * 1.75 + (1 - Math.abs(luminance - 0.52)) * 0.45;
    buckets.set(key, bucket);
  }

  const colors = [...buckets.values()]
    .map((bucket) => ({
      rgb: [
        bucket.red / bucket.count,
        bucket.green / bucket.count,
        bucket.blue / bucket.count
      ] as Rgb,
      score: bucket.score * Math.sqrt(bucket.count)
    }))
    .sort((left, right) => right.score - left.score);

  if (!colors.length) return fallbackPalette;
  const primary = colors[0].rgb;
  const secondary =
    colors
      .slice(1)
      .map((color) => ({
        ...color,
        contrastScore: color.score * (0.42 + distance(primary, color.rgb) / 255)
      }))
      .sort((left, right) => right.contrastScore - left.contrastScore)[0]?.rgb ??
    mix(primary, [65, 83, 138], 0.55);

  return {
    primary: asCssRgb(mix(primary, [255, 255, 255], 0.08)),
    secondary: asCssRgb(mix(secondary, [255, 255, 255], 0.05)),
    deep: asCssRgb(mix(primary, [7, 9, 16], 0.72)),
    highlight: asCssRgb(mix(primary, [255, 255, 255], 0.34))
  };
}

export async function paletteFromCover(source: string): Promise<CoverPalette> {
  const image = new Image();
  image.decoding = "async";
  await new Promise<void>((resolve, reject) => {
    image.onload = () => resolve();
    image.onerror = () => reject(new Error("cover image could not be decoded"));
    // Cached images may finish immediately; attach handlers before setting src.
    image.src = source;
  });

  const canvas = document.createElement("canvas");
  canvas.width = 48;
  canvas.height = 48;
  const context = canvas.getContext("2d", { willReadFrequently: true });
  if (!context) return fallbackPalette;
  context.drawImage(image, 0, 0, canvas.width, canvas.height);
  return paletteFromPixels(
    context.getImageData(0, 0, canvas.width, canvas.height).data
  );
}
