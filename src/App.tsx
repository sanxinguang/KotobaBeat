import {
  BookOpenText,
  Check,
  ChevronDown,
  CircleHelp,
  Download,
  Gauge,
  Import,
  KeyRound,
  Languages,
  ListMusic,
  Maximize2,
  Minimize2,
  Music2,
  Pause,
  Play,
  RefreshCw,
  RotateCcw,
  Settings2,
  SkipBack,
  SkipForward,
  Sparkles,
  Square,
  X
} from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { open } from "@tauri-apps/plugin-dialog";
import {
  disable as disableAutostart,
  enable as enableAutostart,
  isEnabled as isAutostartEnabled
} from "@tauri-apps/plugin-autostart";
import { commands, isTauri } from "./tauri";
import appIcon from "../assets/icon.svg";
import { fallbackPalette, paletteFromCover } from "./palette";
import {
  easeLyricsScroll,
  lyricsScrollDuration,
  lyricsScrollTarget
} from "./lyricsMotion";
import type {
  AppSnapshot,
  LineAnalysis,
  LyricCandidate,
  LyricLine,
  SettingsPatch
} from "./types";
import { useSnapshot } from "./useSnapshot";
import { RubyText } from "./RubyText";
import { errorMessage, runAction as safely, useActionError } from "./actionErrors";
import { isPlaybackShortcut, playerLabel } from "./keyboard";
import { useDialogFocus } from "./useDialogFocus";
import { validateProviderContact } from "./providerContact";
import { candidateWarningLabels, playbackAlert } from "./uiFeedback";

const formatTime = (milliseconds: number) => {
  const total = Math.max(0, Math.floor(milliseconds / 1000));
  return `${Math.floor(total / 60)}:${String(total % 60).padStart(2, "0")}`;
};

function Titlebar({ onSettings }: { onSettings: () => void }) {
  const minimize = () => safely(() => getCurrentWindow().minimize());
  const maximize = () => safely(() => getCurrentWindow().toggleMaximize());
  const close = () => safely(() => getCurrentWindow().close());

  return (
    <header className="titlebar" data-tauri-drag-region>
      <div className="brand" data-tauri-drag-region>
        <span className="brand-mark"><img src={appIcon} alt="" /></span>
        <span>KotobaBeat</span>
      </div>
      <div className="titlebar-actions">
        <button className="window-button" onClick={onSettings} aria-label="设置">
          <Settings2 size={16} />
        </button>
        <button className="window-button" onClick={minimize} aria-label="最小化">
          <Minimize2 size={15} />
        </button>
        <button className="window-button" onClick={maximize} aria-label="最大化">
          <Square size={12} />
        </button>
        <button className="window-button close" onClick={close} aria-label="关闭到托盘">
          <X size={17} />
        </button>
      </div>
    </header>
  );
}

