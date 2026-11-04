use kb_core::agent_tools::{execute_tool, tool_definitions, TaskScope, ToolCall, ToolResult};
use kb_core::{db, KnowledgeBase};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::collections::HashSet;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

const KEYRING_SERVICE: &str = "com.kbox.desktop";
const KEYRING_ACCOUNT: &str = "model-api-key";
pub const CANCELLED_ERROR: &str = "AI_TASK_CANCELLED";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiConfig {
    #[serde(default = "default_provider")]
    pub provider: String,
    #[serde(default = "default_base_url")]
    pub base_url: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub embedding_model: String,
    #[serde(default = "default_auth_mode")]
    pub auth_mode: String,
    #[serde(default = "default_api_key_header")]
    pub api_key_header: String,
    #[serde(default = "default_api_key_prefix")]
    pub api_key_prefix: String,
    #[serde(default)]
    pub headers: HashMap<String, String>,
    #[serde(default = "default_tool_rounds")]
    pub max_tool_rounds: u32,
    /// 只记录“用户已保存过 Key”，不包含密钥正文。
    /// Option 用于兼容旧版配置；None 时根据已完成的 Provider 配置迁移。
    #[serde(default)]
    pub api_key_configured: Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AiConfigView {
    pub provider: String,
    pub base_url: String,
    pub model: String,
    pub embedding_model: String,
    pub auth_mode: String,
    pub api_key_header: String,
    pub api_key_prefix: String,
    pub headers: HashMap<String, String>,
    pub max_tool_rounds: u32,
    pub api_key_configured: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationTurn {
    pub role: String,
    pub content: String,
}

fn recent_conversation(turns: &[ConversationTurn]) -> Vec<ConversationTurn> {
    let mut selected = turns.iter().rev().take(12).cloned().collect::<Vec<_>>();
    selected.reverse();
    let mut remaining = 16_000_usize;
    selected
        .into_iter()
        .filter_map(|turn| {
            if remaining == 0 || !matches!(turn.role.as_str(), "user" | "assistant") {
                return None;
            }
            let content = turn
                .content
                .trim()
                .chars()
                .take(remaining.min(4_000))
                .collect::<String>();
            remaining = remaining.saturating_sub(content.chars().count());
            if content.is_empty() {
                None
            } else {
                Some(ConversationTurn {
                    role: turn.role,
                    content,
                })
            }
        })
        .collect()
}

fn conversation_context_text(turns: &[ConversationTurn]) -> String {
    recent_conversation(turns)
        .into_iter()
        .map(|turn| {
            let speaker = if turn.role == "user" {
                "用户"
            } else {
                "助手"
            };
            format!("{speaker}：{}", turn.content)
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// OpenAI-compatible 的 tool_call 结构。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderToolCallFunction {
    pub name: String,
    /// 参数为 JSON 字符串
    pub arguments: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderToolCall {
    pub id: String,
    pub function: ProviderToolCallFunction,
}

/// 结构化响应：内容、工具调用与结束原因。
#[derive(Debug, Clone, Serialize)]
pub struct ProviderResponse {
    pub content: String,
    pub tool_calls: Vec<ProviderToolCall>,
    pub finish_reason: String,
}

/// 从 Chat Completions 响应 JSON 中解析 content / tool_calls / finish_reason。
/// 抽为纯函数，便于用本地模拟响应做单元测试（不依赖真实模型 / API Key）。
fn parse_chat_response(value: &Value) -> ProviderResponse {
    let message = value.pointer("/choices/0/message");
    let content = message
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_str())
        .unwrap_or("")
        .to_string();
    let tool_calls = message
        .and_then(|m| m.get("tool_calls").cloned())
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();
    let finish_reason = value
        .pointer("/choices/0/finish_reason")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    ProviderResponse {
        content,
        tool_calls,
        finish_reason,
    }
}

#[derive(Debug, Deserialize)]
struct OrganizeDirectory {
    path: String,
    #[serde(default)]
    description: String,
}

#[derive(Debug, Deserialize)]
struct OrganizeAssignment {
    object_id: String,
    path: String,
}

#[derive(Debug, Deserialize)]
struct OrganizePlan {
    #[serde(default)]
    directories: Vec<OrganizeDirectory>,
    #[serde(default)]
    assignments: Vec<OrganizeAssignment>,
    #[serde(default)]
    archive_object_ids: Vec<String>,
    #[serde(default)]
    summary: String,
}

#[derive(Default)]
struct StreamToolCall {
    id: String,
    name: String,
    arguments: String,
}

#[derive(Default)]
struct StreamAccumulator {
    content: String,
    tool_calls: BTreeMap<usize, StreamToolCall>,
    finish_reason: String,
}

fn apply_stream_event<F>(value: &Value, accumulator: &mut StreamAccumulator, on_delta: &mut F)
where
    F: FnMut(String),
{
    let Some(choice) = value.pointer("/choices/0") else {
        return;
    };
    if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
        accumulator.finish_reason = reason.to_string();
    }
    let Some(delta) = choice.get("delta") else {
        return;
    };
    if let Some(content) = delta.get("content").and_then(Value::as_str) {
        accumulator.content.push_str(content);
        on_delta(content.to_string());
    }
    if let Some(calls) = delta.get("tool_calls").and_then(Value::as_array) {
        for call in calls {
            let index = call.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
            let target = accumulator.tool_calls.entry(index).or_default();
            if let Some(id) = call.get("id").and_then(Value::as_str) {
                target.id.push_str(id);
            }
            if let Some(name) = call.pointer("/function/name").and_then(Value::as_str) {
                target.name.push_str(name);
            }
            if let Some(arguments) = call.pointer("/function/arguments").and_then(Value::as_str) {
                target.arguments.push_str(arguments);
            }
        }
    }
}

fn finish_stream(accumulator: StreamAccumulator) -> ProviderResponse {
    ProviderResponse {
        content: accumulator.content,
        tool_calls: accumulator
            .tool_calls
            .into_values()
            .map(|call| ProviderToolCall {
                id: call.id,
                function: ProviderToolCallFunction {
                    name: call.name,
                    arguments: call.arguments,
                },
            })
            .collect(),
        finish_reason: accumulator.finish_reason,
    }
}

fn apply_sse_line<F>(line: &str, accumulator: &mut StreamAccumulator, on_delta: &mut F) -> bool
where
    F: FnMut(String),
{
    let Some(data) = line.trim().strip_prefix("data:").map(str::trim) else {
        return false;
    };
    if data.is_empty() || data == "[DONE]" {
        return true;
    }
    let Ok(value) = serde_json::from_str::<Value>(data) else {
        return false;
    };
    apply_stream_event(&value, accumulator, on_delta);
    true
}

#[derive(Debug, Clone, Serialize)]
pub struct OrganizeResult {
    pub task_id: String,
    pub summary: String,
    pub actions: Vec<String>,
    pub moved_objects: usize,
    pub created_directories: usize,
    pub archived_objects: usize,
    pub removed_directories: usize,
}

fn ensure_not_cancelled(cancelled: &AtomicBool) -> Result<(), String> {
    if cancelled.load(Ordering::Relaxed) {
        Err(CANCELLED_ERROR.to_string())
    } else {
        Ok(())
    }
}

async fn wait_for_cancellation(cancelled: Arc<AtomicBool>) {
    loop {
        if cancelled.load(Ordering::Relaxed) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

fn collection_scope(
    nodes: &[kb_core::model::CollectionNode],
    collection_id: &str,
) -> Option<(String, HashSet<String>, kb_core::model::CollectionNode)> {
    for node in nodes {
        if node.id == collection_id {
            let mut ids = HashSet::new();
            fn collect(node: &kb_core::model::CollectionNode, ids: &mut HashSet<String>) {
                ids.extend(node.objects.iter().map(|object| object.id.clone()));
                for child in &node.children {
                    collect(child, ids);
                }
            }
            collect(node, &mut ids);
            return Some((node.path.clone(), ids, node.clone()));
        }
        if let Some(found) = collection_scope(&node.children, collection_id) {
            return Some(found);
        }
    }
    None
}

fn resolved_plan_path(scope_path: Option<&str>, requested: &str) -> Option<String> {
    let _ = scope_path;
    let path = requested.trim().trim_matches('/');
    if path.is_empty() || path.split('/').count() > 4 {
        return None;
    }
    Some(path.to_string())
}

impl Default for AiConfig {
    fn default() -> Self {
        Self {
            provider: default_provider(),
            base_url: default_base_url(),
            model: String::new(),
            embedding_model: String::new(),
            auth_mode: default_auth_mode(),
            api_key_header: default_api_key_header(),
            api_key_prefix: default_api_key_prefix(),
            headers: HashMap::new(),
            max_tool_rounds: default_tool_rounds(),
            api_key_configured: None,
        }
    }
}

fn default_provider() -> String {
    "openai-compatible".to_string()
}

fn default_base_url() -> String {
    "https://api.openai.com/v1".to_string()
}

fn default_auth_mode() -> String {
    "bearer".to_string()
}

fn default_api_key_header() -> String {
    "Authorization".to_string()
}

fn default_api_key_prefix() -> String {
    "Bearer ".to_string()
}

fn default_tool_rounds() -> u32 {
    12
}

fn config_path(vault_path: &Path) -> std::path::PathBuf {
    vault_path.join(".kb").join("ai-config.json")
}

fn keyring_entry() -> Result<keyring::Entry, String> {
    keyring::Entry::new(KEYRING_SERVICE, KEYRING_ACCOUNT).map_err(|e| e.to_string())
}

/// 钥匙串不可用（如未签名的 debug 构建被 macOS 拒绝）时的降级密钥文件。
fn api_key_file_path(vault_path: &Path) -> std::path::PathBuf {
    vault_path.join(".kb").join("api-key")
}

fn write_api_key_file(vault_path: &Path, api_key: &str) -> Result<(), String> {
    let path = api_key_file_path(vault_path);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let temp = path.with_extension("key.tmp");
    std::fs::write(&temp, api_key.as_bytes()).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&temp, std::fs::Permissions::from_mode(0o600));
    }
    std::fs::rename(&temp, &path).map_err(|e| e.to_string())
}

fn read_api_key_file(vault_path: &Path) -> Option<String> {
    std::fs::read_to_string(api_key_file_path(vault_path))
        .ok()
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
}

fn remove_api_key_file(vault_path: &Path) {
    let _ = std::fs::remove_file(api_key_file_path(vault_path));
}

fn api_key_cache() -> &'static Mutex<Option<String>> {
    static CACHE: OnceLock<Mutex<Option<String>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(None))
}

fn infer_api_key_configured(config: &AiConfig) -> bool {
    config.auth_mode != "none"
        && !config.base_url.trim().is_empty()
        && !config.model.trim().is_empty()
}

fn write_config(vault_path: &Path, config: &AiConfig) -> Result<(), String> {
    let path = config_path(vault_path);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let temp = path.with_extension("json.tmp");
    let data = serde_json::to_vec_pretty(config).map_err(|e| e.to_string())?;
    std::fs::write(&temp, data).map_err(|e| e.to_string())?;
    std::fs::rename(&temp, &path).map_err(|e| e.to_string())
}

pub fn load_config(vault_path: &Path) -> AiConfig {
    std::fs::read_to_string(config_path(vault_path))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

pub fn get_config_view(vault_path: &Path) -> AiConfigView {
    let config = load_config(vault_path);
    // 配置页和 AI 抽屉会频繁读取此状态。这里绝对不读取
    // 钥匙串正文，否则 macOS 会在普通界面检查时反复弹出授权框。
    let api_key_configured = config
        .api_key_configured
        .unwrap_or_else(|| infer_api_key_configured(&config));
    AiConfigView {
        provider: config.provider,
        base_url: config.base_url,
        model: config.model,
        embedding_model: config.embedding_model,
        auth_mode: config.auth_mode,
        api_key_header: config.api_key_header,
        api_key_prefix: config.api_key_prefix,
        headers: config.headers,
        max_tool_rounds: config.max_tool_rounds,
        api_key_configured,
    }
}

pub fn save_config(
    vault_path: &Path,
    mut config: AiConfig,
    api_key: Option<String>,
) -> Result<(), String> {
    if config.base_url.trim().is_empty() {
        return Err("Base URL 不能为空".to_string());
    }
    if config.model.trim().is_empty() {
        return Err("模型名称不能为空".to_string());
    }
    if !matches!(
        config.auth_mode.as_str(),
        "bearer" | "x-api-key" | "custom" | "none"
    ) {
        return Err("不支持的 API 认证方式".to_string());
    }
    if config.auth_mode == "custom" && config.api_key_header.trim().is_empty() {
        return Err("自定义认证 Header 不能为空".to_string());
    }
    let existing = load_config(vault_path);
    let new_api_key = api_key
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if let Some(api_key) = new_api_key {
        // debug 构建（未签名，钥匙串必被系统拒绝）或此前已降级时，直接写本地文件，
        // 避免反复弹出钥匙串授权框；正式签名版仍优先使用钥匙串。
        let use_file = cfg!(debug_assertions) || read_api_key_file(vault_path).is_some();
        let mut stored_in_keyring = false;
        if !use_file {
            match keyring_entry().and_then(|entry| {
                entry
                    .set_password(api_key)
                    .map_err(|e| format!("API Key 保存失败: {e}"))
            }) {
                Ok(()) => {
                    stored_in_keyring = true;
                    remove_api_key_file(vault_path);
                }
                Err(error) => {
                    // 钥匙串不可用（未签名构建 / 用户拒绝授权）时降级到本地文件
                    eprintln!("钥匙串写入失败，改用本地文件存储 API Key: {error}");
                }
            }
        }
        if !stored_in_keyring {
            write_api_key_file(vault_path, api_key)?;
        }
        *api_key_cache()
            .lock()
            .map_err(|_| "API Key 缓存不可用".to_string())? = Some(api_key.to_string());
        config.api_key_configured = Some(true);
    } else {
        config.api_key_configured = Some(
            existing
                .api_key_configured
                .unwrap_or_else(|| infer_api_key_configured(&existing)),
        );
    }
    write_config(vault_path, &config)
}

fn chat_completions_url(base_url: &str) -> String {
    let base = base_url.trim().trim_end_matches('/');
    if base.ends_with("/chat/completions") {
        base.to_string()
    } else {
        format!("{base}/chat/completions")
    }
}

fn embeddings_url(base_url: &str) -> String {
    let base = base_url.trim().trim_end_matches('/');
    if base.ends_with("/embeddings") {
        base.to_string()
    } else {
        format!("{base}/embeddings")
    }
}

fn parse_embeddings(value: &Value, expected: usize) -> Result<Vec<Vec<f32>>, String> {
    let data = value["data"]
        .as_array()
        .ok_or_else(|| "Embedding 响应缺少 data 数组".to_string())?;
    let mut vectors = vec![None; expected];
    for (position, item) in data.iter().enumerate() {
        let index = item["index"].as_u64().unwrap_or(position as u64) as usize;
        if index >= expected {
            continue;
        }
        let vector = item["embedding"]
            .as_array()
            .ok_or_else(|| "Embedding 响应缺少向量".to_string())?
            .iter()
            .map(|number| {
                number
                    .as_f64()
                    .map(|value| value as f32)
                    .ok_or_else(|| "Embedding 向量包含非数字值".to_string())
            })
            .collect::<Result<Vec<_>, _>>()?;
        vectors[index] = Some(vector);
    }
    vectors
        .into_iter()
        .map(|vector| vector.ok_or_else(|| "Embedding 响应数量不完整".to_string()))
        .collect()
}

async fn embed_texts(vault_path: &Path, texts: &[String]) -> Result<Vec<Vec<f32>>, String> {
    let config = load_config(vault_path);
    if config.embedding_model.trim().is_empty() {
        return Err("尚未配置 Embedding 模型".to_string());
    }
    let api_key = api_key_for_config(vault_path, &config)?;
    let headers = request_headers(&config, &api_key)?;
    let client = reqwest::Client::builder()
        .default_headers(headers)
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .map_err(|error| error.to_string())?;
    let response = client
        .post(embeddings_url(&config.base_url))
        .json(&json!({
            "model": config.embedding_model,
            "input": texts,
            "encoding_format": "float"
        }))
        .send()
        .await
        .map_err(|error| format!("Embedding 请求失败: {error}"))?;
    let status = response.status();
    let value = response
        .json::<Value>()
        .await
        .map_err(|error| error.to_string())?;
    if !status.is_success() {
        let detail: String = value.to_string().chars().take(800).collect();
        return Err(format!("Embedding 接口返回 {status}: {detail}"));
    }
    parse_embeddings(&value, texts.len())
}

fn cosine_similarity(left: &[f32], right: &[f32]) -> f64 {
    if left.len() != right.len() || left.is_empty() {
        return 0.0;
    }
    let (mut dot, mut left_norm, mut right_norm) = (0.0_f64, 0.0_f64, 0.0_f64);
    for (left, right) in left.iter().zip(right) {
        let left = *left as f64;
        let right = *right as f64;
        dot += left * right;
        left_norm += left * left;
        right_norm += right * right;
    }
    if left_norm == 0.0 || right_norm == 0.0 {
        0.0
    } else {
        dot / (left_norm.sqrt() * right_norm.sqrt())
    }
}

fn embedding_text(title: &str, content: &str) -> String {
    let content = content.chars().take(12_000).collect::<String>();
    format!("标题：{title}\n\n{content}")
}

pub async fn hybrid_search(
    vault_path: &Path,
    query: &str,
    limit: i64,
) -> Result<Vec<kb_core::ObjectSummary>, String> {
    let kb = KnowledgeBase::new(vault_path);
    let requested = limit.clamp(1, 100) as usize;
    let lexical = kb
        .search(query, (requested * 3) as i64)
        .map_err(|error| error.to_string())?;
    let config = load_config(vault_path);
    if config.embedding_model.trim().is_empty() || query.trim().is_empty() {
        return Ok(lexical.into_iter().take(requested).collect());
    }
    let query_vector = match embed_texts(vault_path, &[query.to_string()]).await {
        Ok(mut vectors) => vectors.remove(0),
        Err(_) => return Ok(lexical.into_iter().take(requested).collect()),
    };
    let objects = {
        let conn = kb.conn().map_err(|error| error.to_string())?;
        db::list_objects(&conn).map_err(|error| error.to_string())?
    };
    let mut cached = HashMap::<String, Vec<f32>>::new();
    let mut missing = Vec::<(String, String, String)>::new();
    for object in &objects {
        let hash = format!(
            "{}:{}",
            object.file_hash.as_deref().unwrap_or(""),
            object.updated_at
        );
        let existing = {
            let conn = kb.conn().map_err(|error| error.to_string())?;
            db::get_object_embedding(&conn, &object.id, &config.embedding_model, &hash)
                .map_err(|error| error.to_string())?
        };
        if let Some(vector) = existing {
            cached.insert(object.id.clone(), vector);
        } else {
            let content = kb.read_object_content(&object.id).unwrap_or_default();
            missing.push((
                object.id.clone(),
                hash,
                embedding_text(&object.title, &content),
            ));
        }
    }
    for batch in missing.chunks(32) {
        let texts = batch.iter().map(|item| item.2.clone()).collect::<Vec<_>>();
        let vectors = match embed_texts(vault_path, &texts).await {
            Ok(vectors) => vectors,
            Err(_) => return Ok(lexical.into_iter().take(requested).collect()),
        };
        let conn = kb.conn().map_err(|error| error.to_string())?;
        for ((id, hash, _), vector) in batch.iter().zip(vectors) {
            db::upsert_object_embedding(&conn, id, &config.embedding_model, hash, &vector)
                .map_err(|error| error.to_string())?;
            cached.insert(id.clone(), vector);
        }
    }

    let mut semantic = cached
        .iter()
        .map(|(id, vector)| (id.clone(), cosine_similarity(&query_vector, vector)))
        .collect::<Vec<_>>();
    semantic.sort_by(|left, right| right.1.total_cmp(&left.1));

    let mut scores = HashMap::<String, f64>::new();
    for (rank, result) in lexical.iter().enumerate() {
        *scores.entry(result.id.clone()).or_default() += 0.65 / (60.0 + rank as f64);
    }
    for (rank, (id, similarity)) in semantic.iter().enumerate() {
        if *similarity > 0.0 {
            *scores.entry(id.clone()).or_default() += 0.35 / (60.0 + rank as f64);
        }
    }
    let lexical_map = lexical
        .into_iter()
        .map(|result| (result.id.clone(), result))
        .collect::<HashMap<_, _>>();
    let object_map = objects
        .into_iter()
        .map(|object| (object.id.clone(), object))
        .collect::<HashMap<_, _>>();
    let mut ranked = scores
        .into_iter()
        .filter_map(|(id, score)| {
            if let Some(mut result) = lexical_map.get(&id).cloned() {
                result.score = score * 1000.0;
                return Some(result);
            }
            let object = object_map.get(&id)?;
            let content = kb.read_object_content(&id).unwrap_or_default();
            Some(kb_core::ObjectSummary {
                id,
                title: object.title.clone(),
                r#type: object.r#type.clone(),
                format: object.format.clone(),
                score: score * 1000.0,
                snippet: content.chars().take(220).collect(),
                created_by: object.created_by.clone(),
            })
        })
        .collect::<Vec<_>>();
    ranked.sort_by(|left, right| right.score.total_cmp(&left.score));
    ranked.truncate(requested);
    Ok(ranked)
}

fn request_headers(config: &AiConfig, api_key: &str) -> Result<HeaderMap, String> {
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    // 先应用自定义 Header，再写入 API Key，避免旧配置中的 Authorization
    // 覆盖用户在设置页填写的 API Key。
    for (name, value) in &config.headers {
        let name = HeaderName::from_bytes(name.as_bytes())
            .map_err(|e| format!("Header 名称无效（{name}）: {e}"))?;
        let value = HeaderValue::from_str(value).map_err(|e| format!("Header 值无效: {e}"))?;
        headers.insert(name, value);
    }
    if config.auth_mode != "none" && !api_key.trim().is_empty() {
        let (name, prefix) = match config.auth_mode.as_str() {
            "bearer" => (AUTHORIZATION, "Bearer "),
            "x-api-key" => (HeaderName::from_static("x-api-key"), ""),
            "custom" => (
                HeaderName::from_bytes(config.api_key_header.trim().as_bytes())
                    .map_err(|e| format!("认证 Header 名称无效: {e}"))?,
                config.api_key_prefix.as_str(),
            ),
            _ => return Err("不支持的 API 认证方式".to_string()),
        };
        let value = HeaderValue::from_str(&format!("{prefix}{}", api_key.trim()))
            .map_err(|e| format!("API Key 格式无效: {e}"))?;
        headers.insert(name, value);
    }
    Ok(headers)
}

fn saved_api_key(vault_path: &Path) -> Result<String, String> {
    let mut cache = api_key_cache()
        .lock()
        .map_err(|_| "API Key 缓存不可用".to_string())?;
    if let Some(api_key) = cache.as_ref() {
        return Ok(api_key.clone());
    }
    // 优先读取降级文件：一旦走了文件方案就不再触碰钥匙串，避免反复弹授权框
    if let Some(api_key) = read_api_key_file(vault_path) {
        *cache = Some(api_key.clone());
        return Ok(api_key);
    }
    // debug 构建未签名，钥匙串不可用，直接判定为未配置
    if cfg!(debug_assertions) {
        return Err("尚未配置 API Key".to_string());
    }
    let api_key = keyring_entry()?
        .get_password()
        .map_err(|_| "尚未配置 API Key".to_string())?;
    *cache = Some(api_key.clone());
    Ok(api_key)
}

fn api_key_for_config(vault_path: &Path, config: &AiConfig) -> Result<String, String> {
    if config.auth_mode == "none" {
        Ok(String::new())
    } else {
        saved_api_key(vault_path)
    }
}

async fn chat_request(
    vault_path: &Path,
    messages: serde_json::Value,
    max_tokens: u32,
    tools: Option<&Value>,
) -> Result<ProviderResponse, String> {
    let config = load_config(vault_path);
    if config.model.trim().is_empty() {
        return Err("请先在设置中配置模型".to_string());
    }
    let api_key = api_key_for_config(vault_path, &config)?;
    let headers = request_headers(&config, &api_key)?;
    let client = reqwest::Client::builder()
        .default_headers(headers)
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .map_err(|e| e.to_string())?;
    let mut body = json!({
        "model": config.model,
        "messages": messages,
        "temperature": 0.2,
        "max_tokens": max_tokens
    });
    if let Some(tools) = tools {
        body["tools"] = tools.clone();
        body["tool_choice"] = json!("auto");
    }
    let response = client
        .post(chat_completions_url(&config.base_url))
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("模型请求失败: {e}"))?;
    let status = response.status();
    let body = response.text().await.map_err(|e| e.to_string())?;
    if !status.is_success() {
        let detail: String = body.chars().take(800).collect();
        return Err(format!("模型接口返回 {status}: {detail}"));
    }
    let value: serde_json::Value =
        serde_json::from_str(&body).map_err(|e| format!("模型响应不是有效 JSON: {e}"))?;
    Ok(parse_chat_response(&value))
}

async fn chat_request_streaming<F>(
    vault_path: &Path,
    messages: serde_json::Value,
    max_tokens: u32,
    tools: Option<&Value>,
    mut on_delta: F,
) -> Result<ProviderResponse, String>
where
    F: FnMut(String),
{
    let config = load_config(vault_path);
    if config.model.trim().is_empty() {
        return Err("请先在设置中配置模型".to_string());
    }
    let api_key = api_key_for_config(vault_path, &config)?;
    let headers = request_headers(&config, &api_key)?;
    let client = reqwest::Client::builder()
        .default_headers(headers)
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .map_err(|e| e.to_string())?;
    let mut body = json!({
        "model": config.model,
        "messages": messages,
        "temperature": 0.2,
        "max_tokens": max_tokens,
        "stream": true
    });
    if let Some(tools) = tools {
        body["tools"] = tools.clone();
        body["tool_choice"] = json!("auto");
    }
    let mut response = client
        .post(chat_completions_url(&config.base_url))
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("模型请求失败: {e}"))?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.map_err(|e| e.to_string())?;
        let detail: String = body.chars().take(800).collect();
        return Err(format!("模型接口返回 {status}: {detail}"));
    }

    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();
    if content_type.contains("application/json") {
        let value = response.json::<Value>().await.map_err(|e| e.to_string())?;
        let parsed = parse_chat_response(&value);
        if !parsed.content.is_empty() {
            on_delta(parsed.content.clone());
        }
        return Ok(parsed);
    }

    let mut pending = Vec::<u8>::new();
    let mut accumulator = StreamAccumulator::default();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| format!("读取模型流失败: {e}"))?
    {
        pending.extend_from_slice(&chunk);
        while let Some(position) = pending.iter().position(|byte| *byte == b'\n') {
            let line = pending.drain(..=position).collect::<Vec<_>>();
            let line = String::from_utf8_lossy(&line);
            apply_sse_line(&line, &mut accumulator, &mut on_delta);
        }
    }
    if !pending.is_empty() {
        let body = String::from_utf8_lossy(&pending);
        let handled = apply_sse_line(&body, &mut accumulator, &mut on_delta);
        if !handled && accumulator.content.is_empty() && accumulator.tool_calls.is_empty() {
            if let Ok(value) = serde_json::from_str::<Value>(body.trim()) {
                let parsed = parse_chat_response(&value);
                if !parsed.content.is_empty() {
                    on_delta(parsed.content.clone());
                }
                return Ok(parsed);
            }
        }
    }
    Ok(finish_stream(accumulator))
}

