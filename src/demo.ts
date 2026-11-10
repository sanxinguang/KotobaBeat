import type { AppSnapshot, LineAnalysis, LyricCandidate, LyricLine } from "./types";

const texts = [
  "夜明け前の駅で",
  "まだ名前のない風を待っている",
  "昨日より少しだけ",
  "素直になれた気がした",
  "遠ざかる街の灯り",
  "忘れないように目を閉じて",
  "君がくれた言葉を",
  "胸の奥で何度も呼んだ",
  "もし明日が雨でも",
  "この歌は消えないから",
  "新しい空の下で",
  "また笑える日を探そう"
];

const lines: LyricLine[] = texts.map((text, index) => ({
  id: `demo-${index}`,
  startMs: index * 5_500,
  endMs: (index + 1) * 5_500,
  text,
  words: []
}));

const analyses: Record<string, LineAnalysis> = {
  "demo-2": {
    lineId: "demo-2",
    translationZh: "只比昨天稍微更进一步",
    reading: "きのう より すこし だけ",
    furigana: [{ text: "昨日", reading: "きのう" }, { text: "より", reading: "" }, { text: "少", reading: "すこ" }, { text: "しだけ", reading: "" }],
    literalStructure: "昨日 + より（比较基准）+ 少しだけ（限定程度）",
    grammarPoints: [
      {
        surface: "昨日より",
        pattern: "N より",
        level: "N4",
        meaningZh: "比……；以……为比较基准",
        explanationZh: "用「より」标记比较的基准，歌词中省略了后续明确的比较对象。",
        conjugation: null
      },
      {
        surface: "少しだけ",
        pattern: "数量词 + だけ",
        level: "N3",
        meaningZh: "仅仅一点",
        explanationZh: "「だけ」限定程度，让语气显得克制，也为下一句的变化留下余韵。",
        conjugation: null
      }
    ],
    omittedElements: ["与昨天相比发生变化的主体「私」被省略"],
    nuance: "短句在下一行完成语义，形成一种小心翼翼承认成长的语气。"
  },
  "demo-3": {
    lineId: "demo-3",
    translationZh: "感觉自己好像变得坦率了一些",
    reading: "すなお に なれた き が した",
    furigana: [{ text: "素直", reading: "すなお" }, { text: "になれた", reading: "" }, { text: "気", reading: "き" }, { text: "がした", reading: "" }],
    literalStructure: "素直に + なれた（可能形过去）+ 気がした",
    grammarPoints: [
      {
        surface: "なれた",
        pattern: "V可能形 + た",
        level: "N4",
        meaningZh: "变得能够……了",
        explanationZh: "「なる」的可能形「なれる」使用过去式，表示主观上感到已经出现变化。",
        conjugation: "なる → なれる → なれた"
      },
      {
        surface: "気がした",
        pattern: "～気がする",
        level: "N3",
        meaningZh: "感觉好像……",
        explanationZh: "避免把判断说得过满，表现出不确定但真实的内心感受。",
        conjugation: "気がする → 気がした"
      }
    ],
    omittedElements: ["主语「私」"],
    nuance: "可能形与「気がした」叠加，使肯定变得柔和，很符合内省式歌词表达。"
  }
};

for (const [index, line] of lines.entries()) {
  if (!analyses[line.id]) {
    analyses[line.id] = {
      lineId: line.id,
      translationZh: [
        "在黎明前的车站",
        "等待着一阵尚未有名字的风",
        "",
        "",
        "渐渐远去的城市灯火",
        "为了不要忘记而闭上双眼",
        "你曾送给我的话语",
        "在心底一次又一次地呼唤",
        "即使明天下起雨",
        "这首歌也不会消失",
        "在崭新的天空下",
        "去寻找再次展露笑容的那一天吧"
      ][index],
      reading: "",
      literalStructure: "",
      grammarPoints: [],
      omittedElements: [],
      nuance: ""
    };
  }
}

