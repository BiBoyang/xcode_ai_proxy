# Xcode AI Proxy Rust

Rust 版本的 Xcode AI 本地代理，功能对齐 Python 版并支持一键命令行操作。

## 特性

- 支持 `GLM/Kimi/DeepSeek`
- 支持任意 OpenAI 兼容上游
- 支持流式透传（SSE）
- 支持重试、超时、CORS
- 提供 `xcodeaiproxy` / `xcodeaiproxy-stop` 命令

## 接口

- `GET /health`
- `GET /debug/config`
- `GET /v1/models`
- `POST /v1/chat/completions`
- `POST /api/v1/chat/completions`
- `POST /v1/messages`

## 安装（推荐）

```bash
./install.sh
```

安装后会把命令安装到 `~/.local/bin`：

- `xcodeaiproxy`
- `xcodeaiproxy-stop`

可选：复制安装（非符号链接）：

```bash
./install.sh --copy
```

若使用 `--copy` 且命令无法自动定位项目目录，可设置：

```bash
export XCODEAIPROXY_HOME="/path/to/xcode-ai-proxy-rust"
```

## 打包发布（GitHub Releases）

一键生成发布目录、压缩包和校验文件：

```bash
./release.sh
```

常见选项：

```bash
# 指定 target
./release.sh --target aarch64-apple-darwin

# 只要 tar.gz
./release.sh --no-zip

# 打包前清理 dist
./release.sh --clean
```

打包结果会输出在 `dist/`，包含：

- 发布目录（内含二进制 + 脚本 + install.sh + .env.example）
- `.tar.gz` / `.zip`
- 对应 `.sha256`

如果你的 PATH 不包含 `~/.local/bin`，请手动加入：

```bash
export PATH="$HOME/.local/bin:$PATH"
```

## 最简单使用流程

1. 首次配置（交互式）

```bash
xcodeaiproxy setup
```

2. 后台启动（默认端口 3000）

```bash
xcodeaiproxy
```

3. 停止

```bash
xcodeaiproxy-stop
```

## 常用命令

```bash
xcodeaiproxy start
xcodeaiproxy stop
xcodeaiproxy restart
xcodeaiproxy status
xcodeaiproxy logs
xcodeaiproxy run
```

指定端口（临时）：

```bash
PORT=3020 xcodeaiproxy
PORT=3020 xcodeaiproxy-stop
```

## Xcode 配置

- Base URL: `http://localhost:3000`（如改端口请同步修改）
- API Key: `any-string-works`（任意字符串）

## 配置说明

- 新变量名：`OPENAI_COMPAT_*`
- 兼容旧变量名：`OPENAI_BASE_URL` / `OPENAI_API_KEY` / `OPENAI_MODEL`
- `OPENAI_MODEL`/`OPENAI_COMPAT_MODELS` 支持多个模型（逗号分隔）
- 真机调试请使用 Mac 局域网 IP，不要用 `localhost`
