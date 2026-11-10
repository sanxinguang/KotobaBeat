export type AppPhase =
  | "no_session"
  | "identifying"
  | "resolving_lyrics"
  | "candidate_required"
  | "analyzing"
  | "ready"
  | "lyrics_only"
  | "error";

export interface PlaybackCapabilities {
  canPlayPause: boolean;
  canSkipNext: boolean;
  canSkipPrevious: boolean;
  canSeek: boolean;
}

export interface SessionSummary {
  id: string;
  sourceApp: string;
  title: string;
  artist: string;
  isCurrent: boolean;
}

export interface MediaState {
  sessionId: string;
  sourceApp: string;
  title: string;
  artist: string;
  album: string;
  durationMs: number;
  positionMs: number;
  sampledAtMs: number;
  playbackRate: number;
  status: string;
  thumbnailDataUrl?: string | null;
  timelineAvailable: boolean;
  positionEstimated: boolean;
  capabilities: PlaybackCapabilities;
}

export interface TrackIdentity {
  title: string;
  artist: string;
  album: string;
  sourceApp: string;
  durationMs: number;
  normalizedTitle: string;
  normalizedArtist: string;
  normalizedAlbum: string;
  versions: string[];
}

export interface WordTiming {
  text: string;
  startMs: number;
  endMs: number;
}

export interface LyricLine {
  id: string;
  startMs?: number | null;
  endMs?: number | null;
  text: string;
  words: WordTiming[];
}

export type SyncType = "word" | "line" | "plain";

export interface ScoreBreakdown {
  title: number;
  artist: number;
  duration: number;
  album: number;
  sourceAffinity: number;
  syncQuality: number;
  penalties: number;
}

export interface LyricCandidate {
  candidateId: string;
  provider: string;
  providerTrackId: string;
  title: string;
  artist: string;
  album: string;
  durationMs: number;
  instrumental: boolean;
  syncType: SyncType;
  lines: LyricLine[];
  score: number;
  scoreBreakdown: ScoreBreakdown;
  warnings: string[];
  artworkUrl?: string | null;
}

export interface LyricsDocument {
  cacheKey: string;
  track: TrackIdentity;
  provider: string;
  providerTrackId: string;
  syncType: SyncType;
  instrumental: boolean;
  offsetMs: number;
  lines: LyricLine[];
  artworkUrl?: string | null;
}

export interface GrammarPoint {
  surface: string;
  pattern: string;
  level: string;
  meaningZh: string;
  explanationZh: string;
  conjugation?: string | null;
}

export interface RubySegment {
  text: string;
  reading: string;
}

export interface LineAnalysis {
  lineId: string;
  translationZh: string;
  reading: string;
  furigana?: RubySegment[];
  literalStructure: string;
  grammarPoints: GrammarPoint[];
  omittedElements: string[];
  nuance: string;
}

export interface PublicSettings {
  deepseekModel: string;
  learningLevel: string;
  reduceMotion: boolean;
  ambientMotionEnabled: boolean;
  lyricsFuriganaEnabled: boolean;
  miniAlwaysOnTop: boolean;
  monitoringEnabled: boolean;
  experimentalNetease: boolean;
  experimentalQq: boolean;
  providerContact?: string;
  hasApiKey: boolean;
}

export interface AppSnapshot {
  phase: AppPhase;
  media?: MediaState | null;
  sessions: SessionSummary[];
  lyrics?: LyricsDocument | null;
  candidates: LyricCandidate[];
  analyses: Record<string, LineAnalysis>;
  settings: PublicSettings;
  message?: string | null;
  generation: number;
  revision?: number;
  selectedSource?: string | null;
  notices?: string[];
}

export type SettingsPatch = Partial<
  Pick<
    PublicSettings,
    | "deepseekModel"
    | "learningLevel"
    | "reduceMotion"
    | "ambientMotionEnabled"
    | "lyricsFuriganaEnabled"
    | "miniAlwaysOnTop"
    | "monitoringEnabled"
    | "experimentalNetease"
    | "experimentalQq"
    | "providerContact"
  >
>;
