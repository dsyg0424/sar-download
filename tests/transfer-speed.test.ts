import { expect, test } from "bun:test";
import { TransferSpeed } from "../src/features/downloader/model/transferSpeed";

test("speed smoothing uses elapsed time and preserves steady transfer rates", () => {
  const speed = new TransferSpeed();
  speed.sample(100, 0);
  speed.sample(100, 500);
  expect(speed.read(500)).toBe(100);
  speed.sample(500, 1000);
  expect(speed.read(1000)).toBeCloseTo(100 + (1 - Math.exp(-0.25)) * 400);
});

test("zero samples and missing events stop stale speed after two seconds", () => {
  const speed = new TransferSpeed();
  speed.sample(100, 0);
  speed.sample(0, 500);
  expect(speed.read(500)).toBeGreaterThan(0);
  expect(speed.read(2000)).toBe(0);
  speed.sample(200, 2500);
  expect(speed.read(2500)).toBe(200);
  expect(speed.read(4500)).toBe(0);
});

test("invalid rates cannot leak into the display", () => {
  const speed = new TransferSpeed();
  for (const raw of [NaN, Infinity, -1, 0]) {
    speed.sample(raw, 0);
    expect(speed.read(0)).toBe(0);
  }
});
