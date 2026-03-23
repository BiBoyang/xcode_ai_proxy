use axum::{
    Json, Router,
    body::Body,
    extract::State,
    http::{Response, StatusCode},
    response::IntoResponse,
    routing::{get, post},
};
use chrono::Utc;
use dotenvy::dotenv;
use futures_util::TryStreamExt;
use reqwest::Client;
use serde::Serialize;
use serde_json::{Map, Value, json};
use std::{collections::HashMap, env, sync::Arc, time::Duration};
use tokio::{net::TcpListener, time::sleep};
use tower_http::{
    cors::{Any, CorsLayer},
    trace::TraceLayer,
};
use tracing::{error, info, warn};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[derive(Clone, Debug)]
struct ModelConfig {
    api_url: String,
    api_key: String,
    model_type: String,
    name: String,
    upstream_model: Option<String>,
    provider: Option<String>,
}

#[derive(Debug)]
struct AppConfig {
    host: String,
    port: u16,
    max_retries: usize,
    retry_delay_ms: u64,
    request_timeout_ms: u64,
    api_configs: HashMap<String, ModelConfig>,
}

#[derive(Clone)]
struct AppState {
    client: Client,
    config: Arc<AppConfig>,
}

#[derive(Debug)]
enum AppError {
    InvalidRequest(String),
    UpstreamApi(StatusCode, String),
    Network(String),
    Proxy(String),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response<Body> {
        match self {
            Self::InvalidRequest(message) => {
                error_response(StatusCode::BAD_REQUEST, message, "invalid_request_error")
            }
            Self::UpstreamApi(status, upstream_message) => error_response(
                status,
                format!("API 请求失败: {} - {}", status.as_u16(), upstream_message),
                "api_error",
            ),
            Self::Network(message) => {
                error_response(StatusCode::INTERNAL_SERVER_ERROR, message, "network_error")
            }
            Self::Proxy(message) => {
                error_response(StatusCode::INTERNAL_SERVER_ERROR, message, "proxy_error")
            }
        }
    }
}

#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
    timestamp: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenv().ok();
    init_tracing();

    let config = match load_config() {
        Ok(config) => config,
        Err(message) => {
            error!("{message}");
            std::process::exit(1);
        }
    };

    info!("🚀 Xcode AI Proxy (Rust) 已启动");
    info!("📡 监听地址: http://{}:{}", config.host, config.port);
    info!("🎯 当前可用模型:");
    let mut model_ids: Vec<_> = config.api_configs.keys().cloned().collect();
    model_ids.sort();
    for model_id in model_ids {
        if let Some(model_config) = config.api_configs.get(&model_id) {
            info!("   ✅ {model_id} ({})", model_config.name);
        }
    }
    info!("⚙️ 重试配置:");
    info!("   最大重试次数: {}", config.max_retries);
    info!("   重试延迟: {}ms (递增)", config.retry_delay_ms);
    info!("   请求超时: {}ms", config.request_timeout_ms);
    info!("📋 配置 Xcode:");
    info!("   ANTHROPIC_BASE_URL: http://localhost:{}", config.port);
    info!("   ANTHROPIC_AUTH_TOKEN: any-string-works");

    let client = Client::builder()
        .timeout(Duration::from_millis(config.request_timeout_ms))
        .build()
        .map_err(|error| format!("无法创建 HTTP 客户端: {error}"))?;

    let shared_state = Arc::new(AppState {
        client,
        config: Arc::new(config),
    });

    let bind_addr = format!("{}:{}", shared_state.config.host, shared_state.config.port);

    let app = Router::new()
        .route("/health", get(health_check))
        .route("/debug/config", get(debug_config))
        .route("/v1/models", get(list_models))
        .route("/v1/chat/completions", post(chat_completions))
        .route("/api/v1/chat/completions", post(api_chat_completions))
        .route("/v1/messages", post(messages))
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(Any)
                .allow_headers(Any),
        )
        .layer(TraceLayer::new_for_http())
        .with_state(shared_state);

    let listener = TcpListener::bind(&bind_addr)
        .await
        .map_err(|error| format!("监听端口失败: {error}"))?;

    axum::serve(listener, app.into_make_service())
        .await
        .map_err(|error| format!("服务运行失败: {error}"))?;

    Ok(())
}

