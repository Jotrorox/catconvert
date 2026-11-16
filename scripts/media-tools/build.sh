#!/usr/bin/env bash
# Build the media tools the desktop app bundles: ffmpeg and ffprobe (with x264,
# so GPL-2.0-or-later) and libjxl's cjxl, djxl and jxlinfo. Everything is
# built from the pinned sources in sources.lock and linked statically, so the
# binaries only need what the OS itself provides.
#
# Usage:
#   build.sh <target> [out-dir]     build for a Rust target triple
#   build.sh sources <file.tar.xz>  pack the corresponding source (all pinned
#                                   tarballs plus this script) for a release
#
# Targets:
#   aarch64-apple-darwin, x86_64-apple-darwin, universal-apple-darwin  (on macOS)
#   x86_64-unknown-linux-gnu                                           (on Linux x86_64)
#   x86_64-pc-windows-msvc        (on Linux, cross-compiled with mingw-w64; the
#                                  app itself links with MSVC, the tools need not)
#
# Output, named the way Tauri's `bundle.externalBin` expects:
#   <out-dir>/catconvert-{ffmpeg,ffprobe,cjxl,djxl,jxlinfo}-<target>[.exe]
#   <out-dir>/THIRD-PARTY-LICENSES.txt
# The default out-dir is apps/desktop/src-tauri/binaries.
#
# Needs: a C/C++ toolchain, cmake (3.23+), ninja, make, pkg-config, git, curl,
# perl, and nasm for x86_64 targets. Windows needs mingw-w64 (posix threads).
#
# Environment:
#   MEDIA_TOOLS_WORK     scratch directory (default: target/media-tools)
#   MEDIA_TOOLS_SOURCES  where pinned tarballs are cached (default: $MEDIA_TOOLS_WORK/sources)
#   MEDIA_TOOLS_JOBS     parallel jobs (default: number of CPUs)
#   MEDIA_TOOLS_MAX_MB   size budget per architecture in MB (default: 100)
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
LOCK="$HERE/sources.lock"
if [ -d "$HERE/../../apps/desktop/src-tauri" ]; then
  ROOT="$(cd "$HERE/../.." && pwd)"
  DEFAULT_WORK="$ROOT/target/media-tools"
  DEFAULT_OUT="$ROOT/apps/desktop/src-tauri/binaries"
else
  # Unpacked from a release's source archive.
  DEFAULT_WORK="$HERE/work"
  DEFAULT_OUT="$HERE/out"
fi
WORK="${MEDIA_TOOLS_WORK:-$DEFAULT_WORK}"
CACHE="${MEDIA_TOOLS_SOURCES:-$WORK/sources}"
JOBS="${MEDIA_TOOLS_JOBS:-$(getconf _NPROCESSORS_ONLN 2>/dev/null || echo 4)}"
MAX_MB="${MEDIA_TOOLS_MAX_MB:-100}"
TOOLS="ffmpeg ffprobe cjxl djxl jxlinfo"
# Prefix of the bundled file names; keep in sync with BUNDLED_PREFIX in
# crates/core/src/ffmpeg.rs. It keeps a Linux package from claiming /usr/bin/ffmpeg.
NAME_PREFIX="catconvert-"
# Keep in sync with bundle.macOS.minimumSystemVersion in tauri.conf.json.
MACOS_MIN="11.0"
# Encoders catconvert-core asks for (crates/core/src/convert.rs).
REQUIRED_ENCODERS="libx264 libvpx-vp9 libopus libmp3lame libwebp libwebp_anim libsvtav1 libjxl libjxl_anim aac flac pcm_s16le gif apng png mjpeg"

log() { printf '\n==> %s\n' "$*" >&2; }
die() {
  printf 'error: %s\n' "$*" >&2
  exit 1
}

# ---------------------------------------------------------------------------
# Sources
# ---------------------------------------------------------------------------

