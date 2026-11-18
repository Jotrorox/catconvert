#!/usr/bin/env bash
# Build the optional AI upscaler the app downloads on request:
# catconvert-upscaler, which is realesrgan-ncnn-vulkan (MIT) on the ncnn neural
# network framework (BSD-3-Clause) with libwebp, built from the pinned sources
# in sources.lock plus the small patches in patches/. ncnn's built-in Vulkan
# loader (simplevk) opens the GPU driver at runtime, so the binary links no
# Vulkan loader, no OpenMP runtime and no C++ runtime library: only what the OS
# itself provides. On macOS, MoltenVK is linked in statically. The Real-ESRGAN
# model files are not built or packed here; the app downloads them itself.
#
# Usage:
#   build.sh <target> [out-dir]     build for a Rust target triple
#   build.sh pack <dir> <file.zip>  zip a built (and possibly signed) <dir> into
#                                   the pack the app downloads
#   build.sh sources <file.tar.xz>  pack the corresponding source (pinned
#                                   archives, patches and this script) for a release
#
# Targets:
#   aarch64-apple-darwin, x86_64-apple-darwin, universal-apple-darwin  (on macOS)
#   x86_64-unknown-linux-gnu  (on Linux x86_64)
#   x86_64-pc-windows-msvc    (on Windows x64, in Git Bash, with Visual Studio's
#                              C++ tools; static CRT, so no runtime DLLs)
#
# Output:
#   <out-dir>/catconvert-upscaler[.exe]
#   <out-dir>/THIRD-PARTY-LICENSES.txt
# The default out-dir is target/upscaler/dist/<target>.
#
# The pack (`build.sh pack`) is a zip with exactly those two files at its top
# level, compressed with deflate, the executable bit set on Unix.
#
# Needs: a C/C++ toolchain, cmake (3.23+), git, curl, patch, and ninja except
# on Windows, which uses the Visual Studio generator. Windows also needs
# llvm-objdump or objdump for the linkage check. No Vulkan SDK.
#
# Environment:
#   UPSCALER_WORK     scratch directory (default: target/upscaler)
#   UPSCALER_SOURCES  where pinned archives are cached (default: $UPSCALER_WORK/sources)
#   UPSCALER_JOBS     parallel jobs (default: number of CPUs)
#   UPSCALER_MAX_MB   size budget per architecture in MB (default: 40)
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
LOCK="$HERE/sources.lock"
PATCHES="$HERE/patches"
if [ -d "$HERE/../../apps/desktop/src-tauri" ]; then
  ROOT="$(cd "$HERE/../.." && pwd)"
  DEFAULT_WORK="$ROOT/target/upscaler"
else
  # Unpacked from a release's source archive.
  DEFAULT_WORK="$HERE/work"
fi
WORK="${UPSCALER_WORK:-$DEFAULT_WORK}"
CACHE="${UPSCALER_SOURCES:-$WORK/sources}"
# Git Bash: /d/a/... rather than D:\a\..., which tar would take for a remote host.
if command -v cygpath >/dev/null 2>&1; then
  WORK=$(cygpath -u "$WORK")
  CACHE=$(cygpath -u "$CACHE")
fi
JOBS="${UPSCALER_JOBS:-$(getconf _NPROCESSORS_ONLN 2>/dev/null || nproc 2>/dev/null || echo 4)}"
MAX_MB="${UPSCALER_MAX_MB:-40}"
# The binary's name in the pack; the app looks for exactly this.
NAME="catconvert-upscaler"
# MoltenVK 1.4 is built for macOS 12, and a statically linked library can't
# run below its own minimum. The app itself supports macOS 11
# (bundle.macOS.minimumSystemVersion in tauri.conf.json), so it must treat the
# upscaler as unavailable there.
MACOS_MIN="12.0"

log() { printf '\n==> %s\n' "$*" >&2; }
die() {
  printf 'error: %s\n' "$*" >&2
  exit 1
}