function AlbumPanel({
  snapshot,
  positionMs,
  onRematch
}: {
  snapshot: AppSnapshot;
  positionMs: number;
  onRematch: () => void;
}) {
  const media = snapshot.media;
  const progress = media?.durationMs ? (positionMs / media.durationMs) * 100 : 0;
  const playing = media?.status === "playing";
  const capability = media?.capabilities;
  const estimatedPosition = media?.positionEstimated === true;

  return (
    <aside className="album-panel">
      {(snapshot.sessions.length > 1 || snapshot.selectedSource) && (
        <div className="source-row">
          <span>播放器</span>
          <label className="source-picker">
            <select
              aria-label="选择媒体会话"
              value={snapshot.selectedSource ?? ""}
              onChange={(event) =>
                safely(() => commands.selectSession(event.target.value || null))
              }
            >
              <option value="">自动跟随播放器</option>
              {snapshot.selectedSource && !snapshot.sessions.some((session) => session.sourceApp === snapshot.selectedSource) && (
                <option value={snapshot.selectedSource}>已锁定播放器（暂不可用）</option>
              )}
              {snapshot.sessions.map((session) => (
                <option value={session.sourceApp} key={session.id}>
                  {playerLabel(session.sourceApp)}{session.title ? ` · ${session.title}` : ""}
                </option>
              ))}
            </select>
            <ChevronDown size={13} />
          </label>
        </div>
      )}

      <div className="artwork-shell">
        {media?.thumbnailDataUrl ? (
          <img src={media.thumbnailDataUrl} alt="" className="artwork" />
        ) : (
          <div className="artwork fallback-art" aria-label="默认专辑封面">
            <span className="art-ring ring-one" />
            <span className="art-ring ring-two" />
            <span className="art-sun" />
            <span className="art-title">言葉</span>
          </div>
        )}
        <div className="artwork-shine" />
      </div>

      <div className="track-heading">
        {media?.album && <p className="eyebrow">{media.album}</p>}
        <h1>{media?.title || "播放一首日语歌"}</h1>
        {media?.artist && <p className="artist">{media.artist}</p>}
      </div>

      <div className="transport">
        <div className="progress-track">
          <span style={{ width: `${Math.min(100, Math.max(0, progress))}%` }} />
        </div>
        <div
          className="time-row"
          title={
            estimatedPosition
              ? "估算进度，播放器未提供时间轴"
              : undefined
          }
        >
          <span>{estimatedPosition ? "≈" : ""}{formatTime(positionMs)}</span>
          <span>
            {media?.durationMs
              ? `-${formatTime(Math.max(0, media.durationMs - positionMs))}`
              : "--:--"}
          </span>
        </div>
        <div className="transport-buttons">
          <button
            disabled={!capability?.canSkipPrevious}
            onClick={() => safely(commands.skipPrevious)}
            aria-label="上一首"
          >
            <SkipBack size={23} fill="currentColor" />
          </button>
          <button
            className="play-button"
            disabled={!capability?.canPlayPause}
            onClick={() => safely(commands.togglePlayPause)}
            aria-label={playing ? "暂停" : "播放"}
          >
            {playing ? <Pause size={24} fill="currentColor" /> : <Play size={24} fill="currentColor" />}
          </button>
          <button
            disabled={!capability?.canSkipNext}
            onClick={() => safely(commands.skipNext)}
            aria-label="下一首"
          >
            <SkipForward size={23} fill="currentColor" />
          </button>
        </div>
      </div>

      <div className="quick-actions">
        <button onClick={() => safely(() => commands.showWindow("mini"))}>
          <Maximize2 size={15} /> 迷你歌词
        </button>
        <button
          disabled={!media || ["identifying", "resolving_lyrics"].includes(snapshot.phase)}
          title="重新搜索，不清除已有翻译"
          onClick={onRematch}
        >
          <RefreshCw size={15} /> 重新匹配
        </button>
      </div>
    </aside>
  );
}

