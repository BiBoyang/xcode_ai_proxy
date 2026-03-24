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

    let mut api_configs = HashMap::new();

    let default_model_id =
        env_non_empty("DEFAULT_MODEL_ID").unwrap_or_else(|| "DefaultModel".to_string());
    let default_model_name =
        env_non_empty("DEFAULT_MODEL_NAME").unwrap_or_else(|| "Default Model".to_string());

    let default_api_url = env_non_empty("OPENAI_BASE_URL");
    let default_api_key = env_non_empty("OPENAI_API_KEY");
    let default_models = parse_csv_env("OPENAI_MODEL");

    if default_api_url.is_some() || default_api_key.is_some() || !default_models.is_empty() {
        if default_api_url.is_none() || default_api_key.is_none() || default_models.is_empty() {
            warn!(
                "⚠️ OPENAI_* 配置不完整。需要同时设置 OPENAI_BASE_URL / OPENAI_API_KEY / OPENAI_MODEL"
            );
        } else {
            let normalized_url = normalize_api_url(
                default_api_url
                    .as_deref()
                    .expect("default_api_url should exist after validation"),
            );
            let api_key = default_api_key.expect("default_api_key should exist after validation");

            if default_models.len() == 1 {
                let upstream_model = default_models[0].clone();
                add_model_config(
                    &mut api_configs,
                    &default_model_id,
                    ModelConfig {
                        api_url: normalized_url,
                        api_key,
                        model_type: "openai_compat".to_string(),
                        name: format!("{default_model_name} ({upstream_model})"),
                        upstream_model: Some(upstream_model),
                        provider: Some("openai_compat".to_string()),
                    },
                );
            } else {
                warn!(
                    "⚠️ OPENAI_MODEL 包含多个模型，已按旧兼容模式加载。建议使用 DEFAULT_MODEL_ID + EXTRA_MODEL_IDS。"
                );
                for model in default_models {
                    add_model_config(
                        &mut api_configs,
                        &model,
                        ModelConfig {
                            api_url: normalized_url.clone(),
                            api_key: api_key.clone(),
                            model_type: "openai_compat".to_string(),
                            name: format!("OpenAI-Compatible {model}"),
                            upstream_model: Some(model.clone()),
                            provider: Some("openai_compat".to_string()),
                        },
                    );
                }
            }
        }
    }

    let openai_compat_api_url = env_non_empty("OPENAI_COMPAT_API_URL");
    let openai_compat_api_key = env_non_empty("OPENAI_COMPAT_API_KEY");
    let openai_compat_models = parse_csv_env("OPENAI_COMPAT_MODELS");

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

    load_extra_models(&mut api_configs)?;

    if api_configs.is_empty() {
        return Err("❌ 未配置任何模型。请先执行 xcodeaiproxy setup，或在 .env 中配置 EXTRA_MODEL_IDS 追加更多模型。".to_string());
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

fn is_valid_extra_model_id(model_id: &str) -> bool {
    let mut chars = model_id.chars();
    match chars.next() {
        Some(first) if first.is_ascii_alphabetic() || first == '_' => {}
        _ => return false,
    }
    chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

fn load_extra_models(api_configs: &mut HashMap<String, ModelConfig>) -> Result<(), String> {
    let extra_model_ids = parse_csv_env("EXTRA_MODEL_IDS");
    for model_id in extra_model_ids {
        if !is_valid_extra_model_id(&model_id) {
            return Err(format!(
                "❌ EXTRA_MODEL_IDS 中模型 id 无效: {model_id}。仅支持字母、数字、下划线，且不能以数字开头。"
            ));
        }

        let base_url_key = format!("MODEL_{model_id}_BASE_URL");
        let api_key_key = format!("MODEL_{model_id}_API_KEY");
        let model_key = format!("MODEL_{model_id}_MODEL");
        let name_key = format!("MODEL_{model_id}_NAME");
        let provider_key = format!("MODEL_{model_id}_PROVIDER");

        let base_url = env_non_empty(&base_url_key)
            .ok_or_else(|| format!("❌ 模型 {model_id} 配置不完整，缺少 {base_url_key}"))?;
        let api_key = env_non_empty(&api_key_key)
            .ok_or_else(|| format!("❌ 模型 {model_id} 配置不完整，缺少 {api_key_key}"))?;
        let upstream_model = env_non_empty(&model_key)
            .ok_or_else(|| format!("❌ 模型 {model_id} 配置不完整，缺少 {model_key}"))?;
        let display_name =
            env_non_empty(&name_key).unwrap_or_else(|| format!("OpenAI-Compatible {model_id}"));
        let provider = env_non_empty(&provider_key).unwrap_or_else(|| "openai_compat".to_string());

        add_model_config(
            api_configs,
            &model_id,
            ModelConfig {
                api_url: normalize_api_url(&base_url),
                api_key,
                model_type: "openai_compat".to_string(),
                name: display_name,
                upstream_model: Some(upstream_model),
                provider: Some(provider),
            },
        );
    }

    Ok(())
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

    let upstream_model = config
        .upstream_model
        .as_deref()
        .unwrap_or(&model)
        .to_string();
    let upstream_body = with_model_override(&request_body, &upstream_model)?;

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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;
    use std::sync::{Mutex, OnceLock};

    const CONFIG_ENV_KEYS: &[&str] = &[
        "HOST",
        "PORT",
        "MAX_RETRIES",
        "RETRY_DELAY",
        "REQUEST_TIMEOUT",
        "DEFAULT_MODEL_ID",
        "DEFAULT_MODEL_NAME",
        "OPENAI_COMPAT_API_URL",
        "OPENAI_COMPAT_API_KEY",
        "OPENAI_COMPAT_MODELS",
        "OPENAI_COMPAT_NAME_PREFIX",
        "OPENAI_COMPAT_PROVIDER_NAME",
        "OPENAI_BASE_URL",
        "OPENAI_API_KEY",
        "OPENAI_MODEL",
        "EXTRA_MODEL_IDS",
        "MODEL_ModelA_BASE_URL",
        "MODEL_ModelA_API_KEY",
        "MODEL_ModelA_MODEL",
        "MODEL_ModelA_NAME",
        "MODEL_ModelA_PROVIDER",
        "MODEL_ModelB_BASE_URL",
        "MODEL_ModelB_API_KEY",
        "MODEL_ModelB_MODEL",
        "MODEL_ModelB_NAME",
        "MODEL_ModelB_PROVIDER",
        "UNIT_TEST_ENV",
        "UNIT_TEST_CSV",
    ];

    static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

    fn env_lock() -> &'static Mutex<()> {
        ENV_LOCK.get_or_init(|| Mutex::new(()))
    }

    struct EnvScope {
        snapshot: Vec<(String, Option<String>)>,
    }

    impl EnvScope {
        fn new(keys: &[&str]) -> Self {
            let snapshot = keys
                .iter()
                .map(|key| ((*key).to_string(), env::var(key).ok()))
                .collect::<Vec<_>>();
            for key in keys {
                // SAFETY: Tests are serialized through ENV_LOCK, so mutating process env is synchronized.
                unsafe { env::remove_var(key) };
            }
            Self { snapshot }
        }

        fn set(&self, key: &str, value: &str) {
            // SAFETY: Tests are serialized through ENV_LOCK, so mutating process env is synchronized.
            unsafe { env::set_var(key, value) };
        }

        fn remove(&self, key: &str) {
            // SAFETY: Tests are serialized through ENV_LOCK, so mutating process env is synchronized.
            unsafe { env::remove_var(key) };
        }
    }

    impl Drop for EnvScope {
        fn drop(&mut self) {
            for (key, value) in &self.snapshot {
                match value {
                    Some(v) => {
                        // SAFETY: Tests are serialized through ENV_LOCK, so mutating process env is synchronized.
                        unsafe { env::set_var(key, v) };
                    }
                    None => {
                        // SAFETY: Tests are serialized through ENV_LOCK, so mutating process env is synchronized.
                        unsafe { env::remove_var(key) };
                    }
                }
            }
        }
    }

    #[test]
    fn env_non_empty_trims_and_filters_blank() {
        let _guard = env_lock().lock().expect("env lock poisoned");
        let env_scope = EnvScope::new(&["UNIT_TEST_ENV"]);

        env_scope.set("UNIT_TEST_ENV", "   ");
        assert_eq!(env_non_empty("UNIT_TEST_ENV"), None);

        env_scope.set("UNIT_TEST_ENV", "  value  ");
        assert_eq!(env_non_empty("UNIT_TEST_ENV"), Some("value".to_string()));

        env_scope.remove("UNIT_TEST_ENV");
        assert_eq!(env_non_empty("UNIT_TEST_ENV"), None);
    }

    #[test]
    fn parse_csv_env_trims_and_ignores_empty_items() {
        let _guard = env_lock().lock().expect("env lock poisoned");
        let env_scope = EnvScope::new(&["UNIT_TEST_CSV"]);
        env_scope.set("UNIT_TEST_CSV", " model-a, ,model-b ,, model-c ");

        let parsed = parse_csv_env("UNIT_TEST_CSV");
        assert_eq!(
            parsed,
            vec![
                "model-a".to_string(),
                "model-b".to_string(),
                "model-c".to_string()
            ]
        );
    }

    #[test]
    fn normalize_api_url_handles_trailing_slash_and_chat_suffix() {
        assert_eq!(
            normalize_api_url("https://api.example.com/v1/"),
            "https://api.example.com/v1"
        );
        assert_eq!(
            normalize_api_url("https://api.example.com/v1/chat/completions"),
            "https://api.example.com/v1"
        );
        assert_eq!(
            normalize_api_url("https://api.example.com/v1/chat/completions/"),
            "https://api.example.com/v1"
        );
    }

    #[test]
    fn validate_chat_completion_request_rejects_non_object() {
        let result = validate_chat_completion_request(&json!("bad"));
        match result {
            Err(AppError::InvalidRequest(message)) => assert_eq!(message, "Invalid request body"),
            other => panic!("expected InvalidRequest, got {other:?}"),
        }
    }

    #[test]
    fn validate_chat_completion_request_requires_model_and_messages() {
        let missing_model = validate_chat_completion_request(&json!({
            "messages": [{"role": "user", "content": "hi"}]
        }));
        match missing_model {
            Err(AppError::InvalidRequest(message)) => {
                assert_eq!(message, "Missing required field: 'model'")
            }
            other => panic!("expected InvalidRequest for missing model, got {other:?}"),
        }

        let missing_messages = validate_chat_completion_request(&json!({
            "model": "gpt-4.1-mini"
        }));
        match missing_messages {
            Err(AppError::InvalidRequest(message)) => {
                assert_eq!(message, "Missing required field: 'messages'")
            }
            other => panic!("expected InvalidRequest for missing messages, got {other:?}"),
        }

        let ok = validate_chat_completion_request(&json!({
            "model": "gpt-4.1-mini",
            "messages": [{"role": "user", "content": "hello"}]
        }));
        assert!(ok.is_ok());
    }

    #[test]
    fn with_model_override_replaces_model_and_keeps_other_fields() {
        let original = json!({
            "model": "local-model",
            "stream": true,
            "messages": [{"role": "user", "content": "hello"}]
        });

        let overridden = with_model_override(&original, "upstream-model")
            .expect("model override should succeed");

        assert_eq!(overridden["model"], "upstream-model");
        assert_eq!(overridden["stream"], true);
        assert_eq!(overridden["messages"], original["messages"]);

        // The source body should remain unchanged.
        assert_eq!(original["model"], "local-model");
    }

    #[test]
    fn with_model_override_rejects_non_object_body() {
        let result = with_model_override(&json!("invalid"), "upstream-model");
        match result {
            Err(AppError::InvalidRequest(message)) => assert_eq!(message, "Invalid request body"),
            other => panic!("expected InvalidRequest, got {other:?}"),
        }
    }

    #[test]
    fn add_model_config_keeps_first_on_id_conflict() {
        let mut models = HashMap::new();

        add_model_config(
            &mut models,
            "model-a",
            ModelConfig {
                api_url: "https://first.example.com/v1".to_string(),
                api_key: "first-key".to_string(),
                model_type: "openai_compat".to_string(),
                name: "first".to_string(),
                upstream_model: Some("model-a".to_string()),
                provider: Some("provider-a".to_string()),
            },
        );

        add_model_config(
            &mut models,
            "model-a",
            ModelConfig {
                api_url: "https://second.example.com/v1".to_string(),
                api_key: "second-key".to_string(),
                model_type: "openai_compat".to_string(),
                name: "second".to_string(),
                upstream_model: Some("model-a".to_string()),
                provider: Some("provider-b".to_string()),
            },
        );

        let saved = models.get("model-a").expect("model-a should exist");
        assert_eq!(saved.api_url, "https://first.example.com/v1");
        assert_eq!(saved.api_key, "first-key");
        assert_eq!(saved.name, "first");
    }

    #[test]
    fn load_config_builds_models_from_legacy_openai_env() {
        let _guard = env_lock().lock().expect("env lock poisoned");
        let env_scope = EnvScope::new(CONFIG_ENV_KEYS);

        env_scope.set("OPENAI_BASE_URL", "https://api.example.com/v1/");
        env_scope.set("OPENAI_API_KEY", "sk-legacy-key");
        env_scope.set("OPENAI_MODEL", "model-a, model-b");

        let config = load_config().expect("load_config should succeed");
        assert_eq!(config.host, "0.0.0.0");
        assert_eq!(config.port, 3000);
        assert_eq!(config.max_retries, 3);
        assert_eq!(config.retry_delay_ms, 1000);
        assert_eq!(config.request_timeout_ms, 60_000);
        assert_eq!(config.api_configs.len(), 2);

        let model_a = config
            .api_configs
            .get("model-a")
            .expect("model-a should be configured");
        assert_eq!(model_a.api_url, "https://api.example.com/v1");
        assert_eq!(model_a.api_key, "sk-legacy-key");
        assert_eq!(model_a.provider.as_deref(), Some("openai_compat"));
        assert_eq!(model_a.upstream_model.as_deref(), Some("model-a"));
        assert_eq!(model_a.name, "OpenAI-Compatible model-a");
    }

    #[test]
    fn load_config_uses_default_model_id_for_single_openai_model() {
        let _guard = env_lock().lock().expect("env lock poisoned");
        let env_scope = EnvScope::new(CONFIG_ENV_KEYS);

        env_scope.set("OPENAI_BASE_URL", "https://api.deepseek.com/v1/");
        env_scope.set("OPENAI_API_KEY", "sk-default-key");
        env_scope.set("OPENAI_MODEL", "deepseek-chat");
        env_scope.set("DEFAULT_MODEL_ID", "DefaultModel");
        env_scope.set("DEFAULT_MODEL_NAME", "My Default");

        let config = load_config().expect("load_config should succeed");
        assert_eq!(config.api_configs.len(), 1);
        let model = config
            .api_configs
            .get("DefaultModel")
            .expect("DefaultModel should exist");

        assert_eq!(model.api_url, "https://api.deepseek.com/v1");
        assert_eq!(model.api_key, "sk-default-key");
        assert_eq!(model.upstream_model.as_deref(), Some("deepseek-chat"));
        assert_eq!(model.name, "My Default (deepseek-chat)");
        assert_eq!(model.provider.as_deref(), Some("openai_compat"));
    }

    #[test]
    fn load_config_supports_custom_prefix_and_provider() {
        let _guard = env_lock().lock().expect("env lock poisoned");
        let env_scope = EnvScope::new(CONFIG_ENV_KEYS);

        env_scope.set(
            "OPENAI_COMPAT_API_URL",
            "https://proxy.example.com/v1/chat/completions/",
        );
        env_scope.set("OPENAI_COMPAT_API_KEY", "sk-compat-key");
        env_scope.set("OPENAI_COMPAT_MODELS", "kimi-2.5");
        env_scope.set("OPENAI_COMPAT_NAME_PREFIX", "My Proxy");
        env_scope.set("OPENAI_COMPAT_PROVIDER_NAME", "my_provider");

        let config = load_config().expect("load_config should succeed");
        let model = config
            .api_configs
            .get("kimi-2.5")
            .expect("kimi-2.5 should be configured");

        assert_eq!(model.api_url, "https://proxy.example.com/v1");
        assert_eq!(model.name, "My Proxy kimi-2.5");
        assert_eq!(model.provider.as_deref(), Some("my_provider"));
        assert_eq!(model.upstream_model.as_deref(), Some("kimi-2.5"));
    }

    #[test]
    fn load_config_supports_extra_models_from_env() {
        let _guard = env_lock().lock().expect("env lock poisoned");
        let env_scope = EnvScope::new(CONFIG_ENV_KEYS);

        env_scope.set("OPENAI_BASE_URL", "https://api.deepseek.com/v1");
        env_scope.set("OPENAI_API_KEY", "sk-default-key");
        env_scope.set("OPENAI_MODEL", "deepseek-chat");
        env_scope.set("EXTRA_MODEL_IDS", "ModelA,ModelB");

        env_scope.set("MODEL_ModelA_BASE_URL", "https://api.openai.com/v1/");
        env_scope.set("MODEL_ModelA_API_KEY", "sk-model-a");
        env_scope.set("MODEL_ModelA_MODEL", "gpt-4.1-mini");
        env_scope.set("MODEL_ModelA_NAME", "OpenAI GPT-4.1 mini");

        env_scope.set(
            "MODEL_ModelB_BASE_URL",
            "https://api.moonshot.cn/v1/chat/completions",
        );
        env_scope.set("MODEL_ModelB_API_KEY", "sk-model-b");
        env_scope.set("MODEL_ModelB_MODEL", "kimi-k2-0711-preview");
        env_scope.set("MODEL_ModelB_PROVIDER", "moonshot");

        let config = load_config().expect("load_config should succeed");
        assert_eq!(config.api_configs.len(), 3);

        let model_a = config
            .api_configs
            .get("ModelA")
            .expect("ModelA should exist");
        assert_eq!(model_a.api_url, "https://api.openai.com/v1");
        assert_eq!(model_a.upstream_model.as_deref(), Some("gpt-4.1-mini"));
        assert_eq!(model_a.name, "OpenAI GPT-4.1 mini");
        assert_eq!(model_a.provider.as_deref(), Some("openai_compat"));

        let model_b = config
            .api_configs
            .get("ModelB")
            .expect("ModelB should exist");
        assert_eq!(model_b.api_url, "https://api.moonshot.cn/v1");
        assert_eq!(
            model_b.upstream_model.as_deref(),
            Some("kimi-k2-0711-preview")
        );
        assert_eq!(model_b.name, "OpenAI-Compatible ModelB");
        assert_eq!(model_b.provider.as_deref(), Some("moonshot"));
    }

    #[test]
    fn load_config_errors_on_invalid_extra_model_id() {
        let _guard = env_lock().lock().expect("env lock poisoned");
        let env_scope = EnvScope::new(CONFIG_ENV_KEYS);

        env_scope.set("OPENAI_BASE_URL", "https://api.deepseek.com/v1");
        env_scope.set("OPENAI_API_KEY", "sk-default-key");
        env_scope.set("OPENAI_MODEL", "deepseek-chat");
        env_scope.set("EXTRA_MODEL_IDS", "bad-id");

        let error = load_config().expect_err("load_config should fail for invalid model id");
        assert!(
            error.contains("EXTRA_MODEL_IDS 中模型 id 无效: bad-id"),
            "unexpected error message: {error}"
        );
    }

    #[test]
    fn load_config_errors_on_incomplete_extra_model_config() {
        let _guard = env_lock().lock().expect("env lock poisoned");
        let env_scope = EnvScope::new(CONFIG_ENV_KEYS);

        env_scope.set("OPENAI_BASE_URL", "https://api.deepseek.com/v1");
        env_scope.set("OPENAI_API_KEY", "sk-default-key");
        env_scope.set("OPENAI_MODEL", "deepseek-chat");
        env_scope.set("EXTRA_MODEL_IDS", "ModelA");
        env_scope.set("MODEL_ModelA_BASE_URL", "https://api.openai.com/v1");
        env_scope.set("MODEL_ModelA_API_KEY", "sk-model-a");
        // MODEL_ModelA_MODEL intentionally missing.

        let error =
            load_config().expect_err("load_config should fail for incomplete extra model config");
        assert!(
            error.contains("模型 ModelA 配置不完整，缺少 MODEL_ModelA_MODEL"),
            "unexpected error message: {error}"
        );
    }

    #[test]
    fn load_config_errors_when_missing_required_model_config() {
        let _guard = env_lock().lock().expect("env lock poisoned");
        let _env_scope = EnvScope::new(CONFIG_ENV_KEYS);

        let error = load_config().expect_err("load_config should fail without model config");
        assert!(
            error.contains("未配置任何模型"),
            "unexpected error message: {error}"
        );
    }

    #[test]
    fn load_config_errors_on_invalid_port() {
        let _guard = env_lock().lock().expect("env lock poisoned");
        let env_scope = EnvScope::new(CONFIG_ENV_KEYS);
        env_scope.set("OPENAI_BASE_URL", "https://api.example.com/v1");
        env_scope.set("OPENAI_API_KEY", "sk-legacy-key");
        env_scope.set("OPENAI_MODEL", "model-a");
        env_scope.set("PORT", "not-a-port");

        let error = load_config().expect_err("load_config should fail for invalid port");
        assert!(
            error.contains("环境变量 PORT 不是有效端口"),
            "unexpected error message: {error}"
        );
    }

    #[tokio::test]
    async fn error_response_has_expected_shape() {
        let response = error_response(
            StatusCode::BAD_REQUEST,
            "bad request".to_string(),
            "invalid_request_error",
        );
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body should be readable");
        let value: Value = serde_json::from_slice(&body).expect("body should be valid json");

        assert_eq!(value["detail"]["error"]["message"], "bad request");
        assert_eq!(value["detail"]["error"]["type"], "invalid_request_error");
    }

    #[tokio::test]
    async fn app_error_upstream_api_maps_status_and_type() {
        let response =
            AppError::UpstreamApi(StatusCode::BAD_GATEWAY, "gateway failure".to_string())
                .into_response();
        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);

        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body should be readable");
        let value: Value = serde_json::from_slice(&body).expect("body should be valid json");

        assert_eq!(value["detail"]["error"]["type"], "api_error");
        let message = value["detail"]["error"]["message"]
            .as_str()
            .expect("message should be string");
        assert!(
            message.contains("API 请求失败: 502 - gateway failure"),
            "unexpected message: {message}"
        );
    }
}