async fn chat(
    vault_path: &Path,
    messages: serde_json::Value,
    max_tokens: u32,
) -> Result<ProviderResponse, String> {
    chat_request(vault_path, messages, max_tokens, None).await
}

pub async fn test_connection(vault_path: &Path) -> Result<String, String> {
    chat(
        vault_path,
        json!([
            {"role": "system", "content": "You are a connection test. Reply with OK."},
            {"role": "user", "content": "Reply only with OK"}
        ]),
        16,
    )
    .await
    .map(|r| r.content)
}

pub async fn edit_text(
    vault_path: &Path,
    instruction: &str,
    text: &str,
    document_context: Option<&str>,
) -> Result<String, String> {
    if instruction.trim().is_empty() {
        return Err("请输入 AI 指令".to_string());
    }
    if text.trim().is_empty() {
        return Err("没有可处理的文本".to_string());
    }
    let context = document_context.unwrap_or("");
    let user = format!(
        "指令：\n{}\n\n需要处理的 Markdown：\n<target>\n{}\n</target>\n\n当前文档上下文（仅供参考）：\n<context>\n{}\n</context>",
        instruction.trim(), text, context
    );
    chat(
        vault_path,
        json!([
            {
                "role": "system",
                "content": "你是 KBOX 的 Markdown 编辑助手。严格执行用户指令，保留事实与 Markdown 结构。只返回用于替换目标区域的最终 Markdown，不要解释，不要使用代码围栏包裹结果。"
            },
            {"role": "user", "content": user}
        ]),
        4096,
    )
    .await
    .map(|r| r.content)
}

