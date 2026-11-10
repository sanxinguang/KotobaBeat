import { afterEach, describe, expect, it, vi } from "vitest";
import { connectSnapshots } from "./snapshotConnection";
import type { AppSnapshot } from "./types";

const snapshot = (generation: number, revision = generation) => ({ generation, revision } as AppSnapshot);
const flush = async () => { await Promise.resolve(); await Promise.resolve(); };
function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((ok, no) => { resolve = ok; reject = no; });
  return { promise, resolve, reject };
}

afterEach(() => vi.useRealTimers());

describe("snapshot subscription resilience", () => {
  it("subscribes before fetching and does not overwrite an event with a late response", async () => {
    vi.useFakeTimers();
    const pending = deferred<AppSnapshot>();
    const order: string[] = [];
    let emit!: (value: AppSnapshot) => void;
    const onSnapshot = vi.fn();
    const stop = connectSnapshots({
      subscribe: async (callback) => { order.push("listen"); emit = callback; return () => {}; },
      snapshot: () => { order.push("fetch"); return pending.promise; }
    }, onSnapshot, vi.fn());
    await flush();
    emit(snapshot(2, 4));
    pending.resolve(snapshot(2, 3));
    await flush();
    expect(order).toEqual(["listen", "fetch"]);
    expect(onSnapshot).toHaveBeenCalledTimes(1);
    expect(onSnapshot).toHaveBeenLastCalledWith(snapshot(2, 4));
    stop();
  });

  it("unsubscribes even when setup completes after unmount (React StrictMode)", async () => {
    const setup = deferred<() => void>();
    const unlisten = vi.fn<() => void>();
    const fetch = vi.fn();
    const stop = connectSnapshots({ subscribe: () => setup.promise, snapshot: fetch }, vi.fn(), vi.fn());
    stop();
    setup.resolve(unlisten);
    await flush();
    expect(unlisten).toHaveBeenCalledOnce();
    expect(fetch).not.toHaveBeenCalled();
  });

  it("ignores older tracks and out-of-order updates within the same track", async () => {
    vi.useFakeTimers();
    let emit!: (value: AppSnapshot) => void;
    const changed = vi.fn();
    const stop = connectSnapshots({
      subscribe: async (callback) => { emit = callback; return () => {}; },
      snapshot: async () => snapshot(3, 20)
    }, changed, vi.fn());
    await flush();
    emit(snapshot(2, 21));
    emit(snapshot(3, 19));
    emit(snapshot(3, 22));
    expect(changed.mock.calls.map(([value]) => value.revision)).toEqual([20, 22]);
    stop();
  });

  it("falls back to bounded polling after listener failure and stops after cleanup", async () => {
    vi.useFakeTimers();
    const fetch = vi.fn().mockRejectedValueOnce(new Error("offline")).mockResolvedValue(snapshot(1));
    const changed = vi.fn();
    const error = vi.fn();
    const stop = connectSnapshots({ subscribe: async () => { throw new Error("no events"); }, snapshot: fetch }, changed, error);
    await flush();
    expect(error).toHaveBeenLastCalledWith("连接中断，正在重连…");
    await vi.advanceTimersByTimeAsync(2_000);
    expect(changed).toHaveBeenCalledWith(snapshot(1));
    expect(error).toHaveBeenLastCalledWith(null);
    stop();
    await vi.advanceTimersByTimeAsync(10_000);
    expect(fetch).toHaveBeenCalledTimes(2);
  });

  it("does not surface an obsolete fetch error after a newer event succeeds", async () => {
    vi.useFakeTimers();
    const pending = deferred<AppSnapshot>();
    let emit!: (value: AppSnapshot) => void;
    const error = vi.fn();
    const stop = connectSnapshots({
      subscribe: async (callback) => { emit = callback; return () => {}; },
      snapshot: () => pending.promise
    }, vi.fn(), error);
    await flush();
    emit(snapshot(1));
    pending.reject(new Error("old failure"));
    await flush();
    expect(error).toHaveBeenLastCalledWith(null);
    stop();
  });

  it("polls an apparently connected but stalled stream and ignores late events after cleanup", async () => {
    vi.useFakeTimers();
    let emit!: (value: AppSnapshot) => void;
    const changed = vi.fn();
    const fetch = vi.fn().mockResolvedValueOnce(snapshot(1)).mockResolvedValue(snapshot(2));
    const stop = connectSnapshots({
      subscribe: async (callback) => { emit = callback; return () => {}; }, snapshot: fetch
    }, changed, vi.fn());
    await flush();
    await vi.advanceTimersByTimeAsync(5_000);
    expect(changed).toHaveBeenLastCalledWith(snapshot(2));
    stop();
    emit(snapshot(3));
    expect(changed).toHaveBeenCalledTimes(2);
  });
});
