#!/bin/bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$SCRIPT_DIR"
CARGO_TOML="$PROJECT_DIR/Cargo.toml"
DIST_DIR="$PROJECT_DIR/dist"
CREATE_TAR=1
CREATE_ZIP=1

usage() {
  cat <<'EOF'
用法:
  ./release.sh [--target <triple>] [--no-tar] [--no-zip] [--clean]

参数:
  --target <triple>  指定编译目标（默认使用 rustc host）
  --no-tar           不生成 .tar.gz
  --no-zip           不生成 .zip
  --clean            打包前清理 dist 目录
EOF
}

if [ ! -f "$CARGO_TOML" ]; then
  echo "❌ 未找到 Cargo.toml: $CARGO_TOML"
  exit 1
fi

CRATE_NAME="$(awk -F\" '/^name = /{print $2; exit}' "$CARGO_TOML")"
VERSION="$(awk -F\" '/^version = /{print $2; exit}' "$CARGO_TOML")"

if [ -z "${CRATE_NAME:-}" ] || [ -z "${VERSION:-}" ]; then
  echo "❌ 无法从 Cargo.toml 读取 name/version"
  exit 1
fi

DEFAULT_TARGET="$(rustc -vV | awk '/^host: /{host=$2} END {print host}')"
TARGET="${DEFAULT_TARGET}"
CLEAN=0

while [ $# -gt 0 ]; do
  case "$1" in
    --target)
      if [ $# -lt 2 ]; then
        echo "❌ --target 需要参数"
        exit 1
      fi
      TARGET="$2"
      shift 2
      ;;
    --no-tar)
      CREATE_TAR=0
      shift
      ;;
    --no-zip)
      CREATE_ZIP=0
      shift
      ;;
    --clean)
      CLEAN=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "❌ 未知参数: $1"
      usage
      exit 1
      ;;
  esac
done

if [ "$CREATE_TAR" -eq 0 ] && [ "$CREATE_ZIP" -eq 0 ]; then
  echo "❌ 至少需要一种输出格式（tar 或 zip）"
  exit 1
fi

if [ "$CLEAN" -eq 1 ]; then
  rm -rf "$DIST_DIR"
fi
mkdir -p "$DIST_DIR"

echo "==> 编译 release 二进制"
echo "crate:  $CRATE_NAME"
echo "ver:    $VERSION"
echo "target: $TARGET"

cargo build --release --target "$TARGET"

BIN_SRC="$PROJECT_DIR/target/$TARGET/release/$CRATE_NAME"
if [ ! -x "$BIN_SRC" ]; then
  echo "❌ 编译完成但未找到可执行文件: $BIN_SRC"
  exit 1
fi

PACKAGE_BASENAME="${CRATE_NAME}-v${VERSION}-${TARGET}"
STAGE_DIR="$DIST_DIR/$PACKAGE_BASENAME"
TAR_FILE="$DIST_DIR/${PACKAGE_BASENAME}.tar.gz"
ZIP_FILE="$DIST_DIR/${PACKAGE_BASENAME}.zip"

rm -rf "$STAGE_DIR"
mkdir -p "$STAGE_DIR/bin"

echo "==> 组装发布目录: $STAGE_DIR"
cp "$BIN_SRC" "$STAGE_DIR/$CRATE_NAME"
cp "$PROJECT_DIR/bin/xcodeaiproxy" "$PROJECT_DIR/bin/xcodeaiproxy-stop" "$STAGE_DIR/bin/"
cp "$PROJECT_DIR/install.sh" "$PROJECT_DIR/.env.example" "$PROJECT_DIR/README.md" "$STAGE_DIR/"

chmod +x "$STAGE_DIR/$CRATE_NAME" "$STAGE_DIR/install.sh" "$STAGE_DIR/bin/xcodeaiproxy" "$STAGE_DIR/bin/xcodeaiproxy-stop"

if [ "$CREATE_TAR" -eq 1 ]; then
  echo "==> 生成 tar.gz"
  rm -f "$TAR_FILE"
  tar -C "$DIST_DIR" -czf "$TAR_FILE" "$PACKAGE_BASENAME"
  shasum -a 256 "$TAR_FILE" > "${TAR_FILE}.sha256"
fi

if [ "$CREATE_ZIP" -eq 1 ]; then
  if command -v zip >/dev/null 2>&1; then
    echo "==> 生成 zip"
    rm -f "$ZIP_FILE"
    (
      cd "$DIST_DIR"
      zip -qr "$ZIP_FILE" "$PACKAGE_BASENAME"
    )
    shasum -a 256 "$ZIP_FILE" > "${ZIP_FILE}.sha256"
  else
    echo "⚠️ 系统没有 zip，已跳过 zip 打包"
  fi
fi

echo
echo "✅ 打包完成。产物目录: $DIST_DIR"
ls -lah "$DIST_DIR" | sed -n '1,200p'
echo
echo "GitHub Releases 上传示例："
echo "  gh release create v$VERSION \"$DIST_DIR\"/*.tar.gz \"$DIST_DIR\"/*.zip \"$DIST_DIR\"/*.sha256 --title \"v$VERSION\" --notes \"release\""