/// 为非文本格式对象生成概念笔记（影子笔记）。
/// 输入：对象的原始提取文本（截断到 max_input_chars）+ 元信息。
/// 输出：AI 生成的概念摘要 Markdown 文本。
pub async fn generate_concept_note(
    vault_path: &Path,
    object_title: &str,
    object_format: &str,
    extracted_text: &str,
    max_input_chars: usize,
) -> Result<String, String> {
    if extracted_text.trim().is_empty() {
        return Err("对象没有可提取的文本内容".to_string());
    }

    // 截断输入，防止超长文本超出 token 限制
    let truncated = if extracted_text.len() > max_input_chars {
        let mut end = max_input_chars;
        // 尽量在 UTF-8 字符边界截断
        while end > 0 && !extracted_text.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}…（已截断，原文共 {} 字符）", &extracted_text[..end], extracted_text.len())
    } else {
        extracted_text.to_string()
    };

    let user_prompt = format!(
        "文件标题：{}\n文件格式：{}\n\n提取的原始文本：\n<content>\n{}\n</content>",
        object_title, object_format, truncated
    );

    chat(
        vault_path,
        json!([
            {
                "role": "system",
                "content": "你是 KBOX 的知识摘要助手。请根据给定的文件提取文本，生成一份简洁的概念笔记（影子笔记）。要求：\n\
1. 用 2-3 段话概括文件的核心内容、关键数据或论点\n\
2. 列出该文件涉及的主要主题（用逗号分隔）\n\
3. 说明该文件适合回答什么类型的问题\n\
4. 只返回 Markdown 格式的摘要正文，不要使用代码围栏，不要包含 YAML front matter\n\
5. 如果文本是表格或结构化数据，保留关键数字和分类信息\n\
6. 如果文本是空或无意义内容，返回：无法生成摘要，文件内容不足。"
            },
            {"role": "user", "content": user_prompt}
        ]),
        2048,
    )
    .await
    .map(|r| r.content)
}

