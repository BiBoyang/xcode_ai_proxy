#!/bin/bash
set -euo pipefail

echo "🚀 启动 Xcode AI Proxy Rust 版本"

if ! command -v cargo >/dev/null 2>&1; then
  echo "❌ 未找到 cargo，请先安装 Rust 工具链"
  exit 1
fi

if [ ! -f .env ]; then
  echo "⚠️ 未找到 .env，正在从 .env.example 复制..."
  cp .env.example .env
  echo "📝 请先编辑 .env 后再重新运行"
  exit 1
fi

echo "🔧 启动服务..."
cargo run
