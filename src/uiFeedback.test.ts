import { describe, expect, it } from "vitest";
import { demoSnapshot } from "./demo";
import { playbackAlert } from "./uiFeedback";
import type { AppPhase } from "./types";

describe("quiet listening interface", () => {
  it.each<AppPhase>(["ready", "analyzing", "candidate_required", "identifying", "resolving_lyrics"])(
    "does not display routine %s status", (phase) => {
      expect(playbackAlert({ ...demoSnapshot, phase, message: "请确认歌曲版本", notices: ["QQ 音乐：接口暂不可用"] })).toBeNull();
    }
  );

  it("keeps failures that need action", () => {
    expect(playbackAlert({ ...demoSnapshot, phase: "error", message: "导入失败" })).toBe("导入失败");
    expect(playbackAlert({ ...demoSnapshot, notices: ["媒体连接：读取失败"] })).toBe("播放器连接中断，正在重连。");
    expect(playbackAlert({ ...demoSnapshot, notices: ["Windows 凭据库读取失败"] })).toContain("无法读取密钥");
  });
});