function LyricsPanel({
  snapshot,
  positionMs,
  activeLine,
  selectedLine,
  onSelect,
  onOffset,
  onImport,
  onPaste,
  onChooseCandidates
}: {
  snapshot: AppSnapshot;
  positionMs: number;
  activeLine: LyricLine | null;
  selectedLine: LyricLine | null;
  onSelect: (line: LyricLine) => void;
  onOffset: (value: number) => void;
  onImport: () => void;
  onPaste: () => void;
  onChooseCandidates: () => void;
}) {
  const activeRef = useRef<HTMLButtonElement>(null);
  const scrollRef = useRef<HTMLDivElement>(null);
  const scrollAnimationRef = useRef<number | null>(null);
  const positionedGenerationRef = useRef<number | null>(null);
  const lines = snapshot.lyrics?.lines ?? [];
  const furiganaLayout = snapshot.settings.lyricsFuriganaEnabled
    ? lines.filter((line) => snapshot.analyses[line.id]?.furigana?.length).map((line) => line.id).join("|")
    : "";
  const activeIndex = useMemo(
    () => lines.findIndex((line) => line.id === activeLine?.id),
    [activeLine?.id, lines]
  );

  useEffect(() => {
    const item = activeRef.current;
    const container = scrollRef.current;
    if (!item || !container) return;

    if (scrollAnimationRef.current != null) {
      cancelAnimationFrame(scrollAnimationRef.current);
    }

    const start = container.scrollTop;
    const target = lyricsScrollTarget(
      item.offsetTop,
      item.offsetHeight,
      container.clientHeight,
      container.scrollHeight
    );
    const distance = target - start;
    const reduceMotion =
      snapshot.settings.reduceMotion ||
      window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    const isFirstPositionForTrack =
      positionedGenerationRef.current !== snapshot.generation;
    positionedGenerationRef.current = snapshot.generation;

    if (reduceMotion || isFirstPositionForTrack || Math.abs(distance) < 2) {
      container.scrollTop = target;
      return;
    }

    const duration = lyricsScrollDuration(distance);
    const startedAt = performance.now();
    const animate = (now: number) => {
      const progress = Math.min(1, (now - startedAt) / duration);
      container.scrollTop = start + distance * easeLyricsScroll(progress);
      if (progress < 1) {
        scrollAnimationRef.current = requestAnimationFrame(animate);
      } else {
        scrollAnimationRef.current = null;
      }
    };
    scrollAnimationRef.current = requestAnimationFrame(animate);

    return () => {
      if (scrollAnimationRef.current != null) {
        cancelAnimationFrame(scrollAnimationRef.current);
        scrollAnimationRef.current = null;
      }
    };
  }, [activeLine?.id, snapshot.generation, snapshot.settings.reduceMotion, snapshot.settings.lyricsFuriganaEnabled, furiganaLayout]);

  if (!snapshot.media) {
    return (
      <main className="lyrics-panel empty-state">
        <div className="empty-orbit"><Music2 size={32} /></div>
        <h2>{snapshot.settings.monitoringEnabled ? "等待播放" : "已暂停监听"}</h2>
        {snapshot.message && <p role="status">{snapshot.message}</p>}
        {snapshot.settings.monitoringEnabled && !snapshot.message?.includes("SMTC") && (
          <p className="muted-copy">正在播放却没有歌词？请在网易云「设置 → 系统」开启 SMTC。</p>
        )}
        {!snapshot.settings.monitoringEnabled && (
          <button className="primary" onClick={() => safely(() => commands.updateSettings({ monitoringEnabled: true }))}>恢复监听</button>
        )}
      </main>
    );
  }

  if (!lines.length) {
    const busy = ["identifying", "resolving_lyrics"].includes(snapshot.phase);
    return (
      <main className="lyrics-panel empty-state">
        <div className={`empty-orbit ${busy ? "is-loading" : ""}`}>
          {busy ? <RefreshCw size={30} /> : <ListMusic size={30} />}
        </div>
        <h2>{busy ? "搜索歌词…" : snapshot.lyrics?.instrumental ? "纯音乐" : "暂无歌词"}</h2>
        {snapshot.phase === "error" && <p role="alert">{snapshot.message}</p>}
        {!busy && (
          <div className="empty-actions">
            {!!snapshot.candidates.length && <button className="primary" onClick={onChooseCandidates}><ListMusic size={16} /> 选择候选</button>}
            <button className="primary" onClick={onImport}><Import size={16} /> 导入 LRC</button>
            <button onClick={onPaste}><Download size={16} /> 粘贴歌词</button>
          </div>
        )}
      </main>
    );
  }

  return (
    <main className="lyrics-panel">
      <div className="lyrics-header">
        <div>
          <span className="provider-pill">{snapshot.lyrics?.provider}</span>
        </div>
        <div className="sync-adjuster">
          <button
            onClick={() => onOffset((snapshot.lyrics?.offsetMs ?? 0) - 100)}
            title="歌词提前 0.1 秒"
            aria-label="歌词提前 0.1 秒"
          >−</button>
          <span>{snapshot.lyrics?.offsetMs ? `${snapshot.lyrics.offsetMs > 0 ? "+" : ""}${snapshot.lyrics.offsetMs}ms` : "同步"}</span>
          <button
            onClick={() => onOffset((snapshot.lyrics?.offsetMs ?? 0) + 100)}
            title="歌词延后 0.1 秒"
            aria-label="歌词延后 0.1 秒"
          >+</button>
          <button
            onClick={() => onOffset(0)}
            title="重置歌词偏移"
          ><RotateCcw size={13} /></button>
        </div>
      </div>
      <div className={`lyrics-scroll ${snapshot.settings.lyricsFuriganaEnabled ? "with-furigana" : ""}`} ref={scrollRef}>
        <div className="lyrics-spacer" />
        {lines.map((line, index) => {
          const analysis = snapshot.analyses[line.id];
          const active = activeLine?.id === line.id;
          const selected = selectedLine?.id === line.id;
          const distance = activeIndex < 0 ? 0 : index - activeIndex;
          const depth = Math.min(4, Math.abs(distance));
          const adjustedPosition = positionMs - (snapshot.lyrics?.offsetMs ?? 0);
          const elapsed = line.startMs == null ? 0 : adjustedPosition - line.startMs;
          const duration =
            line.endMs != null && line.startMs != null ? line.endMs - line.startMs : 0;
          const lineProgress = active && duration > 0 ? Math.min(1, Math.max(0, elapsed / duration)) : 0;
          return (
            <button
              key={line.id}
              ref={active ? activeRef : undefined}
              className={`lyric-line ${active ? "active" : ""} ${selected ? "selected" : ""} ${distance < 0 ? "past" : distance > 0 ? "future" : ""}`}
              data-depth={depth}
              onClick={() => onSelect(line)}
            >
              <span className="lyric-original">
                <RubyText text={line.text} segments={analysis?.furigana} enabled={snapshot.settings.lyricsFuriganaEnabled} />
                {active && <span className="line-glow" style={{ width: `${lineProgress * 100}%` }} />}
              </span>
              {analysis?.translationZh && (
                <span className="lyric-translation">{analysis.translationZh}</span>
              )}
            </button>
          );
        })}
        <div className="lyrics-spacer bottom" />
      </div>
    </main>
  );
}

