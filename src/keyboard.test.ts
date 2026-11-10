// @vitest-environment jsdom
import { describe, expect, it } from "vitest";
import { isPlaybackShortcut, playerLabel } from "./keyboard";

function spaceOn(element: Element, options: KeyboardEventInit = {}) {
  const event = new KeyboardEvent("keydown", { code: "Space", bubbles: true, ...options });
  element.dispatchEvent(event);
  return event;
}

describe("safe keyboard playback control", () => {
  it("allows unmodified Space on the background", () => {
    expect(isPlaybackShortcut(spaceOn(document.body), false)).toBe(true);
  });

  it.each(["input", "textarea", "select", "button", "a"])("does not steal Space from %s", (tag) => {
    const target = document.createElement(tag);
    expect(isPlaybackShortcut(spaceOn(target), false)).toBe(false);
  });

  it("respects child elements, editable surfaces, dialog focus, repeats and modifier shortcuts", () => {
    const button = document.createElement("button");
    const label = document.createElement("span");
    button.append(label);
    expect(isPlaybackShortcut(spaceOn(label), false)).toBe(false);
    const editable = document.createElement("div");
    editable.setAttribute("contenteditable", "true");
    expect(isPlaybackShortcut(spaceOn(editable), false)).toBe(false);
    expect(isPlaybackShortcut(spaceOn(document.body), true)).toBe(false);
    expect(isPlaybackShortcut(spaceOn(document.body, { repeat: true }), false)).toBe(false);
    expect(isPlaybackShortcut(spaceOn(document.body, { ctrlKey: true }), false)).toBe(false);
  });

  it("never exposes executable names as player labels", () => {
    expect(playerLabel("cloudmusic.exe")).toBe("网易云音乐");
    expect(playerLabel("QQMusic.exe")).toBe("QQ 音乐");
    expect(playerLabel("unknown.exe")).toBe("音乐播放器");
  });
});
