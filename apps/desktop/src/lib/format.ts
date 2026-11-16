const UNITS = ["B", "KB", "MB", "GB", "TB"] as const;

/** Human-readable size using 1000-based units, e.g. `1.2 MB`. */
export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes < 0) return "–";
  let value = bytes;
  let unit = 0;
  while (value >= 1000 && unit < UNITS.length - 1) {
    value /= 1000;
    unit += 1;
  }
  const digits = unit === 0 || value >= 100 ? 0 : 1;
  return `${value.toFixed(digits)} ${UNITS[unit]}`;
}

/**
 * Clock time: `0:42`, `12:03`, `1:02:03`. With `precise`, tenths of a second
 * are added (`0:03.2`).
 */
export function formatClock(secs: number, precise = false): string {
  if (!Number.isFinite(secs) || secs < 0) return "–";
  const tenths = Math.round(secs * 10);
  const whole = precise ? Math.floor(tenths / 10) : Math.round(secs);
  const h = Math.floor(whole / 3600);
  const m = Math.floor((whole % 3600) / 60);
  const s = whole % 60;
  const ss = String(s).padStart(2, "0");
  const base = h > 0 ? `${h}:${String(m).padStart(2, "0")}:${ss}` : `${m}:${ss}`;
  return precise ? `${base}.${tenths % 10}` : base;
}

/** Parse `42`, `3.2`, `1:05`, `0:03.2` or `1:02:03.5` into seconds; `null` if malformed. */
export function parseClock(text: string): number | null {
  const parts = text.trim().split(":");
  if (parts.length === 0 || parts.length > 3) return null;
  let total = 0;
  for (const [index, part] of parts.entries()) {
    const isLast = index === parts.length - 1;
    if (!(isLast ? /^\d+(\.\d+)?$/ : /^\d+$/).test(part)) return null;
    total = total * 60 + Number(part);
  }
  return total;
}

/** Short duration for summaries: `27.8 s`, `4 min 05 s`. */
export function formatSpan(secs: number): string {
  if (secs < 60) return `${(Math.round(secs * 10) / 10).toString()} s`;
  const m = Math.floor(secs / 60);
  const s = Math.round(secs % 60);
  return `${m} min ${String(s).padStart(2, "0")} s`;
}

/** Last path segment, handling both `/` and `\` separators. */
export function baseName(path: string): string {
  return path.split(/[\\/]/).pop() || path;
}

/** "2 min ago", "Yesterday", or a date. */
export function formatWhen(timestamp: number, now = Date.now()): string {
  const diff = Math.max(0, now - timestamp) / 1000;
  if (diff < 45) return "Just now";
  if (diff < 3600) return `${Math.max(1, Math.round(diff / 60))} min ago`;
  if (diff < 86_400) return `${Math.round(diff / 3600)} h ago`;
  if (diff < 172_800) return "Yesterday";
  return new Date(timestamp).toLocaleDateString(undefined, { month: "short", day: "numeric" });
}
