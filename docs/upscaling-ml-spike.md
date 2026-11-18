# ML upscaling: backend spike

Research for [#19](https://github.com/Jotrorox/catconvert/issues/19), step 2: an
offline AI/ML super-resolution backend for photos and videos. State as of
November 2026. Sizes come from the release assets themselves (downloaded and
listed); licenses come from each repository's `LICENSE`. Follow-up work is
tracked in [#35](https://github.com/Jotrorox/catconvert/issues/35).

## Summary

**Recommendation:** Real-ESRGAN models run by an ncnn + Vulkan helper, shipped as
an **optional download**, not inside the installer.

1. **Photos first.** Build `realesrgan-ncnn-vulkan` (xinntao, MIT) from source
   in our own CI next to the media tools, with an up-to-date ncnn (BSD-3-Clause).
   Run it as a new `Tool::Upscale` step, the same way `cjxl` runs. Models:
   `realesrgan-x4plus` (photos, 32 MB) and `realesrgan-x4plus-anime` (art,
   8.5 MB). The weights are most likely BSD-3-Clause; see
   [Weights license](#weights-license).
2. **Video second.** Use the fast `realesr-animevideov3` model (1.2 MB per
   scale) and the compact `realesr-general-x4v3` for real footage. Keep video
   frames off disk: a small helper of our own (ncnn, raw RGB frames on
   stdin/stdout) sits between two ffmpeg processes. Progress then comes from the
   encoding ffmpeg's `-progress` output, which the core already parses.
3. ### Weights license

Checked against the primary sources in November 2026.

**Findings**

- **No explicit grant.** No file, README, model zoo page or release note says
  what license the weights are under. The repo has one license file, BSD-3-Clause,
  "Copyright (c) 2021, Xintao Wang", and every model is a release asset of that
  repo. The ncnn `.param`/`.bin` conversions also ship inside the MIT-licensed
  ncnn port's release zips.
- **Timing:**
  - The first commit (2021-07-19) had an MIT license. It was removed on
    2021-07-22.
  - `RealESRGAN_x4plus.pth` was published in v0.1.0 on 2021-07-22, with no
    license in the repo.
  - BSD-3-Clause was added on 2021-07-25. Someone asked for a license file
    ([#4](https://github.com/xinntao/Real-ESRGAN/issues/4)) and xinntao
    answered "I have added the BSD-3-Clause License".
  - `x4plus_anime_6B` (2021-08) and `realesr-animevideov3` and
    `realesr-general-x4v3` (2022-04) were all released while BSD-3 was in
    place.
- **The author's only statement is #4,** and it is about the repository as a
  whole. Questions that ask specifically about the weights are still
  unanswered. The project has been dormant since v0.3.0 (2022-09).
  - [#677](https://github.com/xinntao/Real-ESRGAN/issues/677) has been open
    since 2023-08, with follow-ups through 2026-11.
  - [#1016](https://github.com/xinntao/Real-ESRGAN/issues/1016) and
    [#1018](https://github.com/xinntao/Real-ESRGAN/issues/1018) are also
    open.
- **Minor inconsistency:** `setup.py` declares `license='BSD-3-Clause License'`
  but carries the classifier `License :: OSI Approved :: Apache Software License`.
- **"OpenMMLab AI Model License":** one issue
  ([#1004](https://github.com/xinntao/Real-ESRGAN/issues/1004)) says the anime
  video model is under it. No source backs that up: nothing in the repo or the
  releases mentions it, and OpenModelDB lists the model as BSD-3-Clause. It
  looks mistaken.
- **Training data:** the models were trained on DF2K (DIV2K + Flickr2K) and
  OST. DIV2K is "made available for academic research purpose only" and its
  images belong to their owners. Whether dataset terms carry over to trained
  weights is legally unsettled. The usual reading is that they don't, but it is
  a residual risk that no license file can remove.
- **Practice:** the weights are redistributed widely as BSD-3-Clause, with no
  known objection from the author. Examples are OpenModelDB, AMD's Hugging Face
  re-host, chaiNNer, Video2X and the official ncnn zips themselves.

**Conclusion.** BSD-3-Clause is the only license the author ever attached to
anything they published with these weights, so it is the most defensible
reading. It is not certain. For an open-source, non-commercial, optional
feature the risk is low.

**How to keep the risk small**

1. **Don't redistribute the weights.** Since an in-app download is fine, the
   pack can hold only our build of the binary (MIT + BSD-3 code). The app then
   fetches the model files straight from xinntao's official GitHub release
   assets, checked against SHA-256 hashes compiled into the app. Only the
   official ncnn zips hold `.param`/`.bin`, so either extract just `models/`
   from the official `realesrgan-ncnn-vulkan-20220424-*.zip`, or convert the
   `.pth` locally. That way CatConvert never hosts or ships the weights itself.
2. **Attribute anyway.** List the weights in the third-party notices as
   "Real-ESRGAN models © 2021 Xintao Wang, BSD-3-Clause (as published with
   xinntao/Real-ESRGAN)", with the full BSD-3 text and the paper citation.
3. **Pick the models with the cleanest history.** Prefer models released while
   the BSD-3 file was in place (`realesr-general-x4v3`,
   `realesr-animevideov3`, `x4plus_anime_6B`) over `x4plus`, which was
   published three days before the file appeared. That difference is cosmetic,
   since the author attached the license to the same repository right after.
4. Optionally add our own comment to #677. It is unlikely to be answered.

**Optional download.** The pack is about 45 MB per platform zipped (binary
plus four models). Install it from _Settings → Upscaling_, with an
"install from file" path for offline machines. Verify it against a SHA-256
compiled into the app. This keeps the installers at their current size and
the default app fully offline.

**Rejected for now:** ffmpeg's `dnn_processing`/`sr` filters (heavy runtimes, no
GPU on macOS, no tiling, dated models). waifu2x and Real-CUGAN (anime only,
Real-CUGAN unmaintained). upscayl-ncnn (AGPL-3.0). See below.
**Worth watching:** Apple's `VTSuperResolutionScaler` (macOS 26, zero bundle
cost) as a later Mac-only fast path, and ONNX Runtime if we ever need Core ML
or DirectML instead of Vulkan.

**Set expectations in the UI:** a 12 MP photo at 4× takes tens of seconds to
minutes on a typical GPU. Video at 1080p runs at roughly 1–3 fps with the
compact models on a mid-range GPU, so a 10-minute clip takes hours. Without a
Vulkan GPU, ML upscaling is unavailable (or CPU-only, very slow) and Lanczos
from step 1 stays the fallback.

## Comparison

| Backend                                    | Code license                               | Weights license                                              | Binary (mac univ. / Linux / Win)                                          | Models                                                       | GPU / CPU                                                                 | Last release                             |
| ------------------------------------------ | ------------------------------------------ | ------------------------------------------------------------ | ------------------------------------------------------------------------- | ------------------------------------------------------------ | ------------------------------------------------------------------------- | ---------------------------------------- |
| **realesrgan-ncnn-vulkan** (xinntao)       | MIT (+ realsr-ncnn-vulkan MIT, ncnn BSD-3) | BSD-3-Clause (Real-ESRGAN repo; no separate weights license) | 25.5 MB / 10.9 MB / 5.9 MB + `vcomp140.dll`                               | x4plus 31.9 MB, x4plus-anime 8.5 MB, animevideov3 3 × 1.2 MB | Vulkan only, **no CPU mode**                                              | v0.2.0, 2022-04 (ncnn pinned 2022-04-21) |
| **waifu2x-ncnn-vulkan** (nihui)            | MIT                                        | MIT (nagadomi/waifu2x)                                       | 22.4 MB / 7.9 MB / 4.9 MB                                                 | cunet 9 × 2.6 MB, upconv_7 photo 5 × 1.1 MB                  | Vulkan, CPU via `-g -1`                                                   | 20250915 (repo active 2026-04)           |
| **realcugan-ncnn-vulkan** (nihui)          | MIT                                        | MIT (bilibili/ailab Real-CUGAN)                              | – / 9.3 MB / –                                                            | ~2.5 MB each, ~20 files                                      | Vulkan, CPU via `-g -1`                                                   | 20220728 (repo idle since 2023-03)       |
| **upscayl-ncnn** (`upscayl-bin`)           | **AGPL-3.0**                               | varies; several upscayl models are CC BY-NC-SA               | 27.0 MB / 11.4 MB / 7.4 MB                                                | none bundled                                                 | Vulkan only                                                               | 20251207                                 |
| **ffmpeg `dnn_processing`** (9.0.2)        | LGPL/GPL (ours: GPL)                       | depends on model                                             | ffmpeg + runtime: ORT ~10–80 MB, OpenVINO ~100 MB+, TF/LibTorch ~0.5–2 GB | ESPCN/SRCNN `.pb` <1 MB, or any ONNX                         | ORT: CPU, CUDA, DirectML, Vitis AI; **no Core ML**                        | ffmpeg 9.0 (ORT backend new in 9.0)      |
| **ffmpeg `sr`**                            | as above                                   | ESPCN/SRCNN (train yourself)                                 | TensorFlow C lib                                                          | tiny                                                         | TensorFlow only                                                           | legacy                                   |
| **ONNX Runtime helper** (own, `ort` crate) | ORT MIT, `ort` Apache-2.0                  | BSD-3 (Real-ESRGAN, exported ONNX)                           | ORT ~10 MB (macOS arm64) to ~78 MB (Win x64 CPU)                          | x4plus fp16 ~33 MB                                           | Core ML (mac), DirectML/WinML (Win), CPU; Linux GPU needs CUDA (~370 MB+) | ORT ships monthly                        |
| **Apple VTSuperResolutionScaler**          | OS API                                     | OS models, downloaded by macOS                               | 0 (small Swift helper)                                                    | 0                                                            | Apple GPU/ANE, macOS 26+ only                                             | macOS 26                                 |

### Speed ballpark (to be measured during implementation)

The upstream projects publish almost no comparable numbers. The figures below
combine the few measured data points (cited) with the models' compute cost:
x4plus is about 16.7 M parameters, roughly 36 MFLOP per input pixel, so about
75 TFLOP for one 1080p input frame. animevideov3 is about 0.6 M parameters,
roughly 2.5 TFLOP per 1080p frame.

| Model, input → output     | Mid-range dGPU (RTX 3060 / RX 6600 class)  | Apple M1–M3 base GPU (MoltenVK) | CPU                                             |
| ------------------------- | ------------------------------------------ | ------------------------------- | ----------------------------------------------- |
| x4plus, 1080p → 4× (8K)   | ~10–30 s                                   | ~40–90 s                        | not supported by this binary; ncnn CPU: minutes |
| x4plus, 12 MP photo → 4×  | ~1–3 min                                   | ~4–8 min                        | –                                               |
| animevideov3, 1080p frame | ~0.3–1 s (V100 PyTorch: 10 fps model time) | ~1–3 s                          | –                                               |
| animevideov3, 720p frame  | ~0.15–0.5 s (V100 PyTorch: 22.6 fps)       | ~0.5–1.5 s                      | –                                               |
| waifu2x cunet 2×, 1080p   | ~1–3 s                                     | ~2–5 s                          | ~30–90 s (`-g -1`)                              |

Other reference points: Real-CUGAN is about 3× slower than animevideov3 at
1080p (3.4 vs. 10 fps on a V100). An RTX 5090 runs Real-ESRGAN 4× at 720p at
about 1.7 fps (fp16, backend unstated). One Upscayl review timed about 80 s
for a 4× UltraSharp upscale (ESRGAN architecture), but it does not name the
hardware or the image size.

## Per-candidate notes

### Real-ESRGAN via realesrgan-ncnn-vulkan (recommended)

- **Licenses:** The PyTorch repo `xinntao/Real-ESRGAN` is BSD-3-Clause. The ncnn
  port `xinntao/Real-ESRGAN-ncnn-vulkan` is **MIT**, not BSD-3, and its LICENSE
  also carries realsr-ncnn-vulkan's MIT text. ncnn is BSD-3-Clause with zlib and
  other permissive third-party parts. All of this is compatible with
  CatConvert's BSD-3-Clause and with the GPL ffmpeg sidecar, since each tool
  runs as a separate process. For the weights, see
  [Weights license](#weights-license) below: most likely BSD-3-Clause, but
  nothing from the author says so explicitly.
- **Models:** `realesrgan-x4plus` is the general photo model, 4× only. Its
  docs warn that `-s 2` with x4 weights gives poor results, so we upscale 4×
  and then Lanczos down to the target. `realesrgan-x4plus-anime` is for art and
  illustrations. `realesr-animevideov3` comes as separate x2/x3/x4 ncnn files.
  `realesr-general-x4v3` (compact, 4.6 MB `.pth`, also usable at x1–x3) is the
  better fit for real-world _video_. It only ships as PyTorch, so we convert it
  with ncnn's `pnnx` in the build.
- **Maintenance:** Effectively frozen: the last release was in 2022, there are
  68 open issues, and the ncnn submodule dates from April 2022. The code is
  small (`main.cpp` and `realesrgan.cpp`), so we build it with a current ncnn
  pin (20261020) the way upscayl does. Treat it as vendored code, not a
  dependency we update.
- **Build/linkage:** The upstream Linux binary links `libvulkan.so.1` and
  `libgomp.so.1`, which our `check_linkage` would reject. Current ncnn uses its
  built-in `simplevk` loader by default: it `dlopen`s the driver, needs no
  Vulkan SDK and no linked libvulkan. nihui's 2025 waifu2x builds already link
  no libvulkan. Use ncnn's own OpenMP (`NCNN_SIMPLEOMP`) or a static libgomp to
  drop libgomp. On macOS, MoltenVK links statically (Metal and other system
  frameworks only) and the binary is universal. On Windows, avoid shipping
  `vcomp140d.dll`, which is a debug runtime the upstream zip contains.
- **GPU requirements:**
  - **Windows:** `vulkan-1.dll` comes with every current NVIDIA, AMD and Intel
    driver. It is missing on "Microsoft Basic Display Adapter" machines and
    many VMs.
  - **Linux:** needs Mesa RADV/ANV or the NVIDIA driver. Ubuntu installs Mesa
    Vulkan by default. Mesa's `lavapipe` is a software Vulkan that works as a
    very slow fallback.
  - **macOS:** MoltenVK works on Apple silicon and on Intel Macs with Metal
    GPUs. Upscayl ships this way to a large user base.
  - **No GPU:** the upstream binary rejects a negative `-g` ("invalid gpu
    device"). ncnn itself runs on CPU, so our build can add a `-g -1` path like
    waifu2x has.
- **Interface:** `-i in.png -o out.png -n <model> -s 4 -t <tile> -g <gpu> -f png`.
  It reads and writes files (jpg/png/webp) or directories, with no stdin. It
  prints `NN.NN%` per tile to stderr, always (not only with `-v`), which gives
  real progress for a single photo. Tile size is picked automatically from the
  GPU's heap budget (32–200 px).
- **Known issue:** "some PCs output black images" (upstream TODO), usually old
  drivers. Detect an all-black output and report a driver error instead of
  writing the file.

### waifu2x-ncnn-vulkan

- MIT code and MIT weights. It is the only ncnn upscaler that nihui still
  releases (2025-09), its CPU mode (`-g -1`) works, and its binaries are small.
- Quality on photos is clearly behind Real-ESRGAN: waifu2x targets anime and
  art, and `upconv_7_photo` is an old model. It scales 2× per pass (4× means
  two passes).
- Verdict: not the main backend. It is a useful reference build (simplevk, no
  libvulkan) and a possible "anime 2× with denoise" model if users ask.

### Real-CUGAN ncnn

- MIT code (nihui), MIT weights (bilibili/ailab). Anime only, 2×/3×/4× with
  denoise levels. About 3× slower than animevideov3 at 1080p, and Real-ESRGAN's
  own comparisons show animevideov3 at equal or better quality. No release
  since 2022-07.
- Verdict: skip.

### upscayl-ncnn (`upscayl-bin`)

- An actively maintained fork of realesrgan-ncnn-vulkan, with current ncnn,
  static MoltenVK and extras such as a target width and per-image scale. It is
  **AGPL-3.0**, though, and upscayl's extra models are a licensing patchwork:
  4x-UltraSharp, for example, is CC BY-NC-SA 4.0.
- Shipping it as a separate process would probably be legal, but it adds AGPL
  source obligations, and the NC models are off-limits for anyone selling
  CatConvert. Use its build scripts as a reference only. Video2X (also AGPL)
  is a good reference for in-process frame piping.

### ffmpeg `dnn_processing` / `sr`

- **What our ffmpeg 9.0.2 has:** `dnn_processing` with the `tensorflow`,
  `openvino`, `torch` and, new in 9.0, `onnx` backends
  (`--enable-libonnxruntime`, with devices `cpu`, `cuda`, `dml` (Windows) and
  `vitisai`). `sr` is TensorFlow only, with SRCNN (2015) and ESPCN (2016)
  models that you train and export yourself. Our build has none of these
  enabled (`--disable-autodetect`, no DNN library).
- **Appeal:** one ffmpeg pass, no frame extraction, and the progress and
  cancellation we already have.
- **Problems:**
  - Every backend is a large shared C/C++ runtime linked into ffmpeg. That
    breaks the static, OS-libraries-only rule and the 120 MB budget in
    `build.sh`. TensorFlow and LibTorch are hundreds of MB to GBs. OpenVINO is
    Intel-centric. ONNX Runtime is the only reasonable one.
  - The ONNX backend runs synchronously, one request at a time, needs NCHW
    float32 input, and has **no Core ML provider**, so it is CPU-only on Macs.
  - There is no tiling. x4plus on a whole 1080p frame needs several GB of
    (V)RAM.
  - The `sr` models' quality is close to bicubic plus sharpening.
- **Verdict:** not now. Revisit if ffmpeg's ONNX backend gains Core ML and
  tiling. A related idea: `libplacebo` with GLSL shaders such as Anime4K (MIT)
  or FSRCNNX (GPL-3.0) gives real-time "light" upscaling inside ffmpeg. It
  needs Vulkan in our ffmpeg build, and its quality is far below GAN models.

### Newer options (late 2026)

- **ONNX Runtime with Real-ESRGAN ONNX** (own helper via the `ort` crate, or
  ORT's C API). Real-ESRGAN ships `scripts/pytorch2onnx.py`.
  - **Pros:** one model format; Core ML on macOS (GPU/ANE); DirectML or
    Windows ML on Windows. ORT is MIT and releases monthly.
  - **Cons:** on Linux, GPU means CUDA, a 370 MB+ package for NVIDIA only;
    otherwise CPU. DirectML is in "sustained engineering" (no new features)
    because Microsoft is moving to Windows ML. Per-platform runtime packaging
    is more work than one ncnn binary.
  - Good plan B if Vulkan coverage turns out poor in practice.
- **Apple `VTSuperResolutionScaler`** (VideoToolbox `VTFrameProcessor`,
  macOS 26+, `isSupported` gates hardware).
  - Has image and video input types and supported scale factors, and uses
    optical flow from previous frames for temporal stability in video.
  - The OS downloads the models on first use (`configurationModelStatus`,
    `downloadConfigurationModel`), so the first run is not offline.
  - Beta headers capped video input at 1920×1080 on macOS.
  - Zero bundle size, likely the fastest path on Apple silicon, and
    temporally stable. Needs a small Swift helper and works only on macOS 26
    (our minimum is 11.0).
  - A good _second_ backend for Macs later.
- **Core ML conversions** of Real-ESRGAN exist on Hugging Face (community,
  same BSD-3 weights). They need the same Swift helper.

## Integration sketch for CatConvert

**Tool and lookup.** Add `Tool::Upscale` (shown as `realesrgan-ncnn-vulkan`)
with its path resolved like libjxl's tools: `CATCONVERT_UPSCALER`, then the
installed pack in the app data dir (`upscaler/<version>/`), then `PATH`. The
core reports `Error::Unsupported("ML upscaling is not installed")` when it is
missing, and the editor offers the download. A probe at startup or install time
(`-g` list, or a 64×64 test image) records whether a usable Vulkan device
exists. The UI hides the ML option, or offers CPU-only, when none does.

**Photo pipeline** (fits today's sequential `Plan`):

1. When the source is not PNG/JPEG/WebP, or needs orientation or HDR
   handling: `ffmpeg` decodes it to a scratch PNG (weight 0.05).
2. `upscale -i scratch.png -o up.png -n realesrgan-x4plus -s 4` (weight 0.85).
   Progress comes from the `NN.NN%` lines. `run_tool` needs a small parser for
   them, as `run_ffmpeg` has for `-progress`.
3. `ffmpeg`/`cjxl` encode to the target format and size. Lanczos takes the 4×
   result down to 2× or 3×, or to a target such as 4K, and applies the existing
   crop and metadata handling (weight 0.1).

Scratch files go into `Plan::scratch` and are removed on success, failure and
cancel. Cancellation works as for other tools (`child.kill()`). Pre-check the
output size: x4plus on a 12 MP photo produces 192 MP, about 576 MB of RGB in
memory. Cap at, say, 8K on the long side and warn above it (ties in with step
1's "huge result" warning).

**Video pipeline**, two stages:

- **v1, disk chunks (no new process plumbing).** Per chunk of N seconds: ffmpeg
  extracts the frames to PNG, the upscaler processes the directory, then ffmpeg
  encodes a segment and the chunk's frames are deleted. A final concat step
  muxes the segments with the original audio.
  - Disk use: 1080p PNG is about 3–6 MB per frame and 4K output about
    12–25 MB, so a 10 s chunk at 30 fps needs about 5–9 GB. Pre-check free
    space.
  - Progress: weight per chunk, using the upscaler's per-file progress over
    frame counts.
  - This needs `Plan` steps generated per chunk. It works with today's runner,
    but every PNG round trip costs time.
- **v2, pipe (preferred).** Run three processes:
  `ffmpeg -i in -f rawvideo -pix_fmt rgb24 - | catconvert-upscale -W w -H h -s 4 -n model | ffmpeg -f rawvideo -s 4w×4h -r fps -i - -i in -map 0:v -map 1:a? …`.
  - The helper is ~300 lines of C++ (or Rust over ncnn's C API). It reuses
    realesrgan.cpp's tiling and keeps nothing on disk.
  - Progress: the last ffmpeg's `-progress` `out_time` over the duration,
    which the runner already handles.
  - Cancel: kill all three. Needs a "pipeline" step kind in the core.
  - Optional: temporal consistency is not handled by these single-image
    models. Expect some flicker on real footage, which is another reason to
    prefer the compact models for video.

**What to tell users** (UI copy, from the ballpark table):

- **Photo:** "AI upscaling takes about 10–60 seconds per photo on a graphics
  card, and minutes on older Macs." Show an ETA after the first tiles.
- **Video:** before starting, show "Estimated: ~N hours", measured on the first
  ~30 frames and extrapolated, plus a strong warning above ~10 minutes of
  footage. Also offer a "Preview 5 seconds" button.
- **No usable GPU:** "This computer has no supported graphics card for AI
  upscaling; classic upscaling (Lanczos) is used instead."

**Optional download.**

- **Build:** a `media-tools`-style CI job builds the pack
  (`catconvert-upscaler-<target>.tar.xz`, about 45 MB) from pinned sources:
  the ncnn tag, the realesrgan-ncnn-vulkan commit, and model files by SHA-256.
- **Publish:** a GitHub release asset, signed and notarized on macOS like the
  sidecars, with its source archive alongside.
- **Install:** the app knows the expected SHA-256 per version. It downloads
  only on an explicit click, unpacks to the app data dir, and can also import
  the same file from disk for offline machines. The updater already talks to
  GitHub releases, so the app is not "network-free" today. The default
  conversion path stays offline.
- **Why not bundle:** the installer would grow by 45 MB per platform, about
  70 MB on universal macOS. Most users never upscale, and GPU-less users can't.

## Open questions

1. ~~**Network.**~~ Decided: an in-app download is fine (it sends nothing; it
   only fetches). Keep "Install from file…" for offline machines.
2. **Model set.** Do we ship x4plus, x4plus-anime, animevideov3 and
   general-x4v3 (~45 MB), or start with x4plus plus general-x4v3 only (~35 MB)?
3. **Video.** Should video ML upscaling be in v1 at all? If yes, accept the
   disk-chunk approach first, or build the pipe helper straight away?
4. **CPU fallback.** Expose `-g -1` (minutes per photo) or hide ML without a
   GPU?
5. **Mac path.** Is a macOS 26 `VTSuperResolutionScaler` helper worth a second
   backend? Its quality has not been tested here, and it downloads OS models
   on first use.
6. **Measure.** Real numbers on an M-series Mac, an RTX 3060-class PC and an
   Intel iGPU laptop under Linux decide the UI copy and the default tile size.
7. ~~**Licensing.**~~ Researched: most likely BSD-3-Clause, with no explicit
   grant. See [Weights license](#weights-license): fetch the models from the
   official releases rather than redistributing them, and attribute anyway.

## Sources

- Real-ESRGAN (BSD-3-Clause), model zoo, anime video model and speed table:
  https://github.com/xinntao/Real-ESRGAN,
  https://github.com/xinntao/Real-ESRGAN/blob/master/docs/model_zoo.md,
  https://github.com/xinntao/Real-ESRGAN/blob/master/docs/anime_comparisons.md,
  https://github.com/xinntao/Real-ESRGAN/releases/tag/v0.2.5.0
- realesrgan-ncnn-vulkan (MIT), usage, source (`main.cpp`, `realesrgan.cpp`):
  https://github.com/xinntao/Real-ESRGAN-ncnn-vulkan,
  https://github.com/xinntao/Real-ESRGAN-ncnn-vulkan/blob/master/LICENSE
- ncnn (BSD-3-Clause), releases, simplevk loader:
  https://github.com/Tencent/ncnn/blob/master/LICENSE.txt,
  https://github.com/Tencent/ncnn/releases/tag/20261020,
  https://github.com/Tencent/ncnn/wiki/vulkan-driver-loader
- waifu2x-ncnn-vulkan (MIT): https://github.com/nihui/waifu2x-ncnn-vulkan,
  waifu2x models (MIT): https://github.com/nagadomi/waifu2x
- realcugan-ncnn-vulkan (MIT): https://github.com/nihui/realcugan-ncnn-vulkan,
  Real-CUGAN (MIT): https://github.com/bilibili/ailab/tree/main/Real-CUGAN
- upscayl-ncnn (AGPL-3.0): https://github.com/upscayl/upscayl-ncnn,
  upscayl (AGPL-3.0): https://github.com/upscayl/upscayl,
  4x-UltraSharp (CC BY-NC-SA 4.0): https://openmodeldb.info/models/4x-UltraSharp
- Weights license history: https://github.com/xinntao/Real-ESRGAN/commits/master/LICENSE,
  https://github.com/xinntao/Real-ESRGAN/issues/4,
  https://github.com/xinntao/Real-ESRGAN/issues/677,
  https://github.com/xinntao/Real-ESRGAN/issues/1004,
  https://github.com/xinntao/Real-ESRGAN/issues/1016,
  https://github.com/xinntao/Real-ESRGAN/issues/1018,
  https://github.com/xinntao/Real-ESRGAN/blob/master/setup.py,
  https://github.com/xinntao/Real-ESRGAN/releases,
  DIV2K terms: https://data.vision.ee.ethz.ch/cvl/DIV2K/,
  https://openmodeldb.info/models/4x-realesr-animevideo-v3,
  https://huggingface.co/amd/realesrgan-x4plus
- Weights treated as BSD-3: https://openmodeldb.info/models/2x-realesrgan-x2plus,
  https://huggingface.co/wkrettek/real-esrgan-models
- Video2X (AGPL-3.0): https://github.com/k4yt3x/video2x
- FFmpeg 9.0.2 source (`doc/filters.texi`, `configure`,
  `libavfilter/dnn/dnn_backend_onnx.c`, `Changelog`):
  https://ffmpeg.org/releases/ffmpeg-9.0.2.tar.xz,
  https://ffmpeg.org/ffmpeg-filters.html#dnn_005fprocessing
- ONNX Runtime (MIT): https://github.com/microsoft/onnxruntime,
  sizes for 1.23: https://www.sourceforge.net/projects/onnx-runtime.mirror/files/v1.23.0,
  `ort` crate: https://github.com/pykeio/ort
- DirectML in sustained engineering:
  https://learn.microsoft.com/en-ca/windows/ai/directml/dml-version-history
- Apple VideoToolbox super resolution:
  https://developer.apple.com/documentation/videotoolbox/vtsuperresolutionscalerconfiguration,
  https://developer.apple.com/documentation/videotoolbox/vtsuperresolutionscalerparameters,
  https://developer.apple.com/videos/play/wwdc2025/300,
  header limits: https://github.com/dotnet/macios/wiki/VideoToolbox-macOS-xcode26.0-b4
- Speed anecdotes: https://aireiter.com/blog/real-esrgan-guide-download-gui
  (RTX 5090 figure), https://www.dedoimedo.com/computers/upscayl-second-review.html
  (upscayl timing)
- libplacebo (LGPL-2.1): https://github.com/haasn/libplacebo,
  Anime4K (MIT): https://github.com/bloc97/Anime4K,
  FSRCNN-TensorFlow (GPL-3.0): https://github.com/igv/FSRCNN-TensorFlow
