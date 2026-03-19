#!/bin/bash
# Builds Zebar and creates a macOS app bundle.
#
# Usage:
#   ./resources/scripts/build-macos.sh [--release]
#
# Options:
#   --release    Build in release mode (default: debug)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

BUILD_MODE="debug"
CARGO_FLAGS=""

for arg in "$@"; do
  case $arg in
    --release)
      BUILD_MODE="release"
      CARGO_FLAGS="--release"
      ;;
  esac
done

# Corepack/pnpm can fail with ENOENT when its tools directory has not
# been created yet (for example: $XDG_DATA_HOME/pnpm/.tools/pnpm).
# Create it up front so `pnpm install` can download the package-manager
# version declared in package.json reliably.
# Use a project-local XDG data directory for pnpm during this build. This
# avoids failures when inherited env vars point at non-macOS paths such as
# /home/<user>.
export XDG_DATA_HOME="$PROJECT_ROOT/target/.pnpm-data"
PNPM_DATA_DIR="$XDG_DATA_HOME/pnpm"

mkdir -p "$PNPM_DATA_DIR/.tools/pnpm"

if ! command -v pnpm >/dev/null 2>&1; then
  echo "pnpm not found. Enable corepack or install pnpm first." >&2
  exit 1
fi

echo "Installing dependencies..."
pnpm install --dir "$PROJECT_ROOT"

echo "Building frontend..."
pnpm run --dir "$PROJECT_ROOT" --filter zebar --filter @zebar/settings-ui build

echo "Building Zebar ($BUILD_MODE)..."
cargo build $CARGO_FLAGS -p zebar --features custom-protocol --manifest-path "$PROJECT_ROOT/Cargo.toml"

BUILD_DIR="$PROJECT_ROOT/target/$BUILD_MODE"
APP_DIR="$BUILD_DIR/Zebar.app"
CONTENTS_DIR="$APP_DIR/Contents"

echo "Creating app bundle at $APP_DIR..."

rm -rf "$APP_DIR"
mkdir -p "$CONTENTS_DIR/MacOS" "$CONTENTS_DIR/Resources"

cp "$BUILD_DIR/zebar" "$CONTENTS_DIR/MacOS/"
chmod +x "$CONTENTS_DIR/MacOS"/*

# Convert PNG icon to ICNS.
ICONSET_DIR="$BUILD_DIR/icon.iconset"
rm -rf "$ICONSET_DIR"
mkdir -p "$ICONSET_DIR"

ICON_PNG="$PROJECT_ROOT/packages/desktop/resources/icons/icon.png"
sips -z 16 16     "$ICON_PNG" --out "$ICONSET_DIR/icon_16x16.png"      > /dev/null
sips -z 32 32     "$ICON_PNG" --out "$ICONSET_DIR/icon_16x16@2x.png"   > /dev/null
sips -z 32 32     "$ICON_PNG" --out "$ICONSET_DIR/icon_32x32.png"      > /dev/null
sips -z 64 64     "$ICON_PNG" --out "$ICONSET_DIR/icon_32x32@2x.png"   > /dev/null
sips -z 128 128   "$ICON_PNG" --out "$ICONSET_DIR/icon_128x128.png"    > /dev/null
sips -z 256 256   "$ICON_PNG" --out "$ICONSET_DIR/icon_128x128@2x.png" > /dev/null
sips -z 256 256   "$ICON_PNG" --out "$ICONSET_DIR/icon_256x256.png"    > /dev/null
sips -z 512 512   "$ICON_PNG" --out "$ICONSET_DIR/icon_256x256@2x.png" > /dev/null
sips -z 512 512   "$ICON_PNG" --out "$ICONSET_DIR/icon_512x512.png"    > /dev/null
sips -z 1024 1024 "$ICON_PNG" --out "$ICONSET_DIR/icon_512x512@2x.png" > /dev/null
iconutil -c icns "$ICONSET_DIR" -o "$CONTENTS_DIR/Resources/icon.icns"
rm -rf "$ICONSET_DIR"

VERSION="${VERSION_NUMBER:-0.0.0}"
sed "s/\${VERSION}/$VERSION/g" "$PROJECT_ROOT/resources/Info.plist" > "$CONTENTS_DIR/Info.plist"

echo -n "APPL????" > "$CONTENTS_DIR/PkgInfo"

echo "Done: $APP_DIR"
