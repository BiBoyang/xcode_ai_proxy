# Xcode AI Proxy Rust

Rust 版本的 Xcode AI 本地代理，功能对齐 Python 版并支持一键命令行操作。

## 特性

- 全部按 OpenAI 兼容格式转发
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

如果你的 PATH 不包含 `~/.local/bin`，请手动加入：

```bash
export PATH="$HOME/.local/bin:$PATH"
```

## 最简单使用流程

1. 首次配置（交互式）

```bash
xcodeaiproxy setup
```

你会看到类似下面的提示（`当前值` 是你正在使用的值，`示例` 仅供参考）：

```text
OPENAI_BASE_URL（OpenAI 兼容接口地址）
当前值: https://api.deepseek.com/v1
示例:   https://api.openai.com/v1
请输入新值（回车保留当前值）:
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

- 推荐使用 `xcodeaiproxy setup` 进行交互式配置（会写入项目根目录 `.env`）
- `setup` 交互里会明确区分 `当前值` 与 `示例`，避免误读
- `OPENAI_BASE_URL` 必须以 `http://` 或 `https://` 开头，且不能包含空格
- `OPENAI_API_KEY` 不能为空、不能有空格、长度至少 8
- `OPENAI_MODEL` 只允许字母、数字和 `._:/-` 字符
- `PORT` 提示为 `默认3000，回车直接使用`；不填写会自动使用 `3000`
- `xcodeaiproxy start` 启动前会再次校验上述配置，格式不对会提示执行 `xcodeaiproxy setup`
- 真机调试请使用 Mac 局域网 IP，不要用 `localhost`
