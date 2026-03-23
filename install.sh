#!/bin/bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$SCRIPT_DIR"
BIN_SRC_DIR="$PROJECT_DIR/bin"
INSTALL_DIR="${INSTALL_DIR:-$HOME/.local/bin}"
MODE="symlink"

usage() {
  cat <<'EOF'
用法:
  ./install.sh [--copy] [--install-dir <path>]

参数:
  --copy                 复制脚本到目标目录（默认使用符号链接）
  --install-dir <path>   指定安装目录（默认: ~/.local/bin）
EOF
}

while [ $# -gt 0 ]; do
  case "$1" in
    --copy)
      MODE="copy"
      shift
      ;;
    --install-dir)
      if [ $# -lt 2 ]; then
        echo "❌ --install-dir 需要路径参数"
        exit 1
      fi
      INSTALL_DIR="$2"
      shift 2
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

if [ ! -d "$BIN_SRC_DIR" ]; then
  echo "❌ 未找到脚本目录: $BIN_SRC_DIR"
  exit 1
fi

mkdir -p "$INSTALL_DIR"
chmod +x "$BIN_SRC_DIR/xcodeaiproxy" "$BIN_SRC_DIR/xcodeaiproxy-stop"

install_one() {
  local name="$1"
  local src="$BIN_SRC_DIR/$name"
  local dst="$INSTALL_DIR/$name"

  if [ "$MODE" = "copy" ]; then
    cp "$src" "$dst"
    chmod +x "$dst"
    echo "✅ copied: $dst"
  else
    ln -sfn "$src" "$dst"
    echo "✅ linked: $dst -> $src"
  fi
}

install_one "xcodeaiproxy"
install_one "xcodeaiproxy-stop"

echo
if [ "$MODE" = "copy" ]; then
  echo "⚠️ 当前为 copy 模式。若命令无法自动定位项目目录，请设置："
  echo "  export XCODEAIPROXY_HOME=\"$PROJECT_DIR\""
  echo
fi

if [[ ":$PATH:" != *":$INSTALL_DIR:"* ]]; then
  echo "⚠️ 你的 PATH 里没有 $INSTALL_DIR"
  echo "请在 shell 配置中加入："
  echo "  export PATH=\"$INSTALL_DIR:\$PATH\""
  echo
fi

echo "安装完成。"
echo "下一步："
echo "  1) xcodeaiproxy setup"
echo "  2) xcodeaiproxy"
echo "  3) xcodeaiproxy-stop"