function GrammarPanel({
  line,
  analysis,
  snapshot,
  onSettings
}: {
  line: LyricLine | null;
  analysis?: LineAnalysis;
  snapshot: AppSnapshot;
  onSettings: () => void;
}) {
  return (
    <aside className="grammar-panel">
      <div className="grammar-heading">
        <div>
          <h2>歌词解析</h2>
        </div>
        <button className="icon-button" onClick={onSettings}><Settings2 size={18} /></button>
      </div>

      {!line ? (
        <div className="grammar-placeholder">
          <BookOpenText size={32} />
        </div>
      ) : !analysis ? (
        <div className="grammar-placeholder">
          <Sparkles className={snapshot.phase === "analyzing" ? "sparkle" : ""} size={32} />
          <p>
            {snapshot.settings.hasApiKey
              ? snapshot.phase === "analyzing" ? "加载中…" : "暂无解析"
              : "请先设置翻译服务"}
          </p>
          {!snapshot.settings.hasApiKey && (
            <button className="primary" onClick={onSettings}><KeyRound size={15} /> 前往设置</button>
          )}
          {snapshot.settings.hasApiKey && snapshot.phase !== "analyzing" && (
            <button className="primary" onClick={() => safely(commands.retryAnalysis)}><RefreshCw size={15} /> 重试解析</button>
          )}
        </div>
      ) : (
        <div className="grammar-content">
          <section className="focus-sentence">
            <h3><RubyText text={line.text} segments={analysis.furigana} /></h3>
            {!analysis.furigana?.length && analysis.reading && <p className="reading">{analysis.reading}</p>}
            <p className="focus-translation">{analysis.translationZh}</p>
          </section>

          {analysis.literalStructure && (
            <section className="structure-card">
              <div className="card-label"><Gauge size={14} /> 句式</div>
              <p>{analysis.literalStructure}</p>
            </section>
          )}

          {analysis.grammarPoints.length > 0 && <section className="grammar-list">
            <div className="section-title">
              <span>语法与表达</span>
            </div>
            {analysis.grammarPoints.map((point, index) => (
                <article className="grammar-card" key={`${point.pattern}-${index}`}>
                  <div className="grammar-card-top">
                    <span className={`level-tag level-${point.level.toLowerCase()}`}>{point.level}</span>
                    <strong>{point.pattern}</strong>
                    <span className="surface">{point.surface}</span>
                  </div>
                  <p className="meaning">{point.meaningZh}</p>
                  <p>{point.explanationZh}</p>
                  {point.conjugation && <code>{point.conjugation}</code>}
                </article>
              ))}
          </section>}

          {analysis.omittedElements.length > 0 && (
            <section className="note-row">
              <CircleHelp size={16} />
              <div><strong>省略</strong><p>{analysis.omittedElements.join("；")}</p></div>
            </section>
          )}
          {analysis.nuance && (
            <section className="note-row nuance">
              <Sparkles size={16} />
              <div><strong>语气</strong><p>{analysis.nuance}</p></div>
            </section>
          )}
        </div>
      )}
    </aside>
  );
}

