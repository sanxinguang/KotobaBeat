import type { AppSnapshot } from "./types";

export interface SnapshotTransport {
  snapshot: () => Promise<AppSnapshot>;
  subscribe: (callback: (snapshot: AppSnapshot) => void) => Promise<() => void>;
}

/** Subscribe before fetching, and never let an older response replace a newer event. */
export function connectSnapshots(
  transport: SnapshotTransport,
  onSnapshot: (snapshot: AppSnapshot) => void,
  onError: (message: string | null) => void
): () => void {
  let active = true;
  let stop: (() => void) | undefined;
  let retry: ReturnType<typeof setTimeout> | undefined;
  let eventCount = 0;
  let generation = -1;
  let revision = -1;
  let subscribed = false;

  const accept = (next: AppSnapshot) => {
    if (!active || next.generation < generation) return;
    if (next.revision != null && next.revision < revision) return;
    generation = next.generation;
    revision = next.revision ?? revision;
    onSnapshot(next);
    onError(null);
  };

  const refresh = async () => {
    const eventsBeforeRequest = eventCount;
    let failed = false;
    try {
      const next = await transport.snapshot();
      if (eventCount === eventsBeforeRequest) accept(next);
    } catch {
      failed = eventCount === eventsBeforeRequest;
      if (active && failed) onError("连接中断，正在重连…");
    }
    // A cheap local IPC watchdog also recovers if an established event stream stalls.
    if (active) retry = setTimeout(refresh, failed || !subscribed ? 2_000 : 5_000);
  };

  void (async () => {
    try {
      const unlisten = await transport.subscribe((next) => {
        eventCount += 1;
        accept(next);
      });
      if (!active) {
        unlisten();
        return;
      }
      stop = unlisten;
      subscribed = true;
    } catch {
      if (active) onError("正在重新连接…");
    }
    if (active) await refresh();
  })();

  return () => {
    active = false;
    if (retry) clearTimeout(retry);
    stop?.();
  };
}
