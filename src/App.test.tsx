// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AppSnapshot, LyricLine } from "./types";
import { demoCandidateSnapshot, demoSnapshot } from "./demo";

const live = vi.hoisted(() => ({ snapshot: null as AppSnapshot | null, activeLine: null as LyricLine | null }));
vi.mock("./useSnapshot", () => ({
  useSnapshot: () => ({ snapshot: live.snapshot, setSnapshot: vi.fn(), positionMs: 0, activeLine: live.activeLine, connectionError: null })
}));
vi.mock("./tauri", () => ({
  isTauri: () => true,
  commands: {
    refresh: vi.fn().mockResolvedValue(undefined),
    chooseCandidate: vi.fn().mockResolvedValue(undefined),
    selectSession: vi.fn().mockResolvedValue(undefined),
    togglePlayPause: vi.fn().mockResolvedValue(undefined),
    updateSettings: vi.fn().mockResolvedValue(undefined),
    showWindow: vi.fn().mockResolvedValue(undefined),
    retryAnalysis: vi.fn().mockResolvedValue(undefined)
  }
}));
vi.mock("@tauri-apps/plugin-autostart", () => ({ isEnabled: async () => false, enable: async () => {}, disable: async () => {} }));

import App from "./App";
import { commands } from "./tauri";

let host: HTMLDivElement;
let root: Root;

beforeEach(() => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  vi.clearAllMocks();
  live.snapshot = structuredClone(demoSnapshot);
  live.activeLine = null;
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
});

afterEach(async () => {
  await act(async () => root.unmount());
  host.remove();
});

const render = async () => act(async () => { root.render(<App />); });
const click = async (button: Element | null) => {
  if (!(button instanceof HTMLElement)) throw new Error("Missing test button");
  await act(async () => button.click());
};

