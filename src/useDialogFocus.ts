import { useEffect, useRef } from "react";

const FOCUSABLE = "button:not(:disabled), input:not(:disabled), textarea:not(:disabled), select:not(:disabled), a[href], [tabindex='0']";

export function useDialogFocus<T extends HTMLElement = HTMLDivElement>() {
  const ref = useRef<T | null>(null);
  useEffect(() => {
    const dialog = ref.current;
    if (!dialog) return;
    const previous = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    const elements = () => [...dialog.querySelectorAll<HTMLElement>(FOCUSABLE)];
    const focus = (target: HTMLElement, reveal = false) => {
      // The settings drawer enters with translateX. Native focus scrolling during
      // that animation can otherwise move even overflow:hidden app ancestors.
      target.focus({ preventScroll: true });
      if (!reveal) return;
      // Tab wrapping may need to reveal a control in a long drawer, but must not
      // scroll horizontally or move any ancestor outside this dialog.
      let scroller = target.parentElement;
      while (scroller && dialog.contains(scroller)) {
        if (/^(auto|scroll)$/.test(getComputedStyle(scroller).overflowY)) {
          const bounds = target.getBoundingClientRect();
          const viewport = scroller.getBoundingClientRect();
          if (bounds.top < viewport.top) scroller.scrollTop += bounds.top - viewport.top;
          else if (bounds.bottom > viewport.bottom) scroller.scrollTop += bounds.bottom - viewport.bottom;
        }
        if (scroller === dialog) break;
        scroller = scroller.parentElement;
      }
    };
    focus(dialog.querySelector<HTMLElement>("[autofocus]") ?? elements()[0] ?? dialog);
    const trap = (event: KeyboardEvent) => {
      if (event.key !== "Tab") return;
      const items = elements();
      const first = items[0];
      const last = items.at(-1);
      if (!first || !last) {
        event.preventDefault();
        focus(dialog);
      } else if (event.shiftKey && (document.activeElement === first || !dialog.contains(document.activeElement))) {
        event.preventDefault();
        focus(last, true);
      } else if (!event.shiftKey && (document.activeElement === last || !dialog.contains(document.activeElement))) {
        event.preventDefault();
        focus(first, true);
      }
    };
    dialog.addEventListener("keydown", trap);
    return () => {
      dialog.removeEventListener("keydown", trap);
      if (previous?.isConnected) previous.focus({ preventScroll: true });
    };
  }, []);
  return ref;
}