function CandidateDialog({
  candidates,
  onClose
}: {
  candidates: LyricCandidate[];
  onClose: () => void;
}) {
  const dialogRef = useDialogFocus();
  const [pending, setPending] = useState(false);
  const [lyricCandidateId, setLyricCandidateId] = useState(
    candidates[0]?.candidateId ?? ""
  );
  const [artworkCandidateId, setArtworkCandidateId] = useState(
    candidates.find((candidate) => candidate.artworkUrl)?.candidateId ?? ""
  );

  return (
    <div className="modal-backdrop">
      <div className="modal candidate-modal" ref={dialogRef} role="dialog" aria-modal="true" aria-label="选择正确的歌曲版本" tabIndex={-1}>
        <div className="modal-heading">
          <div><h2>选择版本</h2></div>
          <button className="icon-button" onClick={onClose} aria-label="关闭候选选择"><X size={19} /></button>
        </div>
        <p className="modal-copy">歌词和封面可分别选择</p>
        <div className="candidate-list">
          {candidates.map((candidate) => {
            const lyricSelected = lyricCandidateId === candidate.candidateId;
            const artworkSelected = artworkCandidateId === candidate.candidateId;
            const warnings = candidateWarningLabels(candidate.warnings);
            return (
              <article
                className={`candidate-row ${lyricSelected ? "lyrics-selected" : ""}`}
                key={candidate.candidateId}
              >
                <button
                  className={`candidate-cover ${artworkSelected ? "selected" : ""}`}
                  disabled={!candidate.artworkUrl}
                  onClick={() => setArtworkCandidateId(candidate.candidateId)}
                  aria-label={candidate.artworkUrl ? `选择 ${candidate.album || candidate.title} 的封面` : "没有可用封面"}
                >
                  {candidate.artworkUrl ? (
                    <img
                      src={candidate.artworkUrl}
                      alt=""
                      loading="lazy"
                      onError={(event) => { event.currentTarget.hidden = true; }}
                    />
                  ) : (
                    <Music2 size={22} />
                  )}
                  <span>{artworkSelected ? <Check size={12} /> : "封面"}</span>
                </button>
                <button
                  className="candidate-lyrics-choice"
                  onClick={() => setLyricCandidateId(candidate.candidateId)}
                >
                  <div className="candidate-score" title="匹配分数，不代表统计准确率">{Math.round(candidate.score * 100)}<small>%</small></div>
                  <div className="candidate-info">
                    <strong>{candidate.title}</strong>
                    <span>{candidate.artist} · {candidate.album || "未知专辑"}</span>
                    <small>{candidate.provider} · {candidate.syncType === "plain" ? "纯文本" : "同步歌词"}</small>
                    {!!warnings.length && <small className="candidate-warning">{warnings.join("；")}</small>}
                  </div>
                  <span className="candidate-duration">{formatTime(candidate.durationMs)}</span>
                  <span className={`candidate-check ${lyricSelected ? "selected" : ""}`}>
                    {lyricSelected && <Check size={14} />}
                  </span>
                </button>
              </article>
            );
          })}
        </div>
        <div className="candidate-footer">
          <button
            className="primary"
            disabled={!lyricCandidateId || pending}
            onClick={async () => {
              setPending(true);
              await safely(() => commands.chooseCandidate(lyricCandidateId, artworkCandidateId || null));
              setPending(false);
            }}
          >
            {pending ? "载入中…" : "确定"}
          </button>
        </div>
      </div>
    </div>
  );
}

function PasteDialog({ onClose }: { onClose: () => void }) {
  const [value, setValue] = useState("");
  const [pending, setPending] = useState(false);
  const dialogRef = useDialogFocus();
  return (
    <div className="modal-backdrop">
      <div className="modal paste-modal" ref={dialogRef} role="dialog" aria-modal="true" aria-label="粘贴歌词或 LRC" tabIndex={-1}>
        <div className="modal-heading">
          <div><h2>粘贴歌词</h2></div>
          <button className="icon-button" onClick={onClose} aria-label="关闭歌词输入"><X size={19} /></button>
        </div>
        <textarea
          autoFocus
          aria-label="歌词或 LRC 内容"
          maxLength={200_000}
          placeholder={"[00:12.30] 歌詞をここに貼り付け\n[00:17.80] 时间标签可选"}
          value={value}
          onChange={(event) => setValue(event.target.value)}
        />
        <div className="modal-footer">
          <span>{value.split(/\r?\n/).filter(Boolean).length} 行</span>
          <button
            className="primary"
            disabled={!value.trim() || pending}
            onClick={async () => {
              setPending(true);
              const success = await safely(() => commands.pasteLyrics(value));
              setPending(false);
              if (success) onClose();
            }}
          >
            {pending ? "载入中…" : "确定"}
          </button>
        </div>
      </div>
    </div>
  );
}

function Toggle({
  checked,
  onChange,
  label,
  disabled = false
}: {
  checked: boolean;
  onChange: (value: boolean) => void;
  label: string;
  disabled?: boolean;
}) {
  return (
    <button
      className={`toggle ${checked ? "on" : ""}`}
      role="switch"
      aria-label={label}
      aria-checked={checked}
      disabled={disabled}
      onClick={() => onChange(!checked)}
    >
      <span />
    </button>
  );
}

