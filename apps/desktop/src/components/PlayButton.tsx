import { type Component, createEffect, on, Show } from "solid-js";
import type { OutputFormat } from "../lib/api";
import { formatClock } from "../lib/format";
import { createPlayer, outputType } from "../lib/player";
import { toast } from "../lib/store";
import { Icon } from "./Icon";

type Props = {
  /** A sound file CatConvert wrote. */
  path: string;
  /** What to call it in labels. */
  name: string;
  format: OutputFormat;
  disabled?: boolean;
};

/**
 * Play/pause for a finished sound file in the Queue or History, with how far
 * it got once started. Stops when the row goes away.
 */
export const PlayButton: Component<Props> = (props) => {
  const player = createPlayer(() => props.path, { type: () => outputType(props.format) });
  createEffect(
    on(player.error, (error) => {
      if (error) toast(`Couldn't play ${props.name}: ${error}`, { tone: "error" });
    }),
  );
  const started = () => player.playing() || player.time() > 0;
  return (
    <>
      <Show when={started()}>
        <span class="play-time mono">
          {formatClock(player.time())} / {formatClock(player.duration() ?? Number.NaN)}
        </span>
      </Show>
      <button
        type="button"
        class="icon-btn plain"
        aria-label={`${player.playing() ? "Pause" : "Play"} ${props.name}`}
        title={player.playing() ? "Pause" : "Play"}
        aria-busy={player.loading()}
        disabled={props.disabled}
        onClick={() => player.toggle()}
      >
        <Icon name={player.playing() ? "pause" : "play"} size={18} />
      </button>
    </>
  );
};
