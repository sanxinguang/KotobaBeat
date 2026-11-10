import { describe, expect, it } from "vitest";
import { validateProviderContact } from "./providerContact";

describe("provider contact disclosure input", () => {
  it.each(["", "  ", "owner@example.com", "https://example.com/kotobabeat", "https://github.com/example/project"])("accepts a blank value or public contact: %s", (value) => {
    expect(validateProviderContact(value)).toBeNull();
  });

  it.each(["not-a-contact", "sk-test-only", "owner@example.com\r\nX-Test: injected", "http://example.com/project", "https://name:password@example.com/", "https://example.com/?token=secret", "https://example.com/#private", "https://localhost/project"])("rejects malformed or secret-bearing contact: %s", (value) => {
    expect(validateProviderContact(value)).not.toBeNull();
  });
});