function SettingsDrawer({
  snapshot,
  onClose,
  onPreviewSettings
}: {
  snapshot: AppSnapshot;
  onClose: () => void;
  onPreviewSettings: (patch: SettingsPatch) => void;
}) {
  const [key, setKey] = useState("");
  const [status, setStatus] = useState("");
  const [autostart, setAutostart] = useState(false);
  const [autostartPending, setAutostartPending] = useState(true);
  const [savingKey, setSavingKey] = useState(false);
  const [providerContact, setProviderContact] = useState(snapshot.settings.providerContact ?? "");
  const [savingContact, setSavingContact] = useState(false);
  const [contactStatus, setContactStatus] = useState("");
  const [contactError, setContactError] = useState(false);
  const drawerRef = useDialogFocus<HTMLElement>();

  useEffect(() => {
    let active = true;
    if (isTauri()) {
      void isAutostartEnabled().then((value) => {
        if (active) setAutostart(value);
      }).catch(() => {
        if (active) setStatus("无法读取开机启动设置，请稍后重新打开设置页。");
      }).finally(() => {
        if (active) setAutostartPending(false);
      });
    } else setAutostartPending(false);
    return () => { active = false; };
  }, []);

  const patch = (value: SettingsPatch) => {
    if (!isTauri()) return onPreviewSettings(value);
    return safely(() => commands.updateSettings(value));
  };
  const changeAutostart = (value: boolean) => {
    if (!isTauri()) return setAutostart(value);
    setAutostartPending(true);
    void safely(() => value ? enableAutostart() : disableAutostart()).then((success) => {
      if (success) setAutostart(value);
      setAutostartPending(false);
    });
  };

  return (
    <div className="drawer-backdrop" onMouseDown={onClose}>
      <aside className="settings-drawer" ref={drawerRef} role="dialog" aria-modal="true" aria-label="设置" tabIndex={-1} onMouseDown={(event) => event.stopPropagation()}>
        <div className="modal-heading">
          <div><h2>设置</h2></div>
          <button className="icon-button" onClick={onClose} aria-label="关闭设置"><X size={19} /></button>
        </div>

        <section className="settings-section">
          <div className="settings-section-title"><Languages size={16} /> 翻译</div>
          <label className="field-label">模型</label>
          <div className="segmented">
            {[
              ["deepseek-v4-flash", "V4 Flash", "速度优先"],
              ["deepseek-v4-pro", "V4 Pro", "质量优先"]
            ].map(([value, title, subtitle]) => (
              <button
                className={snapshot.settings.deepseekModel === value ? "active" : ""}
                key={value}
                onClick={() => patch({ deepseekModel: value })}
              >
                <strong>{title}</strong><small>{subtitle}</small>
              </button>
            ))}
          </div>
          <label className="field-label">API Key</label>
          <div className="key-field">
            <KeyRound size={16} />
            <input
              type="password"
              aria-label="API Key"
              autoComplete="off"
              spellCheck={false}
              value={key}
              placeholder={snapshot.settings.hasApiKey ? "已保存" : "sk-…"}
              onChange={(event) => setKey(event.target.value)}
            />
            <button
              disabled={!key.trim() || savingKey}
              onClick={() => safely(async () => {
                setSavingKey(true);
                setStatus("正在验证…");
                try {
                  await commands.saveDeepSeekKey(key.trim());
                  setKey("");
                  setStatus("已保存");
                } catch (error) {
                  setStatus(errorMessage(error));
                } finally {
                  setSavingKey(false);
                }
              })}
            >
              {savingKey ? "验证中…" : "保存"}
            </button>
          </div>
          {status && <p className="field-status">{status}</p>}
          {snapshot.settings.hasApiKey && (
            <button className="text-danger" disabled={savingKey} onClick={() => safely(commands.clearDeepSeekKey)}>
              移除密钥
            </button>
          )}
        </section>

        <section className="settings-section">
          <div className="settings-section-title"><BookOpenText size={16} /> 显示</div>
          <SettingRow title="歌词显示假名">
            <Toggle
              label="歌词显示假名"
              checked={snapshot.settings.lyricsFuriganaEnabled}
              onChange={(value) => patch({ lyricsFuriganaEnabled: value })}
            />
          </SettingRow>
          <SettingRow title="减少动态效果">
            <Toggle
              label="减少动态效果"
              checked={snapshot.settings.reduceMotion}
              onChange={(value) => patch({ reduceMotion: value })}
            />
          </SettingRow>
          <SettingRow title="流动背景">
            <Toggle
              label="流动背景"
              checked={snapshot.settings.ambientMotionEnabled}
              onChange={(value) => patch({ ambientMotionEnabled: value })}
            />
          </SettingRow>
          <SettingRow title="迷你窗置顶">
            <Toggle
              label="迷你窗置顶"
              checked={snapshot.settings.miniAlwaysOnTop}
              onChange={(value) => patch({ miniAlwaysOnTop: value })}
            />
          </SettingRow>
        </section>

        <section className="settings-section">
          <div className="settings-section-title"><ListMusic size={16} /> 歌词来源</div>
          <label className="field-label" htmlFor="provider-contact">歌词源联系信息</label>
          <div className="key-field">
            <ListMusic size={16} />
            <input
              id="provider-contact"
              aria-describedby="provider-contact-help provider-contact-status"
              aria-invalid={contactError}
              value={providerContact}
              placeholder="公开项目 HTTPS 地址或邮箱"
              maxLength={256}
              disabled={savingContact}
              autoComplete="off"
              autoCapitalize="none"
              spellCheck={false}
              onChange={(event) => {
                setProviderContact(event.target.value);
                setContactStatus("");
                setContactError(false);
              }}
            />
            <button
              disabled={savingContact}
              aria-label="保存歌词源联系信息"
              onClick={async () => {
                const value = providerContact.trim();
                const invalid = validateProviderContact(value);
                if (invalid) {
                  setContactError(true);
                  setContactStatus(invalid);
                  return;
                }
                setSavingContact(true);
                setContactError(false);
                setContactStatus("正在保存…");
                try {
                  if (isTauri()) await commands.updateSettings({ providerContact: value });
                  else onPreviewSettings({ providerContact: value });
                  setProviderContact(value);
                  setContactStatus(value ? "已保存" : "已清空，LRCLIB 与 MusicBrainz 已停用");
                } catch (error) {
                  setContactError(true);
                  setContactStatus(errorMessage(error));
                } finally {
                  setSavingContact(false);
                }
              }}
            >{savingContact ? "保存中…" : "保存"}</button>
          </div>
          <p className="field-status" id="provider-contact-help">发送给 LRCLIB / MusicBrainz 的公开联系信息。留空则停用这两个来源。</p>
          <p className="field-status" id="provider-contact-status" role={contactError ? "alert" : "status"}>{contactStatus}</p>
          <SettingRow title="网易云音乐">
            <Toggle
              label="网易云音乐"
              checked={snapshot.settings.experimentalNetease}
              onChange={(value) => patch({ experimentalNetease: value })}
            />
          </SettingRow>
          <SettingRow title="QQ 音乐">
            <Toggle
              label="QQ 音乐"
              checked={snapshot.settings.experimentalQq}
              onChange={(value) => patch({ experimentalQq: value })}
            />
          </SettingRow>
        </section>

        <section className="settings-section">
          <div className="settings-section-title"><Settings2 size={16} /> 应用</div>
          <SettingRow title="监听播放器">
            <Toggle
              label="监听播放器"
              checked={snapshot.settings.monitoringEnabled}
              onChange={(value) => patch({ monitoringEnabled: value })}
            />
          </SettingRow>
          <SettingRow title="开机时启动">
            <Toggle label="开机时启动" checked={autostart} disabled={autostartPending} onChange={changeAutostart} />
          </SettingRow>
        </section>
        <p className="about-copy">KotobaBeat 0.1.0</p>
      </aside>
    </div>
  );
}