/// 为图片对象生成概念笔记（影子笔记）——直接将图片以 base64 发给视觉大模型。
/// images: [("image/png", base64_data), ...]  最多 10 张
/// 若模型不支持视觉输入（返回报错），调用方应回退到 OCR + 文本方案。
pub async fn generate_concept_note_with_images(
    vault_path: &Path,
    object_title: &str,
    images: &[(String, String)], // (mime_type, raw_base64 without prefix)
) -> Result<String, String> {
    if images.is_empty() {
        return Err("没有可发送的图片".to_string());
    }

    // 最多 10 张，避免请求过大
    let images: Vec<&(String, String)> = images.iter().take(10).collect();

    let mut content_parts: Vec<Value> = vec![
        json!({
            "type": "text",
            "text": format!(
                "文件标题：{}\n\n请仔细查看图片内容，生成一份概念笔记（影子笔记）。要求：\n\
1. 用 2-3 段话概括图片的核心内容（如截图描述的界面、图表的数据趋势、照片中的关键元素等）\n\
2. 列出图片涉及的主要主题（用逗号分隔）\n\
3. 说明该图片适合回答什么类型的问题\n\
4. 只返回 Markdown 格式的摘要正文，不要使用代码围栏\n\
5. 如果图片是纯文字（文档/表格截图），请提取并整理关键信息\n\
6. 如果图片模糊不清或无法理解，返回：无法生成摘要，文件内容不足。",
                object_title
            )
        }),
    ];

    for (mime, b64) in &images {
        content_parts.push(json!({
            "type": "image_url",
            "image_url": {
                "url": format!("data:{};base64,{}", mime, b64)
            }
        }));
    }

    chat(
        vault_path,
        json!([
            {
                "role": "system",
                "content": "你是 KBOX 的知识摘要助手。你具备视觉理解能力，可以直接分析图片内容并生成结构化概念笔记。"
            },
            {"role": "user", "content": content_parts}
        ]),
        2048,
    )
    .await
    .map(|r| r.content)
}