fn init_tracing() {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "xcode_ai_proxy_rust=info,tower_http=info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();
}

fn load_config() -> Result<AppConfig, String> {
    let host = env_non_empty("HOST").unwrap_or_else(|| "0.0.0.0".to_string());
    let port = parse_env_u16("PORT", 3000)?;
    let max_retries = parse_env_usize("MAX_RETRIES", 3)?;
    let retry_delay_ms = parse_env_u64("RETRY_DELAY", 1000)?;
    let request_timeout_ms = parse_env_u64("REQUEST_TIMEOUT", 60_000)?;

    let builtin_provider_env_vars = [
        ("ZHIPU_API_KEY", "GLM-4.5 模型"),
        ("KIMI_API_KEY", "Kimi 模型"),
        ("DEEPSEEK_API_KEY", "DeepSeek 模型"),
    ];
    for (env_key, model_name) in builtin_provider_env_vars {
        if env_non_empty(env_key).is_none() {
            info!("ℹ️ 未设置 {env_key}，将跳过 {model_name}");
        }
    }

    let mut api_configs = HashMap::new();

    if let Some(zhipu_api_key) = env_non_empty("ZHIPU_API_KEY") {
        add_model_config(
            &mut api_configs,
            "glm-4.5",
            ModelConfig {
                api_url: "https://open.bigmodel.cn/api/paas/v4".to_string(),
                api_key: zhipu_api_key,
                model_type: "zhipu".to_string(),
                name: "GLM-4.5".to_string(),
                upstream_model: Some("glm-4.5".to_string()),
                provider: None,
            },
        );
    }

    if let Some(kimi_api_key) = env_non_empty("KIMI_API_KEY") {
        add_model_config(
            &mut api_configs,
            "kimi-k2-0905-preview",
            ModelConfig {
                api_url: "https://api.moonshot.cn/v1".to_string(),
                api_key: kimi_api_key,
                model_type: "kimi".to_string(),
                name: "Kimi K2".to_string(),
                upstream_model: Some("kimi-k2-0905-preview".to_string()),
                provider: None,
            },
        );
    }

    if let Some(deepseek_api_key) = env_non_empty("DEEPSEEK_API_KEY") {
        add_model_config(
            &mut api_configs,
            "deepseek-reasoner",
            ModelConfig {
                api_url: "https://api.deepseek.com/v1".to_string(),
                api_key: deepseek_api_key.clone(),
                model_type: "deepseek".to_string(),
                name: "DeepSeek Reasoner".to_string(),
                upstream_model: Some("deepseek-reasoner".to_string()),
                provider: None,
            },
        );
        add_model_config(
            &mut api_configs,
            "deepseek-chat",
            ModelConfig {
                api_url: "https://api.deepseek.com/v1".to_string(),
                api_key: deepseek_api_key,
                model_type: "deepseek".to_string(),
                name: "DeepSeek Chat".to_string(),
                upstream_model: Some("deepseek-chat".to_string()),
                provider: None,
            },
        );
    }

    let openai_compat_api_url =
        env_non_empty("OPENAI_COMPAT_API_URL").or_else(|| env_non_empty("OPENAI_BASE_URL"));
    let openai_compat_api_key =
        env_non_empty("OPENAI_COMPAT_API_KEY").or_else(|| env_non_empty("OPENAI_API_KEY"));

    let mut openai_compat_models = parse_csv_env("OPENAI_COMPAT_MODELS");
    if openai_compat_models.is_empty() {
        openai_compat_models = parse_csv_env("OPENAI_MODEL");
    }

    let openai_compat_name_prefix = env_non_empty("OPENAI_COMPAT_NAME_PREFIX")
        .unwrap_or_else(|| "OpenAI-Compatible".to_string());
    let openai_compat_provider_name =
        env_non_empty("OPENAI_COMPAT_PROVIDER_NAME").unwrap_or_else(|| "openai_compat".to_string());

    if openai_compat_api_url.is_some()
        || openai_compat_api_key.is_some()
        || !openai_compat_models.is_empty()
    {
        if openai_compat_api_url.is_none()
            || openai_compat_api_key.is_none()
            || openai_compat_models.is_empty()
        {
            warn!(
                "⚠️ OPENAI_COMPAT_* 配置不完整。需要同时设置 OPENAI_COMPAT_API_URL / OPENAI_COMPAT_API_KEY / OPENAI_COMPAT_MODELS"
            );
        }
    }

    if let (Some(api_url), Some(api_key)) = (openai_compat_api_url, openai_compat_api_key) {
        if !openai_compat_models.is_empty() {
            let normalized_url = normalize_api_url(&api_url);
            for model in openai_compat_models {
                add_model_config(
                    &mut api_configs,
                    &model,
                    ModelConfig {
                        api_url: normalized_url.clone(),
                        api_key: api_key.clone(),
                        model_type: "openai_compat".to_string(),
                        name: format!("{openai_compat_name_prefix} {model}"),
                        upstream_model: Some(model.clone()),
                        provider: Some(openai_compat_provider_name.clone()),
                    },
                );
            }
        }
    }

    if api_configs.is_empty() {
        return Err(
            "❌ 未配置任何模型。请至少设置一个可用配置（ZHIPU/KIMI/DEEPSEEK/OPENAI_COMPAT）"
                .to_string(),
        );
    }

    Ok(AppConfig {
        host,
        port,
        max_retries,
        retry_delay_ms,
        request_timeout_ms,
        api_configs,
    })
}

