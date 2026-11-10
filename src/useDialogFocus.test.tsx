// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { useDialogFocus } from "./useDialogFocus";

it("prevents focus from scrolling the page during drawer entry, Tab wrap and restoration", async () => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  const launcher = document.createElement("button");
  const host = document.createElement("div");
  document.body.append(launcher, host);
  launcher.focus();
  const focus = vi.spyOn(HTMLElement.prototype, "focus");
  const root = createRoot(host);
  function Drawer() {
    const ref = useDialogFocus();
    return <div ref={ref} role="dialog"><button>First</button><button>Last</button></div>;
  }
  try {
    await act(async () => root.render(<Drawer />));
    expect(document.activeElement?.textContent).toBe("First");
    expect(focus).toHaveBeenLastCalledWith({ preventScroll: true });
    const buttons = host.querySelectorAll("button");
    buttons[1].focus({ preventScroll: true });
    await act(async () => {
      buttons[1].dispatchEvent(new KeyboardEvent("keydown", { key: "Tab", bubbles: true, cancelable: true }));
    });
    expect(document.activeElement).toBe(buttons[0]);
    await act(async () => root.unmount());
    expect(document.activeElement).toBe(launcher);
    expect(focus.mock.calls.every(([options]) => options?.preventScroll === true)).toBe(true);
  } finally {
    focus.mockRestore();
    launcher.remove();
    host.remove();
  }
});