fn extract_json_object(text: &str) -> Result<&str, String> {
    let start = text
        .find('{')
        .ok_or_else(|| "模型没有返回 JSON 对象".to_string())?;
    let end = text
        .rfind('}')
        .ok_or_else(|| "模型返回的 JSON 不完整".to_string())?;
    if end < start {
        return Err("模型返回的 JSON 不完整".to_string());
    }
    Ok(&text[start..=end])
}

pub async fn organize_knowledge_base<F>(
    vault_path: &Path,
    task_id: &str,
    collection_id: Option<&str>,
    instruction: &str,
    conversation_context: &[ConversationTurn],
    cancelled: Arc<AtomicBool>,
    mut on_action: F,
) -> Result<OrganizeResult, String>
where
    F: FnMut(String),
{
    if instruction.trim().is_empty() {
        return Err("请输入整理要求".to_string());
    }
    let config = load_config(vault_path);
    if config.model.trim().is_empty() {
        return Err("请先在设置中配置模型".to_string());
    }
    let kb = KnowledgeBase::new(vault_path);
    ensure_not_cancelled(&cancelled)?;
    let conn = kb.conn().map_err(|e| e.to_string())?;
    let objects = db::list_objects(&conn).map_err(|e| e.to_string())?;
    drop(conn);
    let current_tree = kb.get_collection_tree().map_err(|e| e.to_string())?;
    let scope_path = if let Some(collection_id) = collection_id {
        let (path, _object_ids, _scoped_tree) = collection_scope(&current_tree, collection_id)
            .ok_or_else(|| format!("目录不存在或已被删除: {collection_id}"))?;
        Some(path)
    } else {
        None
    };
    let prompt_tree = current_tree;
    if objects.is_empty() {
        let summary = if scope_path.is_some() {
            "选定目录中没有需要整理的对象".to_string()
        } else {
            "知识库为空，没有需要整理的对象".to_string()
        };
        return Ok(OrganizeResult {
            task_id: task_id.to_string(),
            summary,
            actions: Vec::new(),
            moved_objects: 0,
            created_directories: 0,
            archived_objects: 0,
            removed_directories: 0,
        });
    }

    let mut inventory = Vec::with_capacity(objects.len());
    for object in objects.iter().take(2000) {
        ensure_not_cancelled(&cancelled)?;
        let (_, tags, relations) = kb
            .get_object_detail(&object.id)
            .map_err(|e| e.to_string())?;
        let content = kb.read_object_content(&object.id).unwrap_or_default();
        let excerpt: String = content.chars().take(600).collect();
        inventory.push(json!({
            "id": object.id,
            "title": object.title,
            "type": object.r#type,
            "format": object.format,
            "current_path": object.group_name,
            "tags": tags,
            "relation_count": relations.len(),
            "excerpt": excerpt
        }));
    }
    let analyzing = format!("正在分析 {} 个知识对象", inventory.len());
    kb.add_ai_task_action(task_id, "analyze", None, None, &analyzing)
        .map_err(|e| e.to_string())?;
    on_action(analyzing);

    let scope_description = scope_path
        .as_deref()
        .map(|path| {
            format!("当前从目录「{path}」开始关注；目录 path 使用知识库根路径，AI 可按任务需要操作其他目录。")
        })
        .unwrap_or_else(|| "整理整个知识库。".to_string());
    let context_text = conversation_context_text(conversation_context);
    let context_section = if context_text.is_empty() {
        String::new()
    } else {
        format!("最近对话（仅用于理解本次追问）：\n{context_text}\n\n")
    };
    let prompt = format!(
        "整理范围：{scope_description}\n\n{context_section}用户本次要求：\n{}\n\n当前目录树：\n{}\n\n知识对象清单：\n{}",
        instruction.trim(),
        serde_json::to_string(&prompt_tree).map_err(|e| e.to_string())?,
        serde_json::to_string(&inventory).map_err(|e| e.to_string())?
    );
    ensure_not_cancelled(&cancelled)?;
    let request = chat(
        vault_path,
        json!([
            {
                "role": "system",
                "content": "你是 KBOX 知识库整理 Agent。根据用户要求设计清晰、稳定、最多四层的中文逻辑目录树，并把对象分配到最合适的主要目录。不得虚构对象 ID，不修改正文，不永久删除。需要归档的对象放入 archive_object_ids。只返回 JSON，不要解释。JSON 结构必须是：{\"directories\":[{\"path\":\"一级/二级\",\"description\":\"目录语义\"}],\"assignments\":[{\"object_id\":\"obj_x\",\"path\":\"一级/二级\"}],\"archive_object_ids\":[\"obj_x\"],\"summary\":\"整理思路摘要\"}"
            },
            {"role": "user", "content": prompt}
        ]),
        12000,
    );
    let response = tokio::select! {
        response = request => response?,
        _ = wait_for_cancellation(Arc::clone(&cancelled)) => return Err(CANCELLED_ERROR.to_string()),
    };
    ensure_not_cancelled(&cancelled)?;
    let plan: OrganizePlan = match serde_json::from_str(extract_json_object(&response.content)?) {
        Ok(plan) => plan,
        Err(error) => {
            let message = format!("无法解析模型生成的整理方案: {error}");
            return Err(message);
        }
    };

    let valid_ids: HashSet<String> = objects.iter().map(|object| object.id.clone()).collect();
    let mut directory_ids = HashMap::new();
    let mut directories = plan.directories;
    directories.sort_by_key(|directory| directory.path.matches('/').count());
    let mut actions = Vec::new();
    let mut created_directories = 0;
    for directory in directories {
        ensure_not_cancelled(&cancelled)?;
        let Some(path) = resolved_plan_path(scope_path.as_deref(), &directory.path) else {
            continue;
        };
        let id = kb
            .ensure_collection_path(&path, &directory.description, "agent")
            .map_err(|e| e.to_string())?;
        directory_ids.insert(path.clone(), id.clone());
        let description = format!("建立目录：{path}");
        kb.add_ai_task_action(task_id, "create_collection", None, Some(&id), &description)
            .map_err(|e| e.to_string())?;
        on_action(description.clone());
        actions.push(description);
        created_directories += 1;
    }

    let mut moved_objects = 0;
    for assignment in plan.assignments {
        ensure_not_cancelled(&cancelled)?;
        if !valid_ids.contains(&assignment.object_id) {
            continue;
        }
        let Some(path) = resolved_plan_path(scope_path.as_deref(), &assignment.path) else {
            continue;
        };
        let collection_id = if let Some(id) = directory_ids.get(&path) {
            id.clone()
        } else {
            let id = kb
                .ensure_collection_path(&path, "", "agent")
                .map_err(|e| e.to_string())?;
            directory_ids.insert(path.clone(), id.clone());
            id
        };
        kb.move_object_to_collection(&assignment.object_id, &collection_id)
            .map_err(|e| e.to_string())?;
        let description = format!("移动 {} → {}", assignment.object_id, path);
        kb.add_ai_task_action(
            task_id,
            "move_object",
            Some(&assignment.object_id),
            Some(&collection_id),
            &description,
        )
        .map_err(|e| e.to_string())?;
        on_action(description.clone());
        actions.push(description);
        moved_objects += 1;
    }

    let archive_id = if plan.archive_object_ids.is_empty() {
        None
    } else {
        Some(
            kb.ensure_collection_path(
                &scope_path
                    .as_deref()
                    .map(|path| format!("{path}/归档"))
                    .unwrap_or_else(|| "归档".to_string()),
                "用户手动发起整理后归档的内容",
                "agent",
            )
            .map_err(|e| e.to_string())?,
        )
    };
    let mut archived_objects = 0;
    if let Some(archive_id) = archive_id {
        for object_id in plan.archive_object_ids {
            ensure_not_cancelled(&cancelled)?;
            if !valid_ids.contains(&object_id) {
                continue;
            }
            kb.move_object_to_collection(&object_id, &archive_id)
                .map_err(|e| e.to_string())?;
            let description = format!("归档对象：{object_id}");
            kb.add_ai_task_action(
                task_id,
                "archive_object",
                Some(&object_id),
                Some(&archive_id),
                &description,
            )
            .map_err(|e| e.to_string())?;
            on_action(description.clone());
            actions.push(description);
            archived_objects += 1;
        }
    }
    ensure_not_cancelled(&cancelled)?;
    let removed_directories = if scope_path.is_none() {
        kb.prune_empty_collections().map_err(|e| e.to_string())?
    } else {
        0
    };
    if removed_directories > 0 {
        let description = format!("清理 {removed_directories} 个空目录");
        kb.add_ai_task_action(task_id, "prune_collections", None, None, &description)
            .map_err(|e| e.to_string())?;
        on_action(description.clone());
        actions.push(description);
    }
    let summary = if plan.summary.trim().is_empty() {
        format!(
            "已建立 {created_directories} 个目录，移动 {moved_objects} 个对象，归档 {archived_objects} 个对象"
        )
    } else {
        plan.summary.trim().to_string()
    };
    Ok(OrganizeResult {
        task_id: task_id.to_string(),
        summary,
        actions,
        moved_objects,
        created_directories,
        archived_objects,
        removed_directories,
    })
}