fn add_model_config(
    api_configs: &mut HashMap<String, ModelConfig>,
    model_id: &str,
    config: ModelConfig,
) {
    if api_configs.contains_key(model_id) {
        warn!("⚠️ 模型 id 冲突: {model_id}，保留首个配置并忽略后续配置");
        return;
    }
    api_configs.insert(model_id.to_string(), config);
}

fn parse_env_u16(key: &str, default_value: u16) -> Result<u16, String> {
    match env_non_empty(key) {
        Some(value) => value
            .parse::<u16>()
            .map_err(|_| format!("环境变量 {key} 不是有效端口: {value}")),
        None => Ok(default_value),
    }
}

fn parse_env_usize(key: &str, default_value: usize) -> Result<usize, String> {
    match env_non_empty(key) {
        Some(value) => value
            .parse::<usize>()
            .map_err(|_| format!("环境变量 {key} 不是有效整数: {value}")),
        None => Ok(default_value),
    }
}

fn parse_env_u64(key: &str, default_value: u64) -> Result<u64, String> {
    match env_non_empty(key) {
        Some(value) => value
            .parse::<u64>()
            .map_err(|_| format!("环境变量 {key} 不是有效整数: {value}")),
        None => Ok(default_value),
    }
}

fn env_non_empty(key: &str) -> Option<String> {
    env::var(key).ok().and_then(|value| {
        let trimmed = value.trim().to_string();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    })
}

