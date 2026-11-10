import { useEffect, useMemo, useState } from "react";
import { demoCandidateSnapshot, demoSnapshot } from "./demo";
import { commands, isTauri, onSnapshot } from "./tauri";
import type { AppSnapshot, LyricLine } from "./types";
import { anchorPlayback, extrapolateAnchoredPosition, findActiveLine } from "./utils";
import { connectSnapshots } from "./snapshotConnection";

export function useSnapshot() {
  const [snapshot, setSnapshot] = useState<AppSnapshot>(
    isTauri()
      ? {
          ...demoSnapshot,
          phase: "no_session",
          media: null,
          lyrics: null,
          sessions: [],
          candidates: [],
          analyses: {},
          message: "连接播放器…"
        }
      : new URLSearchParams(window.location.search).has("candidatePreview")
        ? demoCandidateSnapshot
        : demoSnapshot
  );
  const [now, setNow] = useState(performance.now());
  const [connectionError, setConnectionError] = useState<string | null>(null);

  useEffect(() => {
    // 20 Hz keeps line changes visually aligned without repainting the full
    // three-column interface on every animation frame; no timer while idle.
    setNow(performance.now());
    if (snapshot.media?.status !== "playing") return;
    const timer = window.setInterval(() => setNow(performance.now()), 50);
    return () => window.clearInterval(timer);
  }, [snapshot.media?.status]);

  useEffect(() => {
    if (!isTauri()) return;
    const disconnect = connectSnapshots(
      { snapshot: commands.snapshot, subscribe: onSnapshot },
      setSnapshot,
      setConnectionError
    );
    return disconnect;
  }, []);

  const playbackAnchor = useMemo(
    () => anchorPlayback(snapshot.media, Date.now(), performance.now()),
    [snapshot.media]
  );
  const positionMs = extrapolateAnchoredPosition(playbackAnchor, now);

  const activeLine = useMemo<LyricLine | null>(() => {
    const lyrics = snapshot.lyrics;
    if (!lyrics) return null;
    return findActiveLine(lyrics.lines, positionMs, lyrics.offsetMs);
  }, [positionMs, snapshot.lyrics]);

  return { snapshot, setSnapshot, positionMs, activeLine, connectionError };
}