// ---------------------------------------------------------------------------
// PR3：多轮 Tool Calling Runner
// ---------------------------------------------------------------------------

/// 首轮工具请求因 Provider 不支持 tools 而失败时返回的哨兵错误，
/// 调度器据此回退到单次 JSON 规划（organize_knowledge_base）。
const FALLBACK_SENTINEL: &str = "__AGENT_LOOP_FALLBACK__";

/// 将共享 Tool Registry 转成 OpenAI-compatible 的 `tools` 数组。
pub fn build_tools_json() -> Value {
    let tools: Vec<Value> = tool_definitions()
        .iter()
        .map(|d| {
            json!({
                "type": "function",
                "function": {
                    "name": d.name,
                    "description": d.description,
                    "parameters": d.input_schema.clone(),
                }
            })
        })
        .collect();
    json!(tools)
}

fn build_assistant_message(response: &ProviderResponse) -> Value {
    let tool_calls: Vec<Value> = response
        .tool_calls
        .iter()
        .map(|tc| {
            json!({
                "id": tc.id,
                "type": "function",
                "function": { "name": tc.function.name, "arguments": tc.function.arguments },
            })
        })
        .collect();
    let mut msg = json!({
        "role": "assistant",
        "content": if response.content.is_empty() { Value::Null } else { json!(response.content) },
    });
    if !tool_calls.is_empty() {
        msg["tool_calls"] = json!(tool_calls);
    }
    msg
}

/// 生成给用户日志看的可读动作描述（入参 + 成败标记）。
fn describe_tool_call(name: &str, args: &Value, result: &ToolResult) -> String {
    let target = match name {
        "create_collection" => format!("建立目录：{}", args["path"].as_str().unwrap_or("")),
        "move_object" => format!(
            "移动 {} → 目录 {}",
            args["object_id"].as_str().unwrap_or(""),
            args["collection_id"].as_str().unwrap_or("")
        ),
        "archive_object" => format!("归档 {}", args["id"].as_str().unwrap_or("")),
        "search" => format!("搜索：{}", args["query"].as_str().unwrap_or("")),
        "get_object" | "get_object_preview" | "get_relations" | "get_incoming_relations" => {
            format!("读取对象 {}", args["id"].as_str().unwrap_or(""))
        }
        "update_content" => format!("更新内容 {}", args["id"].as_str().unwrap_or("")),
        "link_objects" => format!(
            "关联 {} ↔ {}",
            args["from_id"].as_str().unwrap_or(""),
            args["to_id"].as_str().unwrap_or("")
        ),
        "delete_object" => format!("删除 {}", args["id"].as_str().unwrap_or("")),
        "create_note" => format!("新建笔记 {}", args["title"].as_str().unwrap_or("")),
        "ingest_file" => format!("入库 {}", args["file_path"].as_str().unwrap_or("")),
        "ingest_url" => format!("入库 URL {}", args["url"].as_str().unwrap_or("")),
        "ocr_object" => format!("OCR 图片 {}", args["id"].as_str().unwrap_or("")),
        "add_tags" => format!("更新标签 {}", args["id"].as_str().unwrap_or("")),
        "update_metadata" => format!("更新元数据 {}", args["id"].as_str().unwrap_or("")),
        "export" => format!("导出到 {}", args["destination_path"].as_str().unwrap_or("")),
        _ => name.to_string(),
    };
    let flag = if result.is_error { "⚠ 失败" } else { "✓" };
    format!("{flag} {target}")
}

