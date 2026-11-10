import type { AppSnapshot } from "./types";

/** Older running backends can still send the retired candidate copy. */
export function candidateWarningLabels(warnings: readonly string[]): string[] {
  return warnings.filter((warning) => !/实验性|非官方|适配器|时长未知|缺少可靠时长/.test(warning)).map((warning) => {
    if (warning.startsWith("版本标签不一致")) return "版本不同";
    if (warning.startsWith("匹配置信度较低")) return "匹配度较低";
    return warning;
  });
}

/** Only problems requiring attention belong in the listening interface. */
export function playbackAlert(snapshot: AppSnapshot): string | null {
  for (const notice of snapshot.notices ?? []) {
    if (notice.startsWith("本地数据库")) return "无法保存数据，请检查磁盘空间和权限后重启。";
    if (notice.startsWith("Windows 凭据库")) return "无法读取密钥，请在设置中重新保存。";
    if (notice.startsWith("媒体连接：")) return "播放器连接中断，正在重连。";
  }
  if (snapshot.phase === "error") return snapshot.message ?? null;
  if (snapshot.phase === "lyrics_only" && snapshot.settings.hasApiKey &&
      snapshot.lyrics?.lines.some((line) => !snapshot.analyses[line.id])) {
    return snapshot.message ?? null;
  }
  return null;
}
