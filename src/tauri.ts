import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { AppSnapshot, PublicSettings, SettingsPatch } from "./types";

export const isTauri = () => "__TAURI_INTERNALS__" in window;

export const commands = {
  snapshot: () => invoke<AppSnapshot>("get_snapshot"),
  selectSession: (sourceApp?: string | null) =>
    invoke<void>("select_session", { sourceApp: sourceApp ?? null }),
  togglePlayPause: () => invoke<void>("toggle_play_pause"),
  skipNext: () => invoke<void>("skip_next"),
  skipPrevious: () => invoke<void>("skip_previous"),
  seek: (positionMs: number) => invoke<void>("seek_to", { positionMs }),
  chooseCandidate: (candidateId: string, artworkCandidateId?: string | null) =>
    invoke<void>("choose_candidate", {
      candidateId,
      artworkCandidateId: artworkCandidateId ?? null
    }),
  importLrc: (path: string) => invoke<void>("import_lrc", { path }),
  pasteLyrics: (raw: string) => invoke<void>("paste_lyrics", { raw }),
  refresh: () => invoke<void>("force_refresh"),
  setOffset: (offsetMs: number) => invoke<void>("set_lyric_offset", { offsetMs }),
  saveDeepSeekKey: (apiKey: string) =>
    invoke<void>("save_deepseek_key", { apiKey }),
  clearDeepSeekKey: () => invoke<void>("clear_deepseek_key"),
  retryAnalysis: () => invoke<void>("retry_analysis"),
  updateSettings: (patch: SettingsPatch) =>
    invoke<PublicSettings>("update_settings", { patch }),
  showWindow: (label: "main" | "mini") =>
    invoke<void>("show_window", { label }),
  hideWindow: (label: "main" | "mini") =>
    invoke<void>("hide_window", { label }),
  quit: () => invoke<void>("quit_app")
};

export function onSnapshot(callback: (snapshot: AppSnapshot) => void) {
  return listen<AppSnapshot>("state://snapshot", (event) => callback(event.payload));
}