/// 执行多轮 Tool Calling。兼容模型可连续执行"搜索 → 读取 → 建目录 → 移动 → 建关系"等动作；
/// 每个动作先写 `ai_task_actions` 再通过 `on_action` 推送；达到轮次上限以可诊断摘要结束。
#[allow(clippy::too_many_arguments)]
pub async fn run_agent_loop<F, G>(
    vault_path: &Path,
    task_id: &str,
    collection_id: Option<&str>,
    instruction: &str,
    conversation_context: &[ConversationTurn],
    cancelled: Arc<AtomicBool>,
    mut on_action: F,
    mut on_stream: G,
) -> Result<OrganizeResult, String>
where
    F: FnMut(String),
    G: FnMut(String),
{
    ensure_not_cancelled(&cancelled)?;
    let config = load_config(vault_path);
    if config.model.trim().is_empty() {
        return Err("请先在设置中配置模型".to_string());
    }
    let kb = KnowledgeBase::new(vault_path);
    let scope = match collection_id {
        Some(id) => TaskScope::Collection { id: id.to_string() },
        None => TaskScope::KnowledgeBase,
    };
    let scope_description = match &scope {
        TaskScope::KnowledgeBase => "整理整个知识库。".to_string(),
        TaskScope::Collection { id } => {
            let conn = kb.conn().map_err(|e| e.to_string())?;
            let path = db::collection_path(&conn, id).map_err(|e| e.to_string())?;
            format!("当前关注目录为「{path}」，但可按任务需要操作知识库内其他合法对象和目录。")
        }
    };
    let tools = build_tools_json();
    let system = format!(
        "你是 KBOX 内置 AI 知识整理 Agent。使用提供的工具完成用户的整理要求。\n不得虚构对象 ID；用户已手动触发任务，可按需要操作知识库内其他合法对象和目录；不永久删除，需要归档时用 archive_object 软归档。\n{scope_description}"
    );
    let mut messages: Vec<Value> = vec![json!({ "role": "system", "content": system })];
    messages.extend(
        recent_conversation(conversation_context)
            .into_iter()
            .map(|turn| json!({ "role": turn.role, "content": turn.content })),
    );
    messages.push(json!({ "role": "user", "content": instruction.trim() }));
    let max_rounds = config.max_tool_rounds.clamp(1, 32) as usize;
    let mut round = 0_usize;
    let mut actions: Vec<String> = Vec::new();
    let (mut moved, mut created, mut archived) = (0, 0, 0);

    loop {
        ensure_not_cancelled(&cancelled)?;
        let first_round = round == 0;
        let request = chat_request_streaming(
            vault_path,
            json!(messages),
            12000,
            Some(&tools),
            &mut on_stream,
        );
        let response = tokio::select! {
            resp = request => match resp {
                Ok(r) => r,
                Err(_e) if first_round => return Err(FALLBACK_SENTINEL.to_string()),
                Err(e) => return Err(e),
            },
            _ = wait_for_cancellation(Arc::clone(&cancelled)) => return Err(CANCELLED_ERROR.to_string()),
        };
        ensure_not_cancelled(&cancelled)?;
        messages.push(build_assistant_message(&response));

        if response.tool_calls.is_empty() || response.finish_reason == "stop" {
            let summary = if response.content.trim().is_empty() {
                "AI 已完成整理".to_string()
            } else {
                response.content.trim().to_string()
            };
            return Ok(OrganizeResult {
                task_id: task_id.to_string(),
                summary,
                actions,
                moved_objects: moved,
                created_directories: created,
                archived_objects: archived,
                removed_directories: 0,
            });
        }

        round += 1;
        if round > max_rounds {
            let summary = format!(
                "已达到最大工具调用轮次（{max_rounds}），已完成的动作保留，未开始的动作不再执行。"
            );
            return Ok(OrganizeResult {
                task_id: task_id.to_string(),
                summary,
                actions,
                moved_objects: moved,
                created_directories: created,
                archived_objects: archived,
                removed_directories: 0,
            });
        }

        let mut tool_results: Vec<Value> = Vec::new();
        for tool_call in &response.tool_calls {
            ensure_not_cancelled(&cancelled)?;
            let name = tool_call.function.name.clone();
            let args: Value =
                serde_json::from_str(&tool_call.function.arguments).unwrap_or(Value::Null);
            let call = ToolCall {
                name: name.clone(),
                arguments: args.clone(),
            };
            let result = execute_tool(&kb, &call, &scope);
            let description = describe_tool_call(&name, &args, &result);
            let object_id = args
                .get("object_id")
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .map(str::to_string);
            let collection_id = args
                .get("collection_id")
                .and_then(|v| v.as_str())
                .map(str::to_string);
            kb.add_ai_task_action(
                task_id,
                &name,
                object_id.as_deref(),
                collection_id.as_deref(),
                &description,
            )
            .map_err(|e| e.to_string())?;
            on_action(description.clone());
            actions.push(description);
            match name.as_str() {
                "create_collection" => created += 1,
                "move_object" => moved += 1,
                "archive_object" => archived += 1,
                _ => {}
            }
            tool_results.push(json!({
                "role": "tool",
                "tool_call_id": tool_call.id,
                "content": result.content,
            }));
        }
        messages.extend(tool_results);
    }
}