# A path as native programs (cmake, cl) want it: C:/... in Git Bash on Windows.
native_path() {
  if command -v cygpath >/dev/null 2>&1; then
    cygpath -m "$1"
  else
    printf '%s' "$1"
  fi
}

# ---------------------------------------------------------------------------
# Sources
# ---------------------------------------------------------------------------

# lock <name> <column>: 2 version, 3 license, 4 url, 5 sha256, 6 license files, 7 kind.
lock() {
  awk -v n="$1" -v c="$2" '!/^#/ && $1 == n { print $c; found = 1 } END { exit !found }' "$LOCK" ||
    die "$1 is not in $LOCK"
}

# components [kind...]: the names in the lock file, optionally only those of the given kinds.
components() {
  awk -v kinds=" $* " '!/^#/ && NF && (kinds == "  " || index(kinds, " " $7 " ")) { print $1 }' "$LOCK"
}

sha256_of() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | cut -d' ' -f1
  else
    shasum -a 256 "$1" | cut -d' ' -f1
  fi
}

# Set TARBALL to the pinned archive of <name>, downloading and checking it
# first. (Not printed: bash runs $(...) without `set -e`, so failures there
# would not stop the script.)
tarball() {
  local name=$1 version url sum file ext
  version=$(lock "$name" 2)
  url=$(lock "$name" 4)
  sum=$(lock "$name" 5)
  mkdir -p "$CACHE"
  case $url in
    git+*)
      # Fetched by commit id (which is what git verifies) and archived.
      file="$CACHE/$name-$version.tar.gz"
      if [ ! -f "$file" ]; then
        local repo=${url#git+} commit
        commit=${repo##*@}
        repo=${repo%@*}
        log "Fetching $name $version ($commit)"
        rm -rf "$CACHE/$name.git"
        git init -q --bare "$CACHE/$name.git"
        git -C "$CACHE/$name.git" fetch -q --depth 1 "$repo" "$commit"
        [ "$(git -C "$CACHE/$name.git" rev-parse FETCH_HEAD)" = "$commit" ] ||
          die "$name: fetched commit is not $commit"
        # No CRLF conversion, whatever the machine's git config says, or the
        # patches would not apply on Windows.
        git -c core.autocrlf=false -C "$CACHE/$name.git" archive --format=tar.gz \
          --prefix="$name-$version/" -o "$file.part" "$commit"
        mv "$file.part" "$file"
        rm -rf "$CACHE/$name.git"
      fi
      ;;
    *)
      case $url in
        *.tar.xz) ext=tar.xz ;;
        *.tar.bz2) ext=tar.bz2 ;;
        *.tar.gz) ext=tar.gz ;;
        *.tar) ext=tar ;;
        *.zip) ext=zip ;;
        *) die "$name: unknown archive type: $url" ;;
      esac
      file="$CACHE/$name-$version.$ext"
      if [ ! -f "$file" ] || [ "$(sha256_of "$file")" != "$sum" ]; then
        log "Downloading $name $version"
        curl -fsSL --retry 3 -o "$file.part" "$url"
        local got
        got=$(sha256_of "$file.part")
        [ "$got" = "$sum" ] || die "$name: SHA-256 of $url is $got, expected $sum"
        mv "$file.part" "$file"
      fi
      ;;
  esac
  TARBALL=$file
}

# unpack <name> <dir>: extract <name> into <dir> (fresh every build) and apply
# its patches, patches/<name>-*.patch, in order.
unpack() {
  local name=$1 dir=$2 patch
  tarball "$name"
  rm -rf "$dir"
  mkdir -p "$dir"
  case $TARBALL in
    # ncnn's full-source zip has no top-level directory.
    *.zip) (cd "$dir" && cmake -E tar xf "$(native_path "$TARBALL")") ;;
    *) tar -xf "$TARBALL" -C "$dir" --strip-components=1 ;;
  esac
  for patch in "$PATCHES/$name"-*.patch; do
    [ -f "$patch" ] || continue
    log "Applying $(basename "$patch")"
    patch -d "$dir" -p1 --forward --batch <"$patch"
  done
}

