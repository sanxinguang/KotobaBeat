/** Do not steal Space from form fields, focused buttons or an open dialog. */
export function isPlaybackShortcut(event: KeyboardEvent, modalOpen: boolean): boolean {
  if (event.code !== "Space" || event.repeat || event.defaultPrevented || modalOpen) return false;
  if (event.ctrlKey || event.altKey || event.metaKey || event.shiftKey) return false;
  return !(event.target instanceof Element && event.target.closest(
    "input, textarea, select, button, a, [contenteditable]:not([contenteditable='false']), [role='switch'], [role='dialog']"
  ));
}

export function playerLabel(source: string): string {
  const lower = source.toLowerCase();
  if (lower.includes("cloudmusic")) return "网易云音乐";
  if (lower.includes("qqmusic")) return "QQ 音乐";
  if (lower.includes("spotify")) return "Spotify";
  return "音乐播放器";
}