/// 调度器：优先走多轮 Tool Calling；首轮因 Provider 不支持 tools 而失败时回退到单次 JSON 规划。
#[allow(clippy::too_many_arguments)]
pub async fn organize_with_agent<F, G>(
    vault_path: &Path,
    task_id: &str,
    collection_id: Option<&str>,
    instruction: &str,
    conversation_context: &[ConversationTurn],
    cancelled: Arc<AtomicBool>,
    mut on_action: F,
    mut on_stream: G,
) -> Result<OrganizeResult, String>
where
    F: FnMut(String),
    G: FnMut(String),
{
    match run_agent_loop(
        vault_path,
        task_id,
        collection_id,
        instruction,
        conversation_context,
        Arc::clone(&cancelled),
        &mut on_action,
        &mut on_stream,
    )
    .await
    {
        Ok(result) => Ok(result),
        Err(error) if error == FALLBACK_SENTINEL => {
            organize_knowledge_base(
                vault_path,
                task_id,
                collection_id,
                instruction,
                conversation_context,
                cancelled,
                &mut on_action,
            )
            .await
        }
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        apply_sse_line, apply_stream_event, build_tools_json, cosine_similarity, finish_stream,
        infer_api_key_configured, parse_chat_response, parse_embeddings, request_headers,
        resolved_plan_path, AiConfig, StreamAccumulator,
    };
    use std::collections::HashMap;

    #[test]
    fn collection_context_does_not_rewrite_plan_paths() {
        assert_eq!(
            resolved_plan_path(Some("工作/KBOX"), "技术/后端").as_deref(),
            Some("技术/后端")
        );
        assert_eq!(
            resolved_plan_path(Some("工作/KBOX"), "工作/KBOX/产品").as_deref(),
            Some("工作/KBOX/产品")
        );
        assert_eq!(
            resolved_plan_path(Some("工作/KBOX"), "其他目录").as_deref(),
            Some("其他目录")
        );
        assert!(resolved_plan_path(Some("工作/KBOX"), "").is_none());
        assert!(resolved_plan_path(None, "a/b/c/d/e").is_none());
    }

    #[test]
    fn parse_chat_response_extracts_content_and_finish_reason() {
        let value = serde_json::json!({
            "choices": [{
                "message": { "role": "assistant", "content": "你好，KBOX" },
                "finish_reason": "stop"
            }]
        });
        let parsed = parse_chat_response(&value);
        assert_eq!(parsed.content, "你好，KBOX");
        assert_eq!(parsed.finish_reason, "stop");
        assert!(parsed.tool_calls.is_empty());
    }

    #[test]
    fn parse_chat_response_extracts_tool_calls() {
        let value = serde_json::json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "call_1",
                        "type": "function",
                        "function": { "name": "search", "arguments": "{\"query\":\"信号\"}" }
                    }]
                },
                "finish_reason": "tool_calls"
            }]
        });
        let parsed = parse_chat_response(&value);
        assert_eq!(parsed.content, "");
        assert_eq!(parsed.finish_reason, "tool_calls");
        assert_eq!(parsed.tool_calls.len(), 1);
        assert_eq!(parsed.tool_calls[0].id, "call_1");
        assert_eq!(parsed.tool_calls[0].function.name, "search");
        assert_eq!(
            parsed.tool_calls[0].function.arguments,
            "{\"query\":\"信号\"}"
        );
    }

    #[test]
    fn parse_chat_response_handles_empty_or_missing_message() {
        let value = serde_json::json!({ "choices": [] });
        let parsed = parse_chat_response(&value);
        assert_eq!(parsed.content, "");
        assert!(parsed.tool_calls.is_empty());
        assert_eq!(parsed.finish_reason, "");
    }

    #[test]
    fn build_tools_json_is_openai_function_wrapped() {
        let tools = build_tools_json();
        let arr = tools.as_array().expect("tools 应为数组");
        let names: Vec<&str> = arr
            .iter()
            .map(|t| t["function"]["name"].as_str().unwrap_or(""))
            .collect();
        assert!(names.contains(&"search"));
        assert!(names.contains(&"create_collection"));
        assert!(names.contains(&"move_object"));
        assert!(names.contains(&"archive_object"));
        assert!(names.contains(&"add_tags"));
        assert!(names.contains(&"update_metadata"));
        assert!(names.contains(&"export"));
        assert!(names.contains(&"ocr_object"));
        // 每个条目都应是 function 类型且带 parameters
        for t in arr {
            assert_eq!(t["type"], "function");
            assert!(t["function"]["parameters"].is_object());
        }
    }

    #[test]
    fn streaming_response_accumulates_text_and_fragmented_tool_calls() {
        let mut accumulator = StreamAccumulator::default();
        let mut deltas = Vec::new();
        for value in [
            serde_json::json!({"choices":[{"delta":{"content":"正在"},"finish_reason":null}]}),
            serde_json::json!({"choices":[{"delta":{"content":"整理"},"finish_reason":null}]}),
            serde_json::json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_1","function":{"name":"add_","arguments":"{\"id\":"}}]},"finish_reason":null}]}),
            serde_json::json!({"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"name":"tags","arguments":"\"obj\",\"tags\":[\"A\"]}"}}]},"finish_reason":"tool_calls"}]}),
        ] {
            apply_stream_event(&value, &mut accumulator, &mut |delta| deltas.push(delta));
        }
        let response = finish_stream(accumulator);
        assert_eq!(deltas.join(""), "正在整理");
        assert_eq!(response.content, "正在整理");
        assert_eq!(response.finish_reason, "tool_calls");
        assert_eq!(response.tool_calls[0].function.name, "add_tags");
        assert_eq!(
            response.tool_calls[0].function.arguments,
            "{\"id\":\"obj\",\"tags\":[\"A\"]}"
        );
    }

    #[test]
    fn streaming_response_processes_a_final_line_without_newline() {
        let mut accumulator = StreamAccumulator::default();
        let mut deltas = Vec::new();
        let handled = apply_sse_line(
            r#"data: {"choices":[{"delta":{"content":"收尾"},"finish_reason":"stop"}]}"#,
            &mut accumulator,
            &mut |delta| deltas.push(delta),
        );
        let response = finish_stream(accumulator);
        assert!(handled);
        assert_eq!(deltas, ["收尾"]);
        assert_eq!(response.content, "收尾");
        assert_eq!(response.finish_reason, "stop");
    }

    #[test]
    fn embedding_response_is_reordered_by_index() {
        let value = serde_json::json!({
            "data": [
                {"index": 1, "embedding": [0.0, 1.0]},
                {"index": 0, "embedding": [1.0, 0.0]}
            ]
        });
        let vectors = parse_embeddings(&value, 2).expect("embeddings");
        assert_eq!(vectors, vec![vec![1.0, 0.0], vec![0.0, 1.0]]);
        assert_eq!(cosine_similarity(&vectors[0], &vectors[0]), 1.0);
        assert_eq!(cosine_similarity(&vectors[0], &vectors[1]), 0.0);
    }

    #[test]
    fn api_key_overrides_custom_authorization_header() {
        let mut headers = HashMap::new();
        headers.insert("Authorization".to_string(), "Bearer stale-key".to_string());
        let config = AiConfig {
            headers,
            ..AiConfig::default()
        };
        let result = request_headers(&config, "fresh-key").expect("headers");
        assert_eq!(
            result
                .get(reqwest::header::AUTHORIZATION)
                .and_then(|value| value.to_str().ok()),
            Some("Bearer fresh-key")
        );
    }

    #[test]
    fn x_api_key_and_no_auth_modes_are_supported() {
        let x_api_key_config = AiConfig {
            auth_mode: "x-api-key".to_string(),
            ..AiConfig::default()
        };
        let x_api_key_headers =
            request_headers(&x_api_key_config, "provider-key").expect("headers");
        assert_eq!(
            x_api_key_headers
                .get("x-api-key")
                .and_then(|value| value.to_str().ok()),
            Some("provider-key")
        );

        let no_auth_config = AiConfig {
            auth_mode: "none".to_string(),
            ..AiConfig::default()
        };
        let no_auth_headers = request_headers(&no_auth_config, "ignored-key").expect("headers");
        assert!(no_auth_headers
            .get(reqwest::header::AUTHORIZATION)
            .is_none());
        assert!(no_auth_headers.get("x-api-key").is_none());
    }

    #[test]
    fn legacy_config_key_status_is_inferred_without_reading_keychain() {
        let configured = AiConfig {
            model: "deepseek-chat".to_string(),
            ..AiConfig::default()
        };
        assert!(infer_api_key_configured(&configured));

        let missing_model = AiConfig::default();
        assert!(!infer_api_key_configured(&missing_model));

        let local_without_auth = AiConfig {
            model: "llama3".to_string(),
            auth_mode: "none".to_string(),
            ..AiConfig::default()
        };
        assert!(!infer_api_key_configured(&local_without_auth));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_keyring_backend_is_persistent() {
        use keyring::credential::CredentialPersistence;

        assert!(matches!(
            keyring::macos::default_credential_builder().persistence(),
            CredentialPersistence::UntilDelete
        ));
    }
}