# Where build_upscaler unpacks <name>: ncnn and libwebp go where upstream
# expects its git submodules.
component_dir() {
  case $1 in
    realesrgan-ncnn-vulkan) printf '%s' "$SRC/realesrgan-ncnn-vulkan" ;;
    ncnn | libwebp) printf '%s' "$SRC/realesrgan-ncnn-vulkan/src/$1" ;;
    *) printf '%s' "$SRC/$1" ;;
  esac
}

# ---------------------------------------------------------------------------
# Toolchain per target
# ---------------------------------------------------------------------------

setup_target() {
  TARGET=$1
  BUILD="$WORK/$TARGET"
  SRC="$BUILD/src"
  EXE=""
  MARCH=""
  CMAKE_EXTRA=()
  STRIP_CMD="strip"

  case $TARGET in
    aarch64-apple-darwin | x86_64-apple-darwin)
      [ "$(uname -s)" = Darwin ] || die "$TARGET must be built on macOS"
      OS=macos
      MARCH=${TARGET%%-*}
      [ "$MARCH" = aarch64 ] && MARCH=arm64
      # ncnn and libwebp pick their SIMD code from CMAKE_OSX_ARCHITECTURES.
      CMAKE_EXTRA=(-DCMAKE_OSX_ARCHITECTURES="$MARCH" -DCMAKE_OSX_DEPLOYMENT_TARGET="$MACOS_MIN")
      STRIP_CMD="strip -x"
      ;;
    x86_64-unknown-linux-gnu)
      [ "$(uname -s)-$(uname -m)" = Linux-x86_64 ] || die "$TARGET must be built on Linux x86_64"
      OS=linux
      # The C++ runtime goes inside the binary; glibc stays shared.
      CMAKE_EXTRA=(-DCMAKE_C_COMPILER=gcc -DCMAKE_CXX_COMPILER=g++
        "-DCMAKE_EXE_LINKER_FLAGS=-static-libgcc -static-libstdc++")
      ;;
    x86_64-pc-windows-msvc)
      case $(uname -s) in
        MINGW* | MSYS* | CYGWIN*) ;;
        *) die "$TARGET is built on Windows (Git Bash) with Visual Studio's C++ tools" ;;
      esac
      OS=windows
      EXE=".exe"
      # The static CRT (/MT) everywhere, so there is no vcruntime or msvcp DLL
      # to ship. upstream asks for it too; this also covers ncnn and libwebp.
      CMAKE_EXTRA=(-DCMAKE_MSVC_RUNTIME_LIBRARY=MultiThreaded -DCMAKE_POLICY_DEFAULT_CMP0091=NEW
        -DNCNN_BUILD_WITH_STATIC_CRT=ON)
      # Release builds with MSVC carry no debug info (no /DEBUG) to strip.
      STRIP_CMD=""
      ;;
    *) die "unknown target: $TARGET" ;;
  esac
  mkdir -p "$SRC"
}

# Note: "${a[@]+"${a[@]}"}" below because bash 3.2 (macOS) treats an empty
# array as unset under `set -u`.

# cmake_build <source dir> <build dir> [cmake options...]: configure and build
# a Release, with Ninja, or on Windows with the newest Visual Studio found.
cmake_build() {
  local src=$1 build=$2 generator
  shift 2
  if [ "$OS" = windows ]; then
    generator=(-A x64)
  else
    # Homebrew's prefix is ignored for libraries below, which hides its
    # programs too, so name ninja explicitly.
    command -v ninja >/dev/null 2>&1 || die "ninja not found"
    generator=(-G Ninja -DCMAKE_MAKE_PROGRAM="$(command -v ninja)")
  fi
  cmake -S "$(native_path "$src")" -B "$(native_path "$build")" \
    "${generator[@]}" \
    -DCMAKE_BUILD_TYPE=Release \
    -DCMAKE_IGNORE_PREFIX_PATH="/opt/homebrew;/usr/local;/opt/local" \
    -DCMAKE_POLICY_VERSION_MINIMUM=3.5 \
    -DBUILD_SHARED_LIBS=OFF \
    "$@"
  cmake --build "$(native_path "$build")" --config Release --parallel "$JOBS"
}