# lock <name> <column>: 2 version, 3 license, 4 url, 5 sha256, 6 license files.
lock() {
  awk -v n="$1" -v c="$2" '!/^#/ && $1 == n { print $c; found = 1 } END { exit !found }' "$LOCK" ||
    die "$1 is not in $LOCK"
}

components() { awk '!/^#/ && NF { print $1 }' "$LOCK"; }

sha256_of() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | cut -d' ' -f1
  else
    shasum -a 256 "$1" | cut -d' ' -f1
  fi
}

# Set TARBALL to the pinned tarball of <name>, downloading and checking it
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
        git -C "$CACHE/$name.git" archive --format=tar.gz --prefix="$name-$version/" \
          -o "$file.part" "$commit"
        mv "$file.part" "$file"
        rm -rf "$CACHE/$name.git"
      fi
      ;;
    *)
      case $url in
        *.tar.xz) ext=tar.xz ;;
        *.tar.bz2) ext=tar.bz2 ;;
        *) ext=tar.gz ;;
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

# Extract <name> into $SRC/<name> (fresh every build) and set UNPACKED to it.
unpack() {
  local name=$1
  tarball "$name"
  rm -rf "${SRC:?}/$name"
  mkdir -p "$SRC/$name"
  tar -xf "$TARBALL" -C "$SRC/$name" --strip-components=1
  UNPACKED="$SRC/$name"
}

# ---------------------------------------------------------------------------
# Toolchain per target
# ---------------------------------------------------------------------------

setup_target() {
  TARGET=$1
  BUILD="$WORK/$TARGET"
  SRC="$BUILD/src"
  PREFIX="$BUILD/prefix"
  EXE=""
  CROSS=0
  CROSS_PREFIX=""
  HOST_TRIPLE=""
  CMAKE_EXTRA=()
  STRIP_CMD="strip"

  case $TARGET in
    aarch64-apple-darwin | x86_64-apple-darwin)
      [ "$(uname -s)" = Darwin ] || die "$TARGET must be built on macOS"
      OS=macos
      ARCH=${TARGET%%-*}
      MARCH=$ARCH
      [ "$ARCH" = aarch64 ] && MARCH=arm64
      [ "$MARCH" = "$(uname -m)" ] || CROSS=1
      HOST_TRIPLE="$ARCH-apple-darwin"
      export MACOSX_DEPLOYMENT_TARGET="$MACOS_MIN"
      export CC=clang CXX=clang++
      export CFLAGS="-arch $MARCH -O2" CXXFLAGS="-arch $MARCH -O2" LDFLAGS="-arch $MARCH"
      CMAKE_EXTRA=(-DCMAKE_OSX_ARCHITECTURES="$MARCH" -DCMAKE_OSX_DEPLOYMENT_TARGET="$MACOS_MIN")
      # Projects pick their SIMD code by CMAKE_SYSTEM_PROCESSOR, which is the
      # host's unless this is declared a cross build.
      if [ "$CROSS" = 1 ]; then
        CMAKE_EXTRA+=(-DCMAKE_SYSTEM_NAME=Darwin -DCMAKE_SYSTEM_PROCESSOR="$MARCH")
      fi
      STRIP_CMD="strip -x"
      ;;
    x86_64-unknown-linux-gnu)
      [ "$(uname -s)-$(uname -m)" = Linux-x86_64 ] || die "$TARGET must be built on Linux x86_64"
      OS=linux
      ARCH=x86_64
      MARCH=x86_64
      HOST_TRIPLE="x86_64-linux-gnu"
      export CC=gcc CXX=g++
      export CFLAGS="-O2" CXXFLAGS="-O2" LDFLAGS="-static-libgcc -static-libstdc++"
      ;;
    x86_64-pc-windows-msvc)
      [ "$(uname -s)" = Linux ] || die "$TARGET is cross-compiled on Linux with mingw-w64"
      OS=windows
      ARCH=x86_64
      MARCH=x86_64
      EXE=".exe"
      CROSS=1
      CROSS_PREFIX="x86_64-w64-mingw32-"
      HOST_TRIPLE="x86_64-w64-mingw32"
      # libjxl needs std::thread, which the win32-threads mingw flavour lacks.
      local flavour=""
      command -v "${CROSS_PREFIX}g++-posix" >/dev/null 2>&1 && flavour="-posix"
      export CC="${CROSS_PREFIX}gcc$flavour" CXX="${CROSS_PREFIX}g++$flavour"
      export AR="${CROSS_PREFIX}ar" RANLIB="${CROSS_PREFIX}ranlib"
      export CFLAGS="-O2" CXXFLAGS="-O2" LDFLAGS="-static -static-libgcc -static-libstdc++"
      mkdir -p "$BUILD"
      cat >"$BUILD/mingw.cmake" <<EOF
