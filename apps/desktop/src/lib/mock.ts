/**
 * Static sample data for the UI mockup. Nothing here comes from the backend yet;
 * screens render these values until conversion is wired up.
 */

export type Tone = "pink" | "green" | "yellow" | "lavender";

export const TARGET_FORMATS = ["MP4", "WEBM", "GIF", "MP3", "MOV"] as const;
export type TargetFormat = (typeof TARGET_FORMATS)[number];

export type Recipe = {
  id: string;
  title: string;
  detail: string;
  shortDetail: string;
  route: string;
  tone: Tone;
  format: TargetFormat;
};

export const RECIPES: readonly Recipe[] = [
  {
    id: "gif",
    title: "Make a GIF",
    detail: "Loop a clip, 480px, 15 fps",
    shortDetail: "Loop a clip",
    route: "MP4 → GIF",
    tone: "pink",
    format: "GIF",
  },
  {
    id: "shrink",
    title: "Shrink to share",
    detail: "Under 25 MB for chats",
    shortDetail: "Under 25 MB",
    route: "ANY → MP4",
    tone: "green",
    format: "MP4",
  },
  {
    id: "audio",
    title: "Pull the audio",
    detail: "Keep just the sound",
    shortDetail: "Just the sound",
    route: "VIDEO → MP3",
    tone: "yellow",
    format: "MP3",
  },
  {
    id: "web",
    title: "Web friendly",
    detail: "Small, fast, plays anywhere",
    shortDetail: "Plays anywhere",
    route: "ANY → WEBM",
    tone: "lavender",
    format: "WEBM",
  },
];

export const SUPPORTED_CHIPS = ["MP4", "MOV", "MKV", "WEBM", "GIF", "MP3", "WAV", "FLAC", "PNG"];

export type RecentItem = {
  name: string;
  kind: "video" | "audio";
  route: string;
  from: string;
  to: string;
  when: string;
};

/** Most recent conversion; the compact Convert screen shows only this one. */
export const LATEST: RecentItem = {
  name: "garden_zoomies.mp4",
  kind: "video",
  route: "MOV → MP4",
  from: "48 MB",
  to: "11 MB",
  when: "2 min ago",
};

export const RECENT: readonly RecentItem[] = [
  LATEST,
  {
    name: "purr_recording.mp3",
    kind: "audio",
    route: "WAV → MP3",
    from: "31 MB",
    to: "3 MB",
    when: "Yesterday",
  },
];

export const SAMPLE_FILE = {
  name: "garden_zoomies.mov",
  meta: "1920×1080 · 00:42 · H.264 · 48 MB",
  shortMeta: "1080p · 00:42 · 48 MB",
  trim: {
    start: "00:00:03.2",
    end: "00:00:31.0",
    summary: "Keeping 00:03.2 – 00:31.0 · 27.8 s",
    short: "00:03 – 00:31",
    /** Selected range as percentages of the clip length. */
    fromPct: 7.6,
    toPct: 73.8,
  },
  sizes: ["Original (1080p)", "720p", "480p", "360p"],
  defaultSize: "720p",
  frameRates: ["Keep (30)", "24", "15", "10"],
  audio: "AAC · 128 kbps",
  estimate: { size: "About 7 MB", saving: "85% lighter" },
  command:
    "ffmpeg -ss 3.2 -to 31 -i garden_zoomies.mov -vf scale=-2:720 -c:v libx264 -crf 24 -c:a aac -b:a 128k garden_zoomies.mp4",
};

export const QUEUE = {
  summary: "3 to go · about 1 min left",
  active: {
    name: "garden_zoomies.mov → MP4",
    percent: 64,
    speed: "2.3×",
    frames: "534",
    totalFrames: "834",
    left: "00:18",
    written: "4.6 MB",
  },
  upNext: [
    {
      position: 2,
      name: "birthday_cake_pounce.mkv",
      route: "MKV → WEBM",
      detail: "MKV → WEBM · 1080p · 1:12",
    },
    {
      position: 3,
      name: "window_bird_watch.mp4",
      route: "MP4 → GIF",
      detail: "MP4 → GIF · 480px · 15 fps · 0:09",
    },
  ],
  finished: {
    name: "purr_recording.mp3",
    detail: "WAV → MP3 · 31 MB → 3 MB · took 6 s",
    short: "31 MB → 3 MB",
  },
  failed: {
    name: "old_vhs_kitten.avi",
    message: "Hmm, the audio track looks scrambled. Try again without audio?",
  },
};

/** Jobs that are running or waiting; shown as the Queue badge. */
export const QUEUE_COUNT = 1 + QUEUE.upNext.length;