fn parse_csv_env(key: &str) -> Vec<String> {
    env_non_empty(key)
        .map(|raw| {
            raw.split(',')
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(ToString::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn normalize_api_url(api_url: &str) -> String {
    let mut normalized = api_url.trim_end_matches('/').to_string();
    if normalized.ends_with("/chat/completions") {
        normalized.truncate(normalized.len() - "/chat/completions".len());
    }
    normalized
}

async fn health_check() -> impl IntoResponse {
    Json(HealthResponse {
        status: "ok",
        timestamp: Utc::now().to_rfc3339(),
    })
}

async fn debug_config(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let mut available_models: Vec<String> = state.config.api_configs.keys().cloned().collect();
    available_models.sort();

    let mut config_summary = Map::new();
    for model_id in &available_models {
        if let Some(config) = state.config.api_configs.get(model_id) {
            config_summary.insert(
                model_id.clone(),
                json!({
                    "name": config.name,
                    "type": config.model_type,
                    "provider": config.provider.as_deref().unwrap_or(&config.model_type),
                    "api_url": config.api_url,
                    "has_api_key": !config.api_key.is_empty(),
                }),
            );
        }
    }

    Json(json!({
        "available_models": available_models,
        "config_summary": config_summary,
    }))
}

async fn list_models(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let mut model_ids: Vec<String> = state.config.api_configs.keys().cloned().collect();
    model_ids.sort();

    let mut models = Vec::with_capacity(model_ids.len());
    for model_id in model_ids {
        if let Some(config) = state.config.api_configs.get(&model_id) {
            models.push(json!({
                "id": model_id,
                "object": "model",
                "created": 1677610602u64,
                "owned_by": config.provider.as_deref().unwrap_or(&config.model_type),
                "name": config.name,
            }));
        }
    }

    Json(json!({
        "object": "list",
        "data": models
    }))
}

async fn chat_completions(
    State(state): State<Arc<AppState>>,
    Json(request_body): Json<Value>,
) -> Result<Response<Body>, AppError> {
    validate_chat_completion_request(&request_body)?;
    handle_proxy(state, request_body).await
}

async fn api_chat_completions(
    State(state): State<Arc<AppState>>,
    Json(request_body): Json<Value>,
) -> Result<Response<Body>, AppError> {
    handle_proxy(state, request_body).await
}

async fn messages(
    State(state): State<Arc<AppState>>,
    Json(request_body): Json<Value>,
) -> Result<Response<Body>, AppError> {
    handle_proxy(state, request_body).await
}

fn validate_chat_completion_request(request_body: &Value) -> Result<(), AppError> {
    if !request_body.is_object() {
        return Err(AppError::InvalidRequest("Invalid request body".to_string()));
    }

    if request_body.get("model").is_none() {
        return Err(AppError::InvalidRequest(
            "Missing required field: 'model'".to_string(),
        ));
    }

    if request_body.get("messages").is_none() {
        return Err(AppError::InvalidRequest(
            "Missing required field: 'messages'".to_string(),
        ));
    }

    Ok(())
}

async fn handle_proxy(
    state: Arc<AppState>,
    request_body: Value,
) -> Result<Response<Body>, AppError> {
    let model = request_body
        .get("model")
        .and_then(Value::as_str)
        .ok_or_else(|| AppError::InvalidRequest("请求缺少 model 字段".to_string()))?
        .to_string();

    let config = state
        .config
        .api_configs
        .get(&model)
        .cloned()
        .ok_or_else(|| {
            let mut models: Vec<_> = state.config.api_configs.keys().cloned().collect();
            models.sort();
            AppError::InvalidRequest(format!(
                "不支持的模型: {model}。支持的模型: {}",
                models.join(", ")
            ))
        })?;

    let stream = request_body
        .get("stream")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    let upstream_body = match config.model_type.as_str() {
        "zhipu" => with_model_override(&request_body, "glm-4.5")?,
        "kimi" => with_model_override(&request_body, "kimi-k2-0905-preview")?,
        "deepseek" => build_deepseek_request(&request_body, &model)?,
        "openai_compat" => {
            let upstream_model = config
                .upstream_model
                .as_deref()
                .unwrap_or(&model)
                .to_string();
            with_model_override(&request_body, &upstream_model)?
        }
        _ => {
            return Err(AppError::Proxy(format!(
                "未知的模型类型: {}",
                config.model_type
            )));
        }
    };

    forward_with_retry(state, config, upstream_body, stream).await
}

fn with_model_override(request_body: &Value, model: &str) -> Result<Value, AppError> {
    let mut body = request_body.clone();
    let body_obj = body
        .as_object_mut()
        .ok_or_else(|| AppError::InvalidRequest("Invalid request body".to_string()))?;
    body_obj.insert("model".to_string(), Value::String(model.to_string()));
    Ok(body)
}

fn build_deepseek_request(request_body: &Value, model: &str) -> Result<Value, AppError> {
    let supported_params = [
        "model",
        "messages",
        "stream",
        "temperature",
        "max_tokens",
        "top_p",
        "frequency_penalty",
        "presence_penalty",
        "stop",
    ];

    let body_obj = request_body
        .as_object()
        .ok_or_else(|| AppError::InvalidRequest("Invalid request body".to_string()))?;

    let mut cleaned = Map::new();
    for key in supported_params {
        if let Some(value) = body_obj.get(key) {
            cleaned.insert(key.to_string(), value.clone());
        }
    }

    cleaned.insert("model".to_string(), Value::String(model.to_string()));
    Ok(Value::Object(cleaned))
}

async fn forward_with_retry(
    state: Arc<AppState>,
    config: ModelConfig,
    request_body: Value,
    stream: bool,
) -> Result<Response<Body>, AppError> {
    let mut last_error: Option<AppError> = None;

    for attempt in 1..=state.config.max_retries {
        info!(
            "🔄 第{attempt}次尝试: {} -> {}",
            config.model_type, config.api_url
        );
        match forward_once(&state, &config, request_body.clone(), stream).await {
            Ok(response) => return Ok(response),
            Err(error) => {
                warn!("❌ 第{attempt}次尝试失败: {error:?}");
                last_error = Some(error);
                if attempt < state.config.max_retries {
                    let delay_ms = state.config.retry_delay_ms * attempt as u64;
                    info!("⏳ {delay_ms}ms 后重试...");
                    sleep(Duration::from_millis(delay_ms)).await;
                }
            }
        }
    }

    Err(last_error.unwrap_or_else(|| AppError::Proxy("请求失败".to_string())))
}

async fn forward_once(
    state: &AppState,
    config: &ModelConfig,
    request_body: Value,
    stream: bool,
) -> Result<Response<Body>, AppError> {
    let endpoint = format!("{}/chat/completions", config.api_url);
    let upstream_response = state
        .client
        .post(&endpoint)
        .header("Authorization", format!("Bearer {}", config.api_key))
        .header("Content-Type", "application/json")
        .json(&request_body)
        .send()
        .await
        .map_err(|error| AppError::Network(format!("网络请求失败: {error}")))?;

    let status = upstream_response.status();
    if !status.is_success() {
        let status_code =
            StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        let upstream_text = upstream_response
            .text()
            .await
            .unwrap_or_else(|_| "无法读取上游错误信息".to_string());
        return Err(AppError::UpstreamApi(status_code, upstream_text));
    }

    upstream_to_downstream_response(upstream_response, stream).await
}

async fn upstream_to_downstream_response(
    upstream_response: reqwest::Response,
    stream: bool,
) -> Result<Response<Body>, AppError> {
    let status = StatusCode::from_u16(upstream_response.status().as_u16())
        .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);

    let content_type = upstream_response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(ToString::to_string);

    let mut builder = Response::builder().status(status);
    if let Some(content_type) = content_type {
        builder = builder.header("content-type", content_type);
    }

    if stream {
        let stream_body = upstream_response
            .bytes_stream()
            .map_err(|error| std::io::Error::other(error.to_string()));
        return builder
            .body(Body::from_stream(stream_body))
            .map_err(|error| AppError::Proxy(format!("构建流式响应失败: {error}")));
    }

    let bytes = upstream_response
        .bytes()
        .await
        .map_err(|error| AppError::Network(format!("读取上游响应失败: {error}")))?;

    builder
        .body(Body::from(bytes))
        .map_err(|error| AppError::Proxy(format!("构建响应失败: {error}")))
}

fn error_response(status: StatusCode, message: String, error_type: &str) -> Response<Body> {
    (
        status,
        Json(json!({
            "detail": {
                "error": {
                    "message": message,
                    "type": error_type,
                }
            }
        })),
    )
        .into_response()
}
