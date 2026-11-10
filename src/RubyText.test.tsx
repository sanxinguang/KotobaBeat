// @vitest-environment jsdom
import { describe, expect, it } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import { RubyText } from "./RubyText";

const text = "昨日より少しだけ";
const segments = [{ text: "昨日", reading: "きのう" }, { text: "より", reading: "" }, { text: "少", reading: "すこ" }, { text: "しだけ", reading: "" }];

describe("furigana", () => {
  it("annotates compounds and kanji stems without duplicating okurigana", () => {
    const container = document.createElement("div");
    container.innerHTML = renderToStaticMarkup(<RubyText text={text} segments={segments} />);
    expect([...container.querySelectorAll("rt")].map((rt) => rt.textContent)).toEqual(["きのう", "すこ"]);
    container.querySelectorAll("rt, rp").forEach((element) => element.remove());
    expect(container.textContent).toBe(text);
  });

  it("keeps plain lyrics when disabled, pending, or the segments mismatch", () => {
    expect(renderToStaticMarkup(<RubyText text={text} segments={segments} enabled={false} />)).toBe(text);
    expect(renderToStaticMarkup(<RubyText text={text} />)).toBe(text);
    expect(renderToStaticMarkup(<RubyText text="別の歌詞" segments={segments} />)).toBe("別の歌詞");
  });

  it("does not annotate kana or render supplied text as HTML", () => {
    const output = renderToStaticMarkup(<RubyText text="かな<script>" segments={[{ text: "かな<script>", reading: "かな" }]} />);
    expect(output).not.toContain("<ruby>");
    expect(output).not.toContain("<script>");
    expect(output).toContain("&lt;script&gt;");
  });
});