# glslangValidator from ncnn's own glslang, for the machine running the build
# (on a Mac building x86_64, that is still arm64): realesrgan-ncnn-vulkan
# compiles its four shaders to SPIR-V at build time. ncnn compiles its own at
# runtime with the glslang linked into it.
build_glslang_host() {
  local dir="$WORK/host" f
  [ -z "${GLSLANG:-}" ] || return 0
  log "Building glslangValidator for this machine"
  unpack ncnn "$dir/ncnn"
  cmake_build "$dir/ncnn/glslang" "$dir/glslang-build" \
    -DENABLE_GLSLANG_BINARIES=ON -DENABLE_OPT=OFF -DENABLE_HLSL=OFF \
    -DENABLE_SPVREMAPPER=OFF -DBUILD_EXTERNAL=OFF -DGLSLANG_TESTS=OFF \
    -DGLSLANG_ENABLE_INSTALL=OFF -DENABLE_PCH=OFF
  # Named glslang nowadays; glslangValidator is a symlink to it.
  for f in "$dir/glslang-build/StandAlone/glslang$EXE" "$dir/glslang-build/StandAlone/Release/glslang$EXE"; do
    if [ -f "$f" ]; then
      GLSLANG=$f
      return 0
    fi
  done
  die "glslang was not built"
}

build_upscaler() {
  local src mvk=() lib
  src=$(component_dir realesrgan-ncnn-vulkan)
  unpack realesrgan-ncnn-vulkan "$src"
  unpack ncnn "$(component_dir ncnn)"
  unpack libwebp "$(component_dir libwebp)"
  if [ "$OS" = macos ]; then
    unpack moltenvk "$(component_dir moltenvk)"
    lib="$(component_dir moltenvk)/MoltenVK/static/MoltenVK.xcframework/macos-arm64_x86_64/libMoltenVK.a"
    [ -f "$lib" ] || die "MoltenVK has no $lib"
    # ncnn links Vulkan_LIBRARY and the frameworks MoltenVK needs; simplevk
    # then takes the Vulkan entry point from it instead of a dylib.
    mvk=(-DUSE_STATIC_MOLTENVK=ON -DVulkan_LIBRARY="$lib")
  fi
  # upstream's CMakeLists builds ncnn with only the layers its models use (its
  # options win over ncnn's defaults). Convolution on the GPU also creates
  # Reshape (1x1 kernels) and Quantize (int8) layers, so turn those on too.
  cmake_build "$src/src" "$src/_build" \
    ${CMAKE_EXTRA[@]+"${CMAKE_EXTRA[@]}"} \
    ${mvk[@]+"${mvk[@]}"} \
    -DGLSLANGVALIDATOR_EXECUTABLE="$(native_path "$GLSLANG")" \
    -DCMAKE_DISABLE_FIND_PACKAGE_OpenMP=ON \
    -DNCNN_OPENMP=OFF \
    -DNCNN_SIMPLEOMP=OFF \
    -DNCNN_SIMPLEVK=ON \
    -DNCNN_SYSTEM_GLSLANG=OFF \
    -DWITH_LAYER_reshape=ON \
    -DWITH_LAYER_quantize=ON
}

# ---------------------------------------------------------------------------
# Output and checks
# ---------------------------------------------------------------------------