export const demoSnapshot: AppSnapshot = {
  phase: "ready",
  media: {
    sessionId: "demo",
    sourceApp: "KotobaBeat Preview",
    title: "まだ名前のない風",
    artist: "KOTONOHA",
    album: "夜明けの輪郭",
    durationMs: 66_000,
    positionMs: 17_500,
    sampledAtMs: Date.now(),
    playbackRate: 1,
    status: "playing",
    thumbnailDataUrl: null,
    timelineAvailable: true,
    positionEstimated: false,
    capabilities: {
      canPlayPause: true,
      canSkipNext: true,
      canSkipPrevious: true,
      canSeek: true
    }
  },
  sessions: [],
  lyrics: {
    cacheKey: "demo",
    track: {
      title: "まだ名前のない風",
      artist: "KOTONOHA",
      album: "夜明けの輪郭",
      sourceApp: "KotobaBeat Preview",
      durationMs: 66_000,
      normalizedTitle: "まだ名前のない風",
      normalizedArtist: "kotonoha",
      normalizedAlbum: "夜明けの輪郭",
      versions: []
    },
    provider: "演示数据",
    providerTrackId: "demo",
    syncType: "line",
    instrumental: false,
    offsetMs: 0,
    lines
  },
  candidates: [],
  analyses,
  settings: {
    deepseekModel: "deepseek-v4-flash",
    learningLevel: "N4-N2",
    reduceMotion: false,
    ambientMotionEnabled: true,
    lyricsFuriganaEnabled: false,
    miniAlwaysOnTop: true,
    monitoringEnabled: true,
    experimentalNetease: true,
    experimentalQq: true,
    hasApiKey: false
  },
  message: null,
  generation: 1
};

const demoCover = (from: string, to: string, accent: string) =>
  `data:image/svg+xml,${encodeURIComponent(`
    <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 400 400">
      <defs><linearGradient id="g" x2="1" y2="1"><stop stop-color="${from}"/><stop offset="1" stop-color="${to}"/></linearGradient></defs>
      <rect width="400" height="400" rx="32" fill="url(#g)"/>
      <circle cx="282" cy="120" r="64" fill="${accent}" opacity=".72"/>
      <path d="M70 292c58-92 106-126 174-168M42 218c84 3 155 45 226 126" fill="none" stroke="white" stroke-opacity=".3" stroke-width="3"/>
    </svg>
  `)}`;

const demoCandidates: LyricCandidate[] = [
  ["lrclib", "まだ名前のない風", "KOTONOHA", "夜明けの輪郭", 0.86, "#7A526D", "#263B5B", "#F3A7B8"],
  ["netease", "まだ名前のない風 (Album Ver.)", "KOTONOHA", "言葉の朝", 0.82, "#304A61", "#171E2B", "#8DB6C8"],
  ["qq", "まだ名前のない風", "KOTONOHA", "夜明け Live", 0.74, "#765C45", "#29231E", "#E3B77C"]
].map(([provider, title, artist, album, score, from, to, accent], index) => ({
  candidateId: `candidate-${index}`,
  providerTrackId: `preview-${index}`,
  provider: String(provider),
  title: String(title),
  artist: String(artist),
  album: String(album),
  durationMs: 66_000 + index * 2_200,
  instrumental: false,
  syncType: index === 2 ? "plain" : "line",
  lines,
  score: Number(score),
  scoreBreakdown: {
    title: 0.96,
    artist: 1,
    duration: 0.9 - index * 0.08,
    album: 0.8,
    sourceAffinity: 0.7,
    syncQuality: index === 2 ? 0.25 : 0.85,
    penalties: index === 2 ? 0.2 : 0
  },
  warnings: [],
  artworkUrl: demoCover(String(from), String(to), String(accent))
}));

export const demoCandidateSnapshot: AppSnapshot = {
  ...demoSnapshot,
  phase: "candidate_required",
  candidates: demoCandidates,
  message: "请选择歌词与封面"
};