function SettingRow({
  title,
  children
}: {
  title: string;
  children: React.ReactNode;
}) {
  return (
    <div className="setting-row">
      <div><strong>{title}</strong></div>
      {children}
    </div>
  );
}

export default function App() {
  const { snapshot, setSnapshot, positionMs, activeLine, connectionError } = useSnapshot();
  const { actionError, dismissActionError } = useActionError();
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [pasteOpen, setPasteOpen] = useState(false);
  const [candidateRequest, setCandidateRequest] = useState<string | null>(null);
  const [coverPalette, setCoverPalette] = useState(fallbackPalette);
  const trackKey = snapshot.media
    ? JSON.stringify([snapshot.media.sessionId, snapshot.media.title, snapshot.media.artist, snapshot.media.album])
    : "";
  const candidateOpen = candidateRequest === trackKey && snapshot.phase === "candidate_required" && snapshot.candidates.length > 0;
  const alertText = actionError || connectionError || playbackAlert(snapshot);

  const selectedLine = useMemo(
    () =>
      snapshot.lyrics?.lines.find((line) => line.id === selectedId) ??
      activeLine,
    [activeLine, selectedId, snapshot.lyrics?.lines]
  );
  const analysis = selectedLine ? snapshot.analyses[selectedLine.id] : undefined;
  const cover = snapshot.media?.thumbnailDataUrl;

  useEffect(() => {
    let cancelled = false;
    if (!cover) {
      setCoverPalette(fallbackPalette);
      return;
    }
    void paletteFromCover(cover)
      .then((palette) => {
        if (!cancelled) setCoverPalette(palette);
      })
      .catch(() => {
        if (!cancelled) setCoverPalette(fallbackPalette);
      });
    return () => {
      cancelled = true;
    };
  }, [cover]);

  useEffect(() => {
    setSelectedId(null);
  }, [snapshot.generation]);

  useEffect(() => { setCandidateRequest(null); }, [trackKey]);

  useEffect(() => {
    if (selectedId && activeLine?.id === selectedId) {
      setSelectedId(null);
    }
  }, [activeLine?.id, selectedId]);

  useEffect(() => {
    const handler = (event: KeyboardEvent) => {
      if (isPlaybackShortcut(event, settingsOpen || pasteOpen || candidateOpen)) {
        event.preventDefault();
        void safely(commands.togglePlayPause);
      }
      if (event.code === "Escape") {
        setSettingsOpen(false);
        setPasteOpen(false);
        setCandidateRequest(null);
      }
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [settingsOpen, pasteOpen, candidateOpen]);

  const selectLine = (line: LyricLine) => {
    setSelectedId(line.id);
    if (
      line.startMs != null &&
      snapshot.media?.capabilities.canSeek
    ) {
      void safely(() => commands.seek(Math.max(0, line.startMs! + (snapshot.lyrics?.offsetMs ?? 0))));
    }
  };

  const importFile = () =>
    safely(async () => {
      const selected = await open({
        multiple: false,
        filters: [{ name: "同步歌词", extensions: ["lrc", "txt"] }]
      });
      if (typeof selected === "string") await commands.importLrc(selected);
    });

  return (
    <div
      className={`app-shell ${snapshot.settings.reduceMotion ? "reduce-motion" : ""} ${snapshot.settings.ambientMotionEnabled ? "ambient-motion" : "ambient-static"}`}
      style={
        {
          "--cover-image": cover ? `url("${cover}")` : "none",
          "--tone-primary": coverPalette.primary,
          "--tone-secondary": coverPalette.secondary,
          "--tone-deep": coverPalette.deep,
          "--tone-highlight": coverPalette.highlight
        } as React.CSSProperties
      }
    >
      <div className="ambient-background" />
      <div className="ambient-color-field">
        <span className="ambient-orb ambient-orb-primary" />
        <span className="ambient-orb ambient-orb-secondary" />
        <span className="ambient-orb ambient-orb-highlight" />
        <span className="ambient-orb ambient-orb-deep" />
      </div>
      <div className="ambient-wash" />
      <div className="ambient-noise" />
      <Titlebar onSettings={() => setSettingsOpen(true)} />
      <div className="main-grid">
        <AlbumPanel snapshot={snapshot} positionMs={positionMs} onRematch={() => {
          setCandidateRequest(null);
          void safely(commands.refresh).then((success) => {
            if (success) setCandidateRequest(trackKey);
          });
        }} />
        <LyricsPanel
          snapshot={snapshot}
          positionMs={positionMs}
          activeLine={activeLine}
          selectedLine={selectedLine}
          onSelect={selectLine}
          onOffset={(value) => safely(() => commands.setOffset(value))}
          onImport={importFile}
          onPaste={() => setPasteOpen(true)}
          onChooseCandidates={() => setCandidateRequest(trackKey)}
        />
        <GrammarPanel
          line={selectedLine}
          analysis={analysis}
          snapshot={snapshot}
          onSettings={() => setSettingsOpen(true)}
        />
      </div>

      {alertText && (
        <div className="status-toast action-error" role="alert">
          {alertText}
          {actionError && <button onClick={dismissActionError} aria-label="关闭错误提示"><X size={14} /></button>}
          {!actionError && !connectionError && snapshot.phase === "lyrics_only" && snapshot.settings.hasApiKey && snapshot.lyrics && (
            <button onClick={() => safely(commands.retryAnalysis)}>重试</button>
          )}
        </div>
      )}
      {candidateOpen && (
          <CandidateDialog
            key={`${snapshot.generation}:${snapshot.candidates.map((candidate) => candidate.candidateId).join(",")}`}
            candidates={snapshot.candidates}
            onClose={() => setCandidateRequest(null)}
          />
        )}
      {settingsOpen && <SettingsDrawer snapshot={snapshot} onClose={() => setSettingsOpen(false)} onPreviewSettings={(patch) => setSnapshot((current) => ({ ...current, settings: { ...current.settings, ...patch } }))} />}
      {pasteOpen && <PasteDialog onClose={() => setPasteOpen(false)} />}
    </div>
  );
}
