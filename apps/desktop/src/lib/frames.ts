/**
 * Stepping through a video one frame at a time.
 *
 * ffmpeg shows the first frame at or after the time it is given (for the
 * preview and for a saved frame alike), so frame `n`, counting from 0, starts
 * at `n / fps`. Times are aimed a hair below that start: ffmpeg gets them in
 * whole milliseconds, and rounding up past the start would skip a frame.
 */

/** Frame rate assumed for a video that doesn't say. Mirrors FALLBACK_FPS in convert.rs. */
export const FALLBACK_FPS = 30;

/**
 * How far below a frame's start to aim. Rounding to milliseconds moves a time
 * by up to half of this, so it stays inside the frame below about 660 fps.
 */
const NUDGE = 0.001;

/** Index of the frame shown at `time`. */
export function frameIndex(time: number, fps: number): number {
  return Math.max(0, Math.ceil(time * fps - 1e-6));
}

/** A time that shows frame `index`. */
export function frameTime(index: number, fps: number): number {
  return index <= 0 ? 0 : index / fps - NUDGE;
}

/** The time `delta` frames from the one shown at `time`, kept within the clip. */
export function stepFrame(time: number, delta: number, fps: number, duration: number): number {
  const last = Math.max(0, Math.ceil(duration * fps - 1e-6) - 1);
  return frameTime(Math.min(last, Math.max(0, frameIndex(time, fps) + delta)), fps);
}