# The libraries a binary links dynamically, one per line.
linked_libs() {
  case $OS in
    # Header lines ("<file>:", or per slice "<file> (architecture arm64):") end in ':'.
    macos) otool -L "$1" | awk '!/:$/ {print $1}' ;;
    linux) readelf -d "$1" | awk '/NEEDED/ { gsub(/[][]/, "", $5); print $5 }' ;;
    windows)
      local tool vswhere="/c/Program Files (x86)/Microsoft Visual Studio/Installer/vswhere.exe"
      if tool=$(command -v llvm-objdump || command -v objdump); then
        "$tool" -p "$1" | awk '/DLL Name:/ { print $3 }' | tr -d '\r'
      elif [ -x "$vswhere" ] &&
        tool=$("$vswhere" -latest -find 'VC/Tools/MSVC/**/bin/Hostx64/x64/dumpbin.exe' | tr -d '\r' | head -n 1) &&
        [ -n "$tool" ]; then
        "$(cygpath -u "$tool")" -nologo -dependents "$1" | tr -d '\r' | awk '/\.[Dd][Ll][Ll]$/ { print $1 }'
      else
        die "llvm-objdump, objdump or Visual Studio's dumpbin is needed to check the DLLs a binary imports"
      fi
      ;;
  esac
}

# Fail if a binary links anything but the OS's own libraries: above all no
# Vulkan loader (simplevk opens the driver at runtime), no OpenMP runtime and
# no C++ runtime.
check_linkage() {
  local bin=$1 lib bad="" count=0
  for lib in $(linked_libs "$bin"); do
    count=$((count + 1))
    case $OS in
      macos) case $lib in /usr/lib/* | /System/Library/*) continue ;; esac ;;
      linux)
        case $lib in
          libc.so.* | libm.so.* | libpthread.so.* | libdl.so.* | librt.so.* | ld-linux-*.so.* | libmvec.so.*) continue ;;
        esac
        ;;
      windows)
        # System DLLs are fine; the Visual C++ runtimes (release or debug),
        # OpenMP, the Vulkan loader and anything mingw-like must not appear.
        case $(printf '%s' "$lib" | tr '[:upper:]' '[:lower:]') in
          vcruntime* | msvcp* | vcomp* | concrt* | ucrtbase* | api-ms-win-crt-* | vulkan-1.dll | lib*.dll) ;;
          *) continue ;;
        esac
        ;;
    esac
    bad="$bad $lib"
  done
  [ "$count" -gt 0 ] || die "found no linked libraries in $(basename "$bin"); is the linkage check broken?"
  [ -z "$bad" ] || die "$(basename "$bin") links libraries the OS doesn't ship:$bad"
}

# Run the new binary when this machine can. -h needs no GPU: like upstream, it
# prints the usage to stderr and exits with -1 (255).
smoke_test() {
  local bin=$1 run=() usage status=0
  if [ "$OS" = macos ] && [ "$MARCH" != "$(uname -m)" ]; then
    arch -"$MARCH" /usr/bin/true 2>/dev/null || {
      log "Skipping smoke test: this Mac can't run $MARCH"
      return
    }
    run=(arch -"$MARCH")
  fi
  usage=$(${run[@]+"${run[@]}"} "$bin" -h 2>&1) || status=$?
  [ "$status" -ne 0 ] || die "$(basename "$bin") -h exited with 0; upstream exits with -1"
  printf '%s\n' "$usage" | grep -q '^Usage: realesrgan-ncnn-vulkan ' ||
    die "$(basename "$bin") -h printed no usage (exit status $status): $usage"
  log "Smoke test passed: $(basename "$bin") -h"
}

# Copy, strip and check the binary of the current target into <dir>.
collect() {
  local dir=$1 from to
  for from in "$SRC/realesrgan-ncnn-vulkan/_build/realesrgan-ncnn-vulkan$EXE" \
    "$SRC/realesrgan-ncnn-vulkan/_build/Release/realesrgan-ncnn-vulkan$EXE" ""; do
    [ -f "$from" ] && break
  done
  [ -n "$from" ] || die "realesrgan-ncnn-vulkan$EXE was not built"
  mkdir -p "$dir"
  to="$dir/$NAME$EXE"
  cp "$from" "$to"
  chmod 755 "$to"
  if [ -n "$STRIP_CMD" ]; then
    # shellcheck disable=SC2086 # STRIP_CMD may carry a flag
    $STRIP_CMD "$to"
  fi
  # Apple silicon runs only signed code; sign ad hoc after stripping. Releases
  # sign again with a Developer ID.
  if [ "$OS" = macos ]; then codesign --force --sign - "$to"; fi
  check_linkage "$to"
}

write_licenses() {
  local out=$1 name file rule dir kinds=source
  [ "$OS" = macos ] && kinds="source macos-binary"
  rule=$(printf '%.0s=' $(seq 1 78))
  {
    echo "Third-party software in CatConvert's AI upscaler"
    echo
    echo "catconvert-upscaler is realesrgan-ncnn-vulkan (with code from realsr-ncnn-vulkan"
    echo "and stb_image, public domain), built on the ncnn neural network framework, which"
    echo "includes glslang, and on libwebp; on macOS also on MoltenVK. It was built from"
    echo "the upstream sources listed below with a few small patches. The complete"
    echo "corresponding source code, together with the patches and the script that built"
    echo "it, is attached to every CatConvert release as"
    echo "catconvert-<version>-upscaler-source.tar.xz:"
    echo "https://github.com/Jotrorox/catconvert/releases"
    echo
    echo "The Real-ESRGAN model files are not part of this download. CatConvert fetches"
    echo "them from the official Real-ESRGAN releases (Copyright (c) 2021 Xintao Wang,"
    echo "published in https://github.com/xinntao/Real-ESRGAN under BSD-3-Clause)."
    # shellcheck disable=SC2086 # several kinds
    for name in $(components $kinds); do
      dir=$(component_dir "$name")
      printf '\n%s\n%s %s (%s)\n%s\n%s\n' "$rule" "$name" "$(lock "$name" 2)" \
        "$(lock "$name" 3 | sed 's/,/ AND /g')" "$(lock "$name" 4)" "$rule"
      for file in $(lock "$name" 6 | tr ',' ' '); do
        [ -f "$dir/$file" ] || die "$name has no $file"
        printf '\n'
        cat "$dir/$file"
      done
    done
  } >"$out"
}

mb() { awk -v b="$1" 'BEGIN { printf "%6.1f", b / 1048576 }'; }

check_size() {
  local file=$1 target=$2 bytes budget=$MAX_MB
  [ "$target" = universal-apple-darwin ] && budget=$((MAX_MB * 2))
  bytes=$(wc -c <"$file" | tr -d ' ')
  log "Size for $target"
  printf '  %s MB  %s (budget %d MB)\n' "$(mb "$bytes")" "$(basename "$file")" "$budget"
  [ "$bytes" -le $((budget * 1048576)) ] ||
    die "the upscaler grew past the $budget MB budget; raise UPSCALER_MAX_MB if that's intended"
}

build_target() {
  local target=$1 out=$2
  setup_target "$target"
  log "Building the upscaler for $TARGET in $BUILD"
  build_glslang_host
  build_upscaler
  collect "$out"
  smoke_test "$out/$NAME$EXE"
}

build() {
  local target=$1 out=$2
  case $target in
    universal-apple-darwin)
      local arch
      for arch in aarch64 x86_64; do
        build_target "$arch-apple-darwin" "$WORK/out/$arch-apple-darwin"
      done
      mkdir -p "$out"
      lipo -create -output "$out/$NAME" \
        "$WORK/out/aarch64-apple-darwin/$NAME" \
        "$WORK/out/x86_64-apple-darwin/$NAME"
      codesign --force --sign - "$out/$NAME"
      MARCH=$(uname -m)
      check_linkage "$out/$NAME"
      smoke_test "$out/$NAME"
      ;;
    *)
      build_target "$target" "$out"
      ;;
  esac
  write_licenses "$out/THIRD-PARTY-LICENSES.txt"
  check_size "$out/$NAME$EXE" "$target"
  log "Done: $out"
}

# The pack the app downloads: the binary and the licenses at the top level of
# a zip, deflated (the app's unzipper reads only stored and deflated entries).
pack() {
  local dir=$1 dest=$2 bin listed
  command -v zip >/dev/null 2>&1 || die "zip not found"
  if [ -f "$dir/$NAME.exe" ]; then
    bin="$NAME.exe"
  elif [ -f "$dir/$NAME" ]; then
    bin="$NAME"
  else
    die "$dir has no $NAME or $NAME.exe"
  fi
  [ -f "$dir/THIRD-PARTY-LICENSES.txt" ] || die "$dir has no THIRD-PARTY-LICENSES.txt"
  mkdir -p "$(dirname "$dest")"
  dest="$(cd "$(dirname "$dest")" && pwd)/$(basename "$dest")"
  rm -f "$dest"
  chmod 755 "$dir/$bin"
  chmod 644 "$dir/THIRD-PARTY-LICENSES.txt"
  # -X: no extra attributes (owner ids, extended timestamps), only what's needed.
  (cd "$dir" && zip -X -9 -q "$dest" "$bin" THIRD-PARTY-LICENSES.txt)
  listed=$(unzip -Z1 "$dest" | sort | tr '\n' ' ')
  [ "$listed" = "$(printf '%s\n' "$bin" THIRD-PARTY-LICENSES.txt | sort | tr '\n' ' ')" ] ||
    die "$dest holds unexpected entries: $listed"
  log "Wrote $dest"
}

pack_sources() {
  local dest=$1 stage name
  stage="$WORK/source-stage/catconvert-upscaler-source"
  rm -rf "$WORK/source-stage"
  mkdir -p "$stage/sources"
  # MoltenVK is a prebuilt library from its release; build.sh downloads it.
  for name in $(components source); do
    tarball "$name"
    cp "$TARBALL" "$stage/sources/"
  done
  cp "$HERE/build.sh" "$HERE/sources.lock" "$stage/"
  cp -R "$PATCHES" "$stage/patches"
  cat >"$stage/README" <<'EOF'
Corresponding source of CatConvert's optional AI upscaler, catconvert-upscaler:
realesrgan-ncnn-vulkan (MIT) on ncnn (BSD-3-Clause, with glslang) and libwebp
(BSD-3-Clause), and on macOS MoltenVK (Apache-2.0).

sources/      the exact upstream source archives it was built from
sources.lock  versions, origins and SHA-256 checksums of those archives, and of
              the prebuilt MoltenVK release that build.sh downloads on macOS
patches/      the small changes applied to them; each explains itself
build.sh      the script that built it

To rebuild, for example for Linux:

    UPSCALER_SOURCES="$PWD/sources" ./build.sh x86_64-unknown-linux-gnu out

See the top of build.sh for targets and required tools.
EOF
  mkdir -p "$(dirname "$dest")"
  tar -cJf "$dest" -C "$WORK/source-stage" catconvert-upscaler-source
  log "Wrote $dest"
}

main() {
  [ $# -ge 1 ] || die "usage: build.sh <target> [out-dir] | build.sh pack <dir> <file.zip> | build.sh sources <file.tar.xz>"
  case $1 in
    -h | --help) sed -n '2,/^set -euo/p' "$0" | sed '$d; s/^# \{0,1\}//' ;;
    pack)
      [ $# -eq 3 ] || die "usage: build.sh pack <dir> <file.zip>"
      pack "$2" "$3"
      ;;
    sources)
      [ $# -eq 2 ] || die "usage: build.sh sources <file.tar.xz>"
      pack_sources "$2"
      ;;
    *)
      local out=${2:-$WORK/dist/$1}
      mkdir -p "$out"
      build "$1" "$(cd "$out" && pwd)"
      ;;
  esac
}

main "$@"