describe("application recovery paths", () => {
  it("renders completed grammar and marks estimated playback without crashing", async () => {
    live.snapshot = {
      ...demoSnapshot,
      media: { ...demoSnapshot.media!, positionEstimated: true },
      settings: { ...demoSnapshot.settings, reduceMotion: true }
    };
    live.activeLine = demoSnapshot.lyrics!.lines[2];
    await render();
    expect(host.textContent).toContain("句式");
    expect(host.textContent).toContain("只比昨天稍微更进一步");
    expect(host.querySelector(".time-row")?.getAttribute("title")).toContain("估算进度");
    expect(host.querySelector(".lyric-line.active")).not.toBeNull();
  });

  it("shows backend media errors and conditional SMTC guidance even without a media session", async () => {
    live.snapshot = { ...demoSnapshot, media: null, lyrics: null, phase: "no_session", message: "暂时无法读取 Windows 媒体会话" };
    await render();
    expect(host.textContent).toContain("暂时无法读取 Windows 媒体会话");
    expect(host.textContent).toContain("正在播放却没有歌词");
    expect(host.textContent).toContain("开启 SMTC");
    expect(host.textContent).not.toContain("SMTC 已关闭");
    expect(host.querySelector<HTMLButtonElement>(".quick-actions button:last-child")?.disabled).toBe(true);
  });

  it("distinguishes deliberately paused monitoring from an unavailable player", async () => {
    live.snapshot = { ...demoSnapshot, media: null, lyrics: null, settings: { ...demoSnapshot.settings, monitoringEnabled: false } };
    await render();
    expect(host.textContent).toContain("已暂停监听");
    expect(host.textContent).not.toContain("开启 SMTC");
    await click([...host.querySelectorAll("button")].find((button) => button.textContent === "恢复监听") ?? null);
    expect(commands.updateSettings).toHaveBeenCalledWith({ monitoringEnabled: true });
  });

  it("only opens candidates on request and stays quiet after switching tracks", async () => {
    live.snapshot = { ...demoCandidateSnapshot, lyrics: null };
    await render();
    expect(host.querySelector("[role='dialog']")).toBeNull();
    expect(host.querySelector(".status-toast")).toBeNull();
    expect(host.textContent).not.toContain("请确认歌曲版本");
    await click(host.querySelector(".quick-actions button:last-child"));
    expect(host.querySelector("[role='dialog']")).not.toBeNull();
    await click(host.querySelector("[aria-label='关闭候选选择']"));
    expect(host.querySelector("[role='dialog']")).toBeNull();
    await click([...host.querySelectorAll("button")].find((button) => button.textContent?.includes("选择候选")) ?? null);
    expect(host.querySelector("[role='dialog']")).not.toBeNull();
    await click(host.querySelector("[aria-label='关闭候选选择']"));
    live.snapshot = { ...live.snapshot!, generation: live.snapshot!.generation + 1, media: { ...live.snapshot!.media!, title: "另一首歌" } };
    await render();
    expect(host.querySelector("[role='dialog']")).toBeNull();
  });

  it("does not submit a stale candidate ID when candidate results are replaced", async () => {
    live.snapshot = { ...demoCandidateSnapshot, lyrics: null };
    await render();
    await click(host.querySelector(".quick-actions button:last-child"));
    live.snapshot = {
      ...live.snapshot!,
      candidates: live.snapshot!.candidates.map((candidate) => ({ ...candidate, candidateId: `new-${candidate.candidateId}` }))
    };
    await render();
    await click(host.querySelector(".candidate-footer button"));
    expect(commands.chooseCandidate).toHaveBeenCalledWith("new-candidate-0", "new-candidate-0");
  });

  it("removes retired disclaimers even from candidates sent by an older backend", async () => {
    live.snapshot = {
      ...demoCandidateSnapshot,
      candidates: demoCandidateSnapshot.candidates.map((candidate) => ({
        ...candidate,
        warnings: ["实验性非官方接口", "时长未知", "缺少可靠时长，需手动确认版本", "版本标签不一致，请确认是否为 Live、翻唱或短版"]
      }))
    };
    await render();
    await click(host.querySelector(".quick-actions button:last-child"));
    expect(host.querySelector("[role='dialog']")).not.toBeNull();
    expect(host.textContent).not.toMatch(/实验性|非官方|请确认|时长未知|缺少可靠时长/);
    expect(host.querySelector(".candidate-warning")?.textContent).toBe("版本不同");
  });

  it("can reopen candidate selection while a refresh preserves the previous lyrics", async () => {
    live.snapshot = structuredClone(demoCandidateSnapshot);
    await render();
    await click(host.querySelector(".quick-actions button:last-child"));
    await click(host.querySelector("[aria-label='关闭候选选择']"));
    expect(host.querySelector("[role='dialog']")).toBeNull();
    expect(host.querySelector(".lyrics-scroll")).not.toBeNull();
    expect(host.querySelector(".status-toast")).toBeNull();
    await click(host.querySelector(".quick-actions button:last-child"));
    expect(host.querySelector("[role='dialog']")).not.toBeNull();
  });

  it("keeps keyboard focus inside a modal and does not hijack Space", async () => {
    live.snapshot = { ...demoCandidateSnapshot, lyrics: null };
    await render();
    await click(host.querySelector(".quick-actions button:last-child"));
    const close = host.querySelector<HTMLButtonElement>("[aria-label='关闭候选选择']")!;
    const confirm = host.querySelector<HTMLButtonElement>(".candidate-footer button")!;
    expect(document.activeElement).toBe(close);
    confirm.focus();
    await act(async () => {
      confirm.dispatchEvent(new KeyboardEvent("keydown", { key: "Tab", code: "Tab", bubbles: true, cancelable: true }));
    });
    expect(document.activeElement).toBe(close);
    await act(async () => {
      close.dispatchEvent(new KeyboardEvent("keydown", { key: " ", code: "Space", bubbles: true, cancelable: true }));
    });
    expect(commands.togglePlayPause).not.toHaveBeenCalled();
  });

  it("surfaces failed commands without leaking key-looking text", async () => {
    vi.mocked(commands.refresh).mockRejectedValueOnce(new Error("请求失败 sk-secret123"));
    await render();
    await click(host.querySelector(".quick-actions button:last-child"));
    const alert = host.querySelector("[role='alert']");
    expect(alert?.textContent).toContain("请求失败 [凭据已隐藏]");
    expect(alert?.textContent).not.toContain("sk-secret123");
    await click(host.querySelector("[aria-label='关闭错误提示']"));
    expect(host.querySelector("[role='alert']")).toBeNull();
  });

  it("hides routine provider diagnostics but permits switching away from a missing player", async () => {
    live.snapshot = { ...demoSnapshot, selectedSource: "cloudmusic.exe", notices: ["一个歌词源暂不可用，正在使用其他来源"] };
    await render();
    expect(host.textContent).not.toContain("一个歌词源暂不可用");
    expect(host.textContent).not.toContain("状态提示");
    expect(host.querySelector(".diagnostic-notices")).toBeNull();
    expect(host.textContent).toContain("已锁定播放器（暂不可用）");
    expect(host.textContent).not.toContain("cloudmusic.exe");
    const selector = host.querySelector<HTMLSelectElement>("[aria-label='选择媒体会话']")!;
    await act(async () => {
      selector.value = "";
      selector.dispatchEvent(new Event("change", { bubbles: true }));
    });
    expect(commands.selectSession).toHaveBeenCalledWith(null);
  });

  it("explains contact sharing and blocks malformed contact before saving", async () => {
    live.snapshot = { ...demoSnapshot, settings: { ...demoSnapshot.settings, providerContact: "not-a-contact" } };
    await render();
    await click(host.querySelector("[aria-label='设置']"));
    expect(host.textContent).toContain("发送给 LRCLIB / MusicBrainz");
    expect(host.textContent).toContain("公开联系信息");
    expect(host.textContent).not.toMatch(/实验适配器|非官方接口|本地优先|PREFERENCES/);
    expect(host.querySelector("[role='switch'][aria-label='网易云音乐']")).not.toBeNull();
    expect(host.querySelector("[role='switch'][aria-label='QQ 音乐']")).not.toBeNull();
    await click(host.querySelector("[aria-label='保存歌词源联系信息']"));
    expect(commands.updateSettings).not.toHaveBeenCalled();
    expect(host.querySelector("#provider-contact-status[role='alert']")?.textContent).toContain("请输入有效的邮箱");
  });

  it("shows backend failure when a valid provider contact cannot be saved", async () => {
    live.snapshot = { ...demoSnapshot, settings: { ...demoSnapshot.settings, providerContact: "owner@example.com" } };
    vi.mocked(commands.updateSettings).mockRejectedValueOnce(new Error("无法保存歌词源设置"));
    await render();
    await click(host.querySelector("[aria-label='设置']"));
    await click(host.querySelector("[aria-label='保存歌词源联系信息']"));
    expect(commands.updateSettings).toHaveBeenCalledWith({ providerContact: "owner@example.com" });
    expect(host.querySelector("#provider-contact-status[role='alert']")?.textContent).toContain("无法保存歌词源设置");
    expect(host.querySelector<HTMLInputElement>("#provider-contact")?.value).toBe("owner@example.com");
  });

  it("permits an explicitly empty contact to disable the contact-requiring sources", async () => {
    await render();
    await click(host.querySelector("[aria-label='设置']"));
    await click(host.querySelector("[aria-label='保存歌词源联系信息']"));
    expect(commands.updateSettings).toHaveBeenCalledWith({ providerContact: "" });
    expect(host.querySelector("#provider-contact-status")?.textContent).toContain("LRCLIB 与 MusicBrainz 已停用");
  });

  it("keeps data-loss warnings visible without a diagnostics panel", async () => {
    live.snapshot = { ...demoSnapshot, notices: ["本地数据库无法打开，临时模式"] };
    await render();
    expect(host.querySelector("[role='alert']")?.textContent).toContain("无法保存数据");
    expect(host.textContent).not.toContain("状态提示");
  });

  it("waits for a user-requested search without opening candidates on other tracks", async () => {
    live.snapshot = { ...demoCandidateSnapshot, phase: "ready", candidates: [] };
    await render();
    await click(host.querySelector(".quick-actions button:last-child"));
    live.snapshot = { ...demoCandidateSnapshot, generation: demoCandidateSnapshot.generation + 1 };
    await render();
    expect(host.querySelector("[role='dialog']")).not.toBeNull();
    live.snapshot = { ...live.snapshot, media: { ...live.snapshot.media!, title: "新歌曲" } };
    await render();
    expect(host.querySelector("[role='dialog']")).toBeNull();
  });
});
