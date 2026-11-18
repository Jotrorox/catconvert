import { type Component, For, Show } from "solid-js";
import type { OutputFormat, VideoCodec } from "../lib/api";
import { CODEC_LABELS, codecHint, codecOf, videoCodecs } from "../lib/formats";

type Props = {
  format: OutputFormat;
  /** The chosen codec; `null` is the format's usual one. */
  codec: VideoCodec | null;
  /** Whether ffmpeg can write `codec`. */
  available: (codec: VideoCodec) => boolean;
  onChange: (codec: VideoCodec | null) => void;
};

/** H.264, H.265, VP9 or AV1, for formats that can hold more than one video codec. */
export const CodecField: Component<Props> = (props) => {
  const codecs = () => videoCodecs(props.format);
  const current = () => codecOf({ format: props.format, videoCodec: props.codec });
  return (
    <Show when={codecs().length > 1}>
      <fieldset class="field codec-field">
        <legend class="field-label">Video codec</legend>
        <div class="format-chips" data-tone="pink">
          <For each={codecs()}>
            {(codec, index) => (
              <button
                type="button"
                class="format-chip"
                aria-pressed={current() === codec}
                disabled={!props.available(codec) && current() !== codec}
                title={
                  props.available(codec)
                    ? undefined
                    : `Your ffmpeg can't write ${CODEC_LABELS[codec]} video`
                }
                // The usual codec is stored as `null`, so recipes follow the format.
                onClick={() => props.onChange(index() === 0 ? null : codec)}
              >
                {CODEC_LABELS[codec]}
              </button>
            )}
          </For>
        </div>
        <Show when={current()}>
          {(codec) => <span class="item-meta">{codecHint(codec())}</span>}
        </Show>
      </fieldset>
    </Show>
  );
};