set(CMAKE_SYSTEM_NAME Windows)
set(CMAKE_SYSTEM_PROCESSOR x86_64)
set(CMAKE_C_COMPILER $CC)
set(CMAKE_CXX_COMPILER $CXX)
set(CMAKE_RC_COMPILER ${CROSS_PREFIX}windres)
set(CMAKE_FIND_ROOT_PATH $PREFIX)
set(CMAKE_FIND_ROOT_PATH_MODE_PROGRAM NEVER)
set(CMAKE_FIND_ROOT_PATH_MODE_LIBRARY ONLY)
set(CMAKE_FIND_ROOT_PATH_MODE_INCLUDE ONLY)
set(CMAKE_FIND_ROOT_PATH_MODE_PACKAGE ONLY)
EOF
      CMAKE_EXTRA=(-DCMAKE_TOOLCHAIN_FILE="$BUILD/mingw.cmake")
      STRIP_CMD="${CROSS_PREFIX}strip"
      ;;
    *) die "unknown target: $TARGET" ;;
  esac

  HOST_ARGS=()
  if [ "$CROSS" = 1 ]; then HOST_ARGS=(--host="$HOST_TRIPLE"); fi

  # Only ever find what this script built, never Homebrew's or the distro's
  # libraries, or the binaries would depend on them.
  export PKG_CONFIG_LIBDIR="$PREFIX/lib/pkgconfig"
  unset PKG_CONFIG_PATH
  mkdir -p "$SRC" "$PREFIX/lib" "$PREFIX/include" "$PREFIX/bin"
}

# Note: "${a[@]+"${a[@]}"}" below because bash 3.2 (macOS) treats an empty
# array as unset under `set -u`.

# cmake_install <source dir> [cmake options...]
cmake_install() {
  local src=$1
  shift
  # Homebrew's prefix is ignored for libraries below, which hides its programs
  # too, so name the build tools explicitly.
  local programs=(-DCMAKE_MAKE_PROGRAM="$(command -v ninja)"
    -DPKG_CONFIG_EXECUTABLE="$(command -v pkg-config)")
  if command -v nasm >/dev/null 2>&1; then
    programs+=(-DCMAKE_ASM_NASM_COMPILER="$(command -v nasm)")
  fi
  cmake -S "$src" -B "$src/_build" -G Ninja \
    "${programs[@]}" \
    -DCMAKE_BUILD_TYPE=Release \
    -DCMAKE_INSTALL_PREFIX="$PREFIX" \
    -DCMAKE_INSTALL_LIBDIR=lib \
    -DCMAKE_PREFIX_PATH="$PREFIX" \
    -DCMAKE_IGNORE_PREFIX_PATH="/opt/homebrew;/usr/local;/opt/local" \
    -DCMAKE_FIND_USE_SYSTEM_ENVIRONMENT_PATH=OFF \
    -DCMAKE_POLICY_VERSION_MINIMUM=3.5 \
    -DBUILD_SHARED_LIBS=OFF \
    -DBUILD_TESTING=OFF \
    ${CMAKE_EXTRA[@]+"${CMAKE_EXTRA[@]}"} \
    "$@"
  cmake --build "$src/_build" --parallel "$JOBS"
  cmake --install "$src/_build"
}

