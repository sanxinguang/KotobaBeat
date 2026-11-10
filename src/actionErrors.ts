import { useEffect, useState } from "react";
import { isTauri } from "./tauri";

const ACTION_ERROR = "kotobabeat:action-error";

export function errorMessage(error: unknown): string {
  const message = error instanceof Error ? error.message : String(error ?? "操作失败");
  return message.replace(/sk-[a-z0-9_-]+/gi, "[凭据已隐藏]").slice(0, 500);
}

export async function runAction(action: () => Promise<unknown>): Promise<boolean> {
  if (!isTauri()) return false;
  try {
    await action();
    return true;
  } catch (error) {
    window.dispatchEvent(new CustomEvent(ACTION_ERROR, { detail: errorMessage(error) }));
    return false;
  }
}

export function useActionError() {
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    const listener = (event: Event) => setError((event as CustomEvent<string>).detail);
    window.addEventListener(ACTION_ERROR, listener);
    return () => window.removeEventListener(ACTION_ERROR, listener);
  }, []);
  return { actionError: error, dismissActionError: () => setError(null) };
}
