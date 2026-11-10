import { ChevronUp, GripHorizontal, Music2, Pause, Play, X } from "lucide-react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useEffect, useState } from "react";
import { commands } from "./tauri";
import { runAction as run, useActionError } from "./actionErrors";
import { useSnapshot } from "./useSnapshot";
import { fallbackPalette, paletteFromCover } from "./palette";
import appIcon from "../assets/icon.svg";
import { playbackAlert } from "./uiFeedback";

export default function MiniApp() {
  const { snapshot, activeLine, connectionError } = useSnapshot();
  const { actionError, dismissActionError } = useActionError();
  const analysis = activeLine ? snapshot.analyses[activeLine.id] : undefined;
  const alertText = actionError || connectionError || playbackAlert(snapshot);
  const playing = snapshot.media?.status === "playing";
  const cover = snapshot.media?.thumbnailDataUrl;
  const [palette, setPalette] = useState(fallbackPalette);

  useEffect(() => {
    let cancelled = false;
    if (!cover) {
      setPalette(fallbackPalette);
      return;
    }
    void paletteFromCover(cover)
      .then((next) => {
        if (!cancelled) setPalette(next);
      })
      .catch(() => {
        if (!cancelled) setPalette(fallbackPalette);
      });
    return () => {
      cancelled = true;
    };
  }, [cover]);

  return (
    <div
      className="mini-shell"
      style={
        {
          "--mini-cover": cover ? `url("${cover}")` : "none",
          "--tone-primary": palette.primary,
          "--tone-secondary": palette.secondary,
          "--tone-deep": palette.deep,
          "--tone-highlight": palette.highlight
        } as React.CSSProperties
      }
    >
      <div className="mini-ambient" />
      <header className="mini-drag" data-tauri-drag-region>
        <span className="mini-brand"><img src={appIcon} alt="" /> KotobaBeat</span>
        <GripHorizontal size={18} />
        <div>
          <button
            title="打开主界面"
            onClick={() => run(() => commands.showWindow("main"))}
          ><ChevronUp size={15} /></button>
          <button
            title="隐藏迷你窗"
            onClick={() => run(() => getCurrentWindow().hide())}
          ><X size={15} /></button>
        </div>
      </header>
      <main className="mini-content">
        <div className="mini-cover-shell">
          {cover ? (
            <img className="mini-cover" src={cover} alt="" />
          ) : (
            <div className="mini-cover mini-cover-fallback"><Music2 size={22} /></div>
          )}
          <button
            className="mini-play"
            disabled={!snapshot.media?.capabilities.canPlayPause}
            onClick={() => run(commands.togglePlayPause)}
            aria-label={playing ? "暂停" : "播放"}
          >
            {playing ? <Pause size={16} fill="currentColor" /> : <Play size={16} fill="currentColor" />}
          </button>
        </div>
        <div className="mini-line">
          <span className="mini-source">
            {snapshot.media ? `${snapshot.media.title} · ${snapshot.media.artist}` : "等待播放器"}
          </span>
          <h1>{activeLine?.text || (snapshot.media ? snapshot.media.title : "等待播放")}</h1>
          {(alertText || analysis?.translationZh) && <p role={alertText ? "alert" : undefined}>
            {alertText || analysis?.translationZh}
            {actionError && <button className="mini-error-dismiss" aria-label="关闭错误提示" onClick={dismissActionError}>×</button>}
          </p>}
          {analysis?.grammarPoints.length ? (
            <div className="mini-grammar">
              {analysis.grammarPoints.slice(0, 2).map((point, index) => (
                <div className="mini-grammar-card" key={`${point.pattern}-${index}`}>
                  <strong><span>{point.level}</span>{point.pattern}</strong>
                  <small>{point.meaningZh}</small>
                </div>
              ))}
            </div>
          ) : null}
        </div>
      </main>
    </div>
  );
}
