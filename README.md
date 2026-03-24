# Xcode AI Proxy Rust

给 Xcode 使用的本地代理，按 OpenAI 兼容格式转发到你的上游模型服务。

## 特性

- 全部按 OpenAI 兼容格式转发
- 支持任意 OpenAI 兼容上游
- 支持流式透传（SSE）
- 支持重试、超时、CORS
- 提供 `xcodeaiproxy` / `xcodeaiproxy-stop` 命令

## 快速开始

1. 获取项目并进入目录

```bash
git clone https://github.com/BiBoyang/xcode_ai_proxy.git
cd xcode_ai_proxy
```

2. 安装命令到本机

```bash
./install.sh
```

3. 首次配置（交互式）

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

4. 启动和停止

```bash
xcodeaiproxy
xcodeaiproxy-stop
```

## 安装说明

`./install.sh` 会把以下命令安装到 `~/.local/bin`：

- `xcodeaiproxy`
- `xcodeaiproxy-stop`

可选：复制安装（非符号链接）：

```bash
./install.sh --copy
```

若使用 `--copy` 且命令无法自动定位项目目录，可设置：

```bash
export XCODEAIPROXY_HOME="/path/to/xcode_ai_proxy"
```

若终端提示 `command not found`，请加入 PATH：

```bash
export PATH="$HOME/.local/bin:$PATH"
```

## 命令说明

- `xcodeaiproxy` 或 `xcodeaiproxy start`：后台启动服务（默认端口 3000）
- `xcodeaiproxy-stop` 或 `xcodeaiproxy stop`：一键停止服务
- `xcodeaiproxy restart`：重启服务（改配置后常用）
- `xcodeaiproxy status`：查看运行状态详情（端口、健康检查、PID、日志路径）
- `xcodeaiproxy models`：查看当前默认模型与可切换模型列表
- `xcodeaiproxy use-model <模型ID>`：切换默认模型来源（写入 `OPENAI_*`）
- `xcodeaiproxy logs`：实时查看日志（排错用）
- `xcodeaiproxy run`：前台运行（调试用，会占用当前终端）
- `xcodeaiproxy setup`：交互式写入/更新 `.env`

临时指定端口（只对当前命令生效）：

```bash
PORT=3020 xcodeaiproxy
PORT=3020 xcodeaiproxy-stop
```

## 后台运行行为

- `xcodeaiproxy` 默认以后台方式启动（`nohup`），关闭当前终端后进程仍会继续运行
- 服务会持续运行，直到你主动停止、进程异常退出，或机器重启
- 默认不是系统服务：不会开机自启，也不会自动拉起
- 运行状态检查：`xcodeaiproxy status`
- 停止服务：`xcodeaiproxy-stop` 或 `xcodeaiproxy stop`

## Xcode 配置

- Base URL: `http://localhost:3000`（如改端口请同步修改）
- API Key: `any-string-works`（任意字符串）
- Model（本地模型名）: `DefaultModel`（推荐）

说明：

- `DefaultModel` 是默认模型 id，`setup` 会写入 `DEFAULT_MODEL_ID=DefaultModel`
- 如果你想固定用某个追加模型，也可以在 Xcode 里直接填 `ModelA` / `ModelB`
- 如果你不想每次改 Xcode，建议一直填 `DefaultModel`，然后在终端切换：

```bash
xcodeaiproxy models
xcodeaiproxy use-model ModelA
xcodeaiproxy restart
```

## MCP 与 Skills

- 当前项目是 OpenAI 兼容请求代理，不包含 MCP 客户端与 Skills 编排能力
- 因此通过 Xcode + `xcodeaiproxy` 这条链路，默认不能直接使用 MCP / Skills
- 推荐用法 1：在 Xcode 中继续使用本代理完成日常对话与代码辅助
- 推荐用法 2：需要 MCP / Skills 时，在终端使用支持这些能力的 Agent 工具处理同一项目

## 配置说明

- 推荐使用 `xcodeaiproxy setup` 进行交互式配置（会写入项目根目录 `.env`）
- `setup` 会写入默认模型：`DEFAULT_MODEL_ID=DefaultModel` + `OPENAI_BASE_URL/OPENAI_API_KEY/OPENAI_MODEL`
- `setup` 交互里会明确区分 `当前值` 与 `示例`，避免误读
- `OPENAI_BASE_URL` 必须以 `http://` 或 `https://` 开头，且不能包含空格
- `OPENAI_API_KEY` 不能为空、不能有空格、长度至少 8
- `OPENAI_MODEL` 只允许字母、数字和 `._:/-` 字符
- `PORT` 提示默认 `3000`，回车可直接使用默认值
- `xcodeaiproxy start` 启动前会再次校验上述配置，格式不对会提示执行 `xcodeaiproxy setup`
- 真机调试请使用 Mac 局域网 IP，不要用 `localhost`

### 多供应商模型（手动追加）

`setup` 只负责默认模型。你可以在 `.env` 追加更多模型（OpenAI 兼容格式）：

```env
# 默认模型（setup 写入）
DEFAULT_MODEL_ID=DefaultModel
OPENAI_BASE_URL=https://api.deepseek.com/v1
OPENAI_API_KEY=sk-xxxx
OPENAI_MODEL=deepseek-chat

# 追加模型列表（模型 id 用逗号分隔）
EXTRA_MODEL_IDS=ModelA,ModelB

# ModelA
MODEL_ModelA_BASE_URL=https://api.openai.com/v1
MODEL_ModelA_API_KEY=sk-openai-xxxx
MODEL_ModelA_MODEL=gpt-4.1-mini
MODEL_ModelA_NAME=OpenAI GPT-4.1 mini
MODEL_ModelA_PROVIDER=openai

# ModelB
MODEL_ModelB_BASE_URL=https://api.moonshot.cn/v1
MODEL_ModelB_API_KEY=sk-kimi-xxxx
MODEL_ModelB_MODEL=kimi-k2-0711-preview
MODEL_ModelB_NAME=Kimi K2
MODEL_ModelB_PROVIDER=moonshot
```

说明：

- `EXTRA_MODEL_IDS` 中的 id 仅支持字母/数字/下划线，且不能以数字开头
- 每个模型必须同时配置 `BASE_URL`、`API_KEY`、`MODEL`
- 查看模型：`xcodeaiproxy models`
- 切换默认模型：`xcodeaiproxy use-model ModelA`
- 改完 `.env` 后执行 `xcodeaiproxy restart`
- 模型列表可通过 `GET /v1/models` 查看，客户端可按模型 id 手动切换

## 开发者信息（接口）

- `GET /health`
- `GET /debug/config`
- `GET /v1/models`
- `POST /v1/chat/completions`
- `POST /api/v1/chat/completions`
- `POST /v1/messages`