# autotools_install <source dir> [configure options...]
autotools_install() {
  local src=$1
  shift
  (
    cd "$src"
    ./configure --prefix="$PREFIX" --disable-shared --enable-static \
      ${HOST_ARGS[@]+"${HOST_ARGS[@]}"} "$@"
    make -j"$JOBS"
    make install
  )
}

# ---------------------------------------------------------------------------
# Libraries
# ---------------------------------------------------------------------------

build_zlib() {
  unpack zlib
  cmake_install "$UNPACKED" -DZLIB_BUILD_SHARED=OFF -DZLIB_BUILD_TESTING=OFF
  # Some toolchains name the static library libzs.a; ffmpeg links -lz.
  if [ ! -f "$PREFIX/lib/libz.a" ] && [ -f "$PREFIX/lib/libzs.a" ]; then
    cp "$PREFIX/lib/libzs.a" "$PREFIX/lib/libz.a"
  fi
}

build_libpng() {
  unpack libpng
  cmake_install "$UNPACKED" -DPNG_SHARED=OFF -DPNG_STATIC=ON -DPNG_TESTS=OFF \
    -DPNG_TOOLS=OFF -DPNG_FRAMEWORK=OFF
}

build_giflib() {
  # giflib only ships a Makefile for the host; the library is eight C files.
  local src f
  unpack giflib
  src=$UNPACKED
  for f in dgif_lib egif_lib gifalloc gif_err gif_font gif_hash openbsd-reallocarray quantize; do
    # shellcheck disable=SC2086 # CFLAGS holds several flags
    $CC $CFLAGS -c "$src/$f.c" -o "$src/$f.o"
  done
  rm -f "$PREFIX/lib/libgif.a"
  ${AR:-ar} rcs "$PREFIX/lib/libgif.a" "$src"/*.o
  cp "$src/gif_lib.h" "$PREFIX/include/"
}

build_libjpeg_turbo() {
  unpack libjpeg-turbo
  cmake_install "$UNPACKED" -DENABLE_SHARED=OFF -DENABLE_STATIC=ON \
    -DWITH_TURBOJPEG=OFF -DWITH_TOOLS=OFF -DWITH_TESTS=OFF -DREQUIRE_SIMD=ON
}

build_lcms2() {
  unpack lcms2
  autotools_install "$UNPACKED" --without-jpeg --without-tiff --without-zlib
}

build_brotli() {
  unpack brotli
  cmake_install "$UNPACKED" -DBROTLI_BUILD_TOOLS=OFF -DBROTLI_DISABLE_TESTS=ON
}

build_highway() {
  unpack highway
  cmake_install "$UNPACKED" -DHWY_ENABLE_TESTS=OFF -DHWY_ENABLE_EXAMPLES=OFF \
    -DHWY_ENABLE_CONTRIB=OFF -DHWY_ENABLE_INSTALL=ON -DHWY_FORCE_STATIC_LIBS=ON
}

build_libjxl() {
  local static=ON linker=""
  # JPEGXL_STATIC links fully static executables on Linux; there only the
  # C++ runtime should be static, glibc stays shared.
  if [ "$OS" = linux ]; then
    static=OFF
    linker="-static-libgcc -static-libstdc++"
  fi
  unpack libjxl
  cmake_install "$UNPACKED" \
    -DJPEGXL_STATIC="$static" \
    -DCMAKE_EXE_LINKER_FLAGS="$linker" \
    -DJPEGXL_ENABLE_TOOLS=ON \
    -DJPEGXL_ENABLE_SKCMS=OFF \
    -DJPEGXL_FORCE_SYSTEM_LCMS2=ON \
    -DJPEGXL_FORCE_SYSTEM_BROTLI=ON \
    -DJPEGXL_FORCE_SYSTEM_HWY=ON \
    -DJPEGXL_BUNDLE_LIBPNG=OFF \
    -DJPEGXL_ENABLE_SJPEG=OFF \
    -DJPEGXL_ENABLE_OPENEXR=OFF \
    -DJPEGXL_ENABLE_BENCHMARK=OFF \
    -DJPEGXL_ENABLE_EXAMPLES=OFF \
    -DJPEGXL_ENABLE_JNI=OFF \
    -DJPEGXL_ENABLE_MANPAGES=OFF \
    -DJPEGXL_ENABLE_DOXYGEN=OFF \
    -DJPEGXL_ENABLE_FUZZERS=OFF \
    -DJPEGXL_ENABLE_DEVTOOLS=OFF \
    -DJPEGXL_ENABLE_PLUGINS=OFF \
    -DJPEGXL_ENABLE_TCMALLOC=OFF
}

build_x264() {
  local src
  unpack x264
  src=$UNPACKED
  (
    cd "$src"
    ./configure --prefix="$PREFIX" --enable-static --disable-cli --disable-opencl \
      ${HOST_ARGS[@]+"${HOST_ARGS[@]}"} \
      ${CROSS_PREFIX:+--cross-prefix="$CROSS_PREFIX"} \
      --extra-cflags="$CFLAGS" --extra-ldflags="$LDFLAGS"
    make -j"$JOBS"
    make install
  )
}

build_libvpx() {
  local src target asm=()
  unpack libvpx
  src=$UNPACKED
  case $TARGET in
    aarch64-apple-darwin) target=arm64-darwin20-gcc ;;
    x86_64-apple-darwin) target=x86_64-darwin20-gcc ;;
    x86_64-unknown-linux-gnu) target=x86_64-linux-gcc ;;
    x86_64-pc-windows-msvc) target=x86_64-win64-gcc ;;
  esac
  [ "$ARCH" = x86_64 ] && asm=(--as=nasm)
  mkdir -p "$src/_build"
  (
    cd "$src/_build"
    CROSS="$CROSS_PREFIX" ../configure --target="$target" --prefix="$PREFIX" \
      --disable-shared --enable-static --disable-examples --disable-tools \
      --disable-docs --disable-unit-tests --disable-vp8 --disable-vp9-decoder \
      --enable-vp9-highbitdepth \
      ${asm[@]+"${asm[@]}"} \
      --extra-cflags="$CFLAGS" --extra-cxxflags="$CXXFLAGS"
    make -j"$JOBS"
    make install
  )
}

build_opus() {
  local hardening=()
  # mingw-w64 needs libssp for these; ffmpeg's own link line doesn't add it.
  [ "$OS" = windows ] && hardening=(-DOPUS_FORTIFY_SOURCE=OFF -DOPUS_STACK_PROTECTOR=OFF)
  unpack opus
  cmake_install "$UNPACKED" -DOPUS_BUILD_SHARED_LIBRARY=OFF -DOPUS_BUILD_PROGRAMS=OFF \
    -DOPUS_BUILD_TESTING=OFF ${hardening[@]+"${hardening[@]}"}
}

build_lame() {
  unpack lame
  autotools_install "$UNPACKED" --disable-frontend --disable-decoder --disable-gtktest
}

build_libwebp() {
  unpack libwebp
  cmake_install "$UNPACKED" -DWEBP_BUILD_ANIM_UTILS=OFF -DWEBP_BUILD_CWEBP=OFF \
    -DWEBP_BUILD_DWEBP=OFF -DWEBP_BUILD_GIF2WEBP=OFF -DWEBP_BUILD_IMG2WEBP=OFF \
    -DWEBP_BUILD_VWEBP=OFF -DWEBP_BUILD_WEBPINFO=OFF -DWEBP_BUILD_WEBPMUX=OFF \
    -DWEBP_BUILD_EXTRAS=OFF -DWEBP_BUILD_LIBWEBPMUX=ON
}

build_svt_av1() {
  local extra=()
  [ "$OS" = windows ] && extra=(-DENABLE_STACK_PROTECTOR=OFF)
  unpack svt-av1
  cmake_install "$UNPACKED" -DBUILD_APPS=OFF -DREPRODUCIBLE_BUILDS=ON \
    ${extra[@]+"${extra[@]}"}
}

build_ffmpeg() {
  local src args
  unpack ffmpeg
  src=$UNPACKED
  args=(
    --prefix="$PREFIX"
    --pkg-config=pkg-config
    --pkg-config-flags=--static
    --cc="$CC"
    --cxx="$CXX"
    --extra-cflags="-I$PREFIX/include $CFLAGS"
    --extra-cxxflags="-I$PREFIX/include $CXXFLAGS"
    --extra-ldflags="-L$PREFIX/lib $LDFLAGS"
    --extra-version=catconvert
    --enable-gpl
    --disable-shared
    --enable-static
    # Nothing found by accident: only the libraries listed below.
    --disable-autodetect
    --disable-debug
    --disable-doc
    --disable-ffplay
    --disable-network
    --disable-indevs
    --enable-indev=lavfi
    --disable-outdevs
    --enable-zlib
    --enable-libx264
    --enable-libvpx
    --enable-libopus
    --enable-libmp3lame
    --enable-libwebp
    --enable-libsvtav1
    --enable-libjxl
  )
  case $OS in
    macos)
      args+=(--enable-videotoolbox --enable-audiotoolbox --extra-libs=-lc++)
      if [ "$CROSS" = 1 ]; then
        args+=(--enable-cross-compile --arch="$ARCH" --target-os=darwin)
      fi
      ;;
    linux)
      # libjxl and highway are C++; keep their runtime inside the binary.
      args+=("--extra-libs=-Wl,-Bstatic -lstdc++ -Wl,-Bdynamic -lm -lpthread")
      ;;
    windows)
      # libjxl's headers assume a DLL unless told otherwise; its .pc files say
      # so only in Cflags.private, which not every pkg-config reads.
      args+=(--enable-cross-compile --arch=x86_64 --target-os=mingw32
        --cross-prefix="$CROSS_PREFIX" "--extra-libs=-lstdc++ -lpthread"
        "--extra-cflags=-DJXL_STATIC_DEFINE -DJXL_THREADS_STATIC_DEFINE -DJXL_CMS_STATIC_DEFINE")
      ;;
  esac
  mkdir -p "$src/_build"
  (
    cd "$src/_build"
    ../configure "${args[@]}" || {
      tail -n 60 ffbuild/config.log >&2
      exit 1
    }
    make -j"$JOBS"
  )
}

# ---------------------------------------------------------------------------
# Output and checks
# ---------------------------------------------------------------------------

# The libraries a binary links dynamically, one per line.
linked_libs() {
  case $OS in
    macos) otool -L "$1" | tail -n +2 | awk '{print $1}' ;;
    linux) readelf -d "$1" | awk '/NEEDED/ { gsub(/[][]/, "", $5); print $5 }' ;;
    windows) "${CROSS_PREFIX}objdump" -p "$1" | awk '/DLL Name:/ { print $3 }' ;;
  esac
}

# Fail if a binary links anything but the OS's own libraries.
check_linkage() {
  local bin=$1 lib bad=""
  for lib in $(linked_libs "$bin"); do
    case $OS in
      macos) case $lib in /usr/lib/* | /System/Library/*) continue ;; esac ;;
      linux)
        case $lib in
          libc.so.* | libm.so.* | libpthread.so.* | libdl.so.* | librt.so.* | ld-linux-*.so.* | libmvec.so.*) continue ;;
        esac
        ;;
      windows)
        # mingw runtime DLLs (libwinpthread-1.dll, libstdc++-6.dll, ...) must be linked in.
        case $(printf '%s' "$lib" | tr '[:upper:]' '[:lower:]') in
          lib*.dll | zlib*.dll) ;;
          *) continue ;;
        esac
        ;;
    esac
    bad="$bad $lib"
  done
  [ -z "$bad" ] || die "$(basename "$bin") links libraries the OS doesn't ship:$bad"
}

# Run the new binaries when this machine can, and make sure every encoder the
# app needs is in.
smoke_test() {
  local dir=$1 encoders enc missing="" run=()
  case $OS in
    macos)
      if [ "$CROSS" = 1 ]; then
        arch -"$MARCH" /usr/bin/true 2>/dev/null || {
          log "Skipping smoke test: this Mac can't run $MARCH"
          return
        }
        run=(arch -"$MARCH")
      fi
      ;;
    windows)
      log "Skipping smoke test: Windows binaries are cross-compiled"
      return
      ;;
  esac
  local ffmpeg="$dir/${NAME_PREFIX}ffmpeg-$TARGET$EXE"
  encoders=$(${run[@]+"${run[@]}"} "$ffmpeg" -hide_banner -encoders)
  for enc in $REQUIRED_ENCODERS; do
    printf '%s\n' "$encoders" | awk '{print $2}' | grep -qxF "$enc" || missing="$missing $enc"
  done
  [ -z "$missing" ] || die "ffmpeg lacks encoders the app needs:$missing"
  ${run[@]+"${run[@]}"} "$dir/${NAME_PREFIX}ffprobe-$TARGET$EXE" -hide_banner -version >/dev/null
  ${run[@]+"${run[@]}"} "$dir/${NAME_PREFIX}cjxl-$TARGET$EXE" --version >/dev/null
  ${run[@]+"${run[@]}"} "$dir/${NAME_PREFIX}djxl-$TARGET$EXE" --version >/dev/null
  log "Smoke test passed: $(${run[@]+"${run[@]}"} "$ffmpeg" -hide_banner -version | head -n 1)"
}

# Copy, strip and check the tools of the current target into <dir>.
collect() {
  local dir=$1 tool from to
  mkdir -p "$dir"
  for tool in $TOOLS; do
    case $tool in
      ffmpeg | ffprobe) from="$SRC/ffmpeg/_build/$tool$EXE" ;;
      *) from="$PREFIX/bin/$tool$EXE" ;;
    esac
    [ -f "$from" ] || die "$from was not built"
    to="$dir/${NAME_PREFIX}$tool-$TARGET$EXE"
    cp "$from" "$to"
    chmod 755 "$to"
    # shellcheck disable=SC2086 # STRIP_CMD may carry a flag
    $STRIP_CMD "$to"
    check_linkage "$to"
  done
}

write_licenses() {
  local src=$1 out=$2 name file rule
  rule=$(printf '%.0s=' $(seq 1 78))
  {
    echo "Third-party software in CatConvert"
    echo
    echo "CatConvert runs these programs, which ship inside the app, as separate processes:"
    echo "ffmpeg and ffprobe (GPL-2.0-or-later, because they include x264), and cjxl, djxl"
    echo "and jxlinfo from libjxl. They were built from the unmodified upstream sources"
    echo "listed below. The complete corresponding source code, together with the script"
    echo "that built these programs, is attached to every CatConvert release as"
    echo "catconvert-<version>-media-tools-source.tar.xz:"
    echo "https://github.com/Jotrorox/catconvert/releases"
    for name in $(components); do
      printf '\n%s\n%s %s (%s)\n%s\n%s\n' "$rule" "$name" "$(lock "$name" 2)" \
        "$(lock "$name" 3 | sed 's/,/ AND /g')" "$(lock "$name" 4)" "$rule"
      for file in $(lock "$name" 6 | tr ',' ' '); do
        [ -f "$src/$name/$file" ] || die "$name has no $file"
        printf '\n'
        cat "$src/$name/$file"
      done
    done
  } >"$out"
}

mb() { awk -v b="$1" 'BEGIN { printf "%6.1f", b / 1048576 }'; }

check_size() {
  local dir=$1 target=$2 total=0 bytes file budget=$MAX_MB
  [ "$target" = universal-apple-darwin ] && budget=$((MAX_MB * 2))
  log "Sizes for $target"
  for file in "$dir/${NAME_PREFIX}"*"-$target"*; do
    bytes=$(wc -c <"$file" | tr -d ' ')
    total=$((total + bytes))
    printf '  %s MB  %s\n' "$(mb "$bytes")" "$(basename "$file")"
  done
  printf '  %s MB  total (budget %d MB)\n' "$(mb "$total")" "$budget"
  [ "$total" -le $((budget * 1048576)) ] ||
    die "the bundled tools grew past the $budget MB budget; raise MEDIA_TOOLS_MAX_MB if that's intended"
}

build_target() {
  local target=$1 out=$2
  setup_target "$target"
  log "Building media tools for $TARGET in $BUILD"
  build_zlib
  build_libpng
  build_giflib
  build_libjpeg_turbo
  build_lcms2
  build_brotli
  build_highway
  build_libjxl
  build_x264
  build_libvpx
  build_opus
  build_lame
  build_libwebp
  build_svt_av1
  build_ffmpeg
  collect "$out"
  smoke_test "$out"
}

build() {
  local target=$1 out=$2
  case $target in
    universal-apple-darwin)
      local arch tool
      for arch in aarch64 x86_64; do
        build_target "$arch-apple-darwin" "$WORK/out/$arch-apple-darwin"
      done
      mkdir -p "$out"
      for tool in $TOOLS; do
        lipo -create -output "$out/${NAME_PREFIX}$tool-$target" \
          "$WORK/out/aarch64-apple-darwin/${NAME_PREFIX}$tool-aarch64-apple-darwin" \
          "$WORK/out/x86_64-apple-darwin/${NAME_PREFIX}$tool-x86_64-apple-darwin"
      done
      ;;
    *)
      build_target "$target" "$out"
      ;;
  esac
  write_licenses "$SRC" "$out/THIRD-PARTY-LICENSES.txt"
  check_size "$out" "$target"
  log "Done: $out"
}

pack_sources() {
  local dest=$1 stage name
  stage="$WORK/source-stage/catconvert-media-tools-source"
  rm -rf "$WORK/source-stage"
  mkdir -p "$stage/sources"
  for name in $(components); do
    tarball "$name"
    cp "$TARBALL" "$stage/sources/"
  done
  cp "$HERE/build.sh" "$HERE/sources.lock" "$stage/"
  cat >"$stage/README" <<'EOF'
Corresponding source of the media tools bundled with CatConvert: ffmpeg and
ffprobe (GPL-2.0-or-later, with x264) and libjxl's cjxl, djxl and jxlinfo.

sources/      the exact upstream source archives the tools were built from
sources.lock  versions, origins and SHA-256 checksums of those archives
build.sh      the script that built them

To rebuild, for example for Linux:

    MEDIA_TOOLS_SOURCES="$PWD/sources" ./build.sh x86_64-unknown-linux-gnu out

See the top of build.sh for targets and required tools.
EOF
  mkdir -p "$(dirname "$dest")"
  tar -cJf "$dest" -C "$WORK/source-stage" catconvert-media-tools-source
  log "Wrote $dest"
}

main() {
  [ $# -ge 1 ] || die "usage: build.sh <target> [out-dir] | build.sh sources <file.tar.xz>"
  case $1 in
    -h | --help) sed -n '2,/^set -euo/p' "$0" | sed '$d; s/^# \{0,1\}//' ;;
    sources)
      [ $# -eq 2 ] || die "usage: build.sh sources <file.tar.xz>"
      pack_sources "$2"
      ;;
    *)
      local out=${2:-$DEFAULT_OUT}
      mkdir -p "$out"
      build "$1" "$(cd "$out" && pwd)"
      ;;
  esac
}

main "$@"
