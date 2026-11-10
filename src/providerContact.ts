/** This value becomes a public application contact in provider request headers. */
export function validateProviderContact(value: string): string | null {
  const contact = value.trim();
  if (!contact) return null;
  if (contact.length > 256 || /[^\x21-\x7e]/.test(contact)) {
    return "请填写公开邮箱或 HTTPS 项目地址。";
  }
  if (/^[a-z0-9._+-]+@(?:[a-z0-9-]+\.)+[a-z0-9-]+$/i.test(contact)) return null;
  try {
    const url = new URL(contact);
    if (url.protocol === "https:" && url.hostname.includes(".") &&
        !url.username && !url.password && !url.search && !url.hash) return null;
  } catch {
    // Report a friendly format error rather than the browser's parser message.
  }
  return "请输入有效的邮箱或 HTTPS 项目地址。";
}
