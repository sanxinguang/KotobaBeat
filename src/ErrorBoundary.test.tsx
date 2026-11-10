// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { ErrorBoundary } from "./ErrorBoundary";

it("keeps a recoverable screen instead of a blank window after a render exception", async () => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  const silent = vi.spyOn(console, "error").mockImplementation(() => {});
  const host = document.createElement("div");
  const root = createRoot(host);
  function Broken(): never { throw new Error("Synthetic render failure"); }
  try {
    await act(async () => root.render(<ErrorBoundary><Broken /></ErrorBoundary>));
    expect(host.querySelector("[role='alert']")?.textContent).toContain("重新加载界面");
    expect(host.textContent).toContain("加载失败，请重试。");
    expect(host.textContent).not.toContain("Synthetic render failure");
  } finally {
    await act(async () => root.unmount());
    silent.mockRestore();
  }
});
