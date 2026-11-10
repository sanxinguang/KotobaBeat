import { memo } from "react";
import type { RubySegment } from "./types";

// Keep the source intact while analysis is pending or when older cache data
// has no aligned readings. Never render model-provided markup as HTML.
export const RubyText = memo(function RubyText({
  text,
  segments,
  enabled = true
}: {
  text: string;
  segments?: RubySegment[];
  enabled?: boolean;
}) {
  if (!enabled || !segments?.length || segments.map((part) => part.text).join("") !== text) {
    return <>{text}</>;
  }
  return <span className="furigana-text" lang="ja" aria-label={text}>
    {segments.map((part, index) => part.reading && /[\p{Script=Han}々〆〇]/u.test(part.text) ? (
      <ruby key={index}>{part.text}<rp>（</rp><rt aria-hidden="true">{part.reading}</rt><rp>）</rp></ruby>
    ) : <span key={index}>{part.text}</span>)}
  </span>;
});
