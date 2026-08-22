use kb_core::agent_tools::{execute_tool, tool_definitions, TaskScope, ToolCall, ToolResult};
use kb_core::{db, KnowledgeBase};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::collections::HashSet;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
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
    pub headers: HashMap<String, String>,
    #[serde(default = "default_tool_rounds")]
    pub max_tool_rounds: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct AiConfigView {
    pub provider: String,
    pub base_url: String,
    pub model: String,
    pub headers: HashMap<String, String>,
    pub max_tool_rounds: u32,
    pub api_key_configured: bool,
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
    let path = requested.trim().trim_matches('/');
    if path.is_empty() || path.split('/').count() > 4 {
        return None;
    }
    match scope_path {
        Some(scope) if path == scope || path.starts_with(&format!("{scope}/")) => {
            Some(path.to_string())
        }
        Some(scope) => Some(format!("{scope}/{path}")),
        None => Some(path.to_string()),
    }
}

impl Default for AiConfig {
    fn default() -> Self {
        Self {
            provider: default_provider(),
            base_url: default_base_url(),
            model: String::new(),
            headers: HashMap::new(),
            max_tool_rounds: default_tool_rounds(),
        }
    }
}

fn default_provider() -> String {
    "openai-compatible".to_string()
}

fn default_base_url() -> String {
    "https://api.openai.com/v1".to_string()
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

pub fn load_config(vault_path: &Path) -> AiConfig {
    std::fs::read_to_string(config_path(vault_path))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

pub fn get_config_view(vault_path: &Path) -> AiConfigView {
    let config = load_config(vault_path);
    let api_key_configured = keyring_entry()
        .and_then(|entry| entry.get_password().map_err(|e| e.to_string()))
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false);
    AiConfigView {
        provider: config.provider,
        base_url: config.base_url,
        model: config.model,
        headers: config.headers,
        max_tool_rounds: config.max_tool_rounds,
        api_key_configured,
    }
}

pub fn save_config(
    vault_path: &Path,
    config: AiConfig,
    api_key: Option<String>,
) -> Result<(), String> {
    if config.base_url.trim().is_empty() {
        return Err("Base URL 不能为空".to_string());
    }
    if config.model.trim().is_empty() {
        return Err("模型名称不能为空".to_string());
    }
    let path = config_path(vault_path);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let temp = path.with_extension("json.tmp");
    let data = serde_json::to_vec_pretty(&config).map_err(|e| e.to_string())?;
    std::fs::write(&temp, data).map_err(|e| e.to_string())?;
    std::fs::rename(&temp, &path).map_err(|e| e.to_string())?;

    if let Some(api_key) = api_key {
        let trimmed = api_key.trim();
        if !trimmed.is_empty() {
            keyring_entry()?
                .set_password(trimmed)
                .map_err(|e| format!("API Key 保存失败: {e}"))?;
        }
    }
    Ok(())
}

fn chat_completions_url(base_url: &str) -> String {
    let base = base_url.trim().trim_end_matches('/');
    if base.ends_with("/chat/completions") {
        base.to_string()
    } else {
        format!("{base}/chat/completions")
    }
}

fn request_headers(config: &AiConfig, api_key: &str) -> Result<HeaderMap, String> {
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    if !api_key.trim().is_empty() {
        let value = HeaderValue::from_str(&format!("Bearer {}", api_key.trim()))
            .map_err(|e| format!("API Key 格式无效: {e}"))?;
        headers.insert(AUTHORIZATION, value);
    }
    for (name, value) in &config.headers {
        let name = HeaderName::from_bytes(name.as_bytes())
            .map_err(|e| format!("Header 名称无效（{name}）: {e}"))?;
        let value = HeaderValue::from_str(value).map_err(|e| format!("Header 值无效: {e}"))?;
        headers.insert(name, value);
    }
    Ok(headers)
}

fn saved_api_key() -> Result<String, String> {
    keyring_entry()?
        .get_password()
        .map_err(|_| "尚未配置 API Key".to_string())
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
    let api_key = saved_api_key().unwrap_or_default();
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
    let mut objects = db::list_objects(&conn).map_err(|e| e.to_string())?;
    drop(conn);
    let current_tree = kb.get_collection_tree().map_err(|e| e.to_string())?;
    let (scope_path, prompt_tree) = if let Some(collection_id) = collection_id {
        let (path, object_ids, scoped_tree) = collection_scope(&current_tree, collection_id)
            .ok_or_else(|| format!("目录不存在或已被删除: {collection_id}"))?;
        objects.retain(|object| object_ids.contains(&object.id));
        (Some(path), vec![scoped_tree])
    } else {
        (None, current_tree)
    };
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
            format!("仅整理目录「{path}」及其子目录，返回的目录 path 必须是相对于该目录的路径。")
        })
        .unwrap_or_else(|| "整理整个知识库。".to_string());
    let prompt = format!(
        "整理范围：{scope_description}\n\n用户整理要求：\n{}\n\n当前目录树：\n{}\n\n知识对象清单：\n{}",
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
    on_action(format!("整理完成：{summary}"));
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
        _ => name.to_string(),
    };
    let flag = if result.is_error { "⚠ 失败" } else { "✓" };
    format!("{flag} {target}")
}

/// 执行多轮 Tool Calling。兼容模型可连续执行"搜索 → 读取 → 建目录 → 移动 → 建关系"等动作；
/// 每个动作先写 `ai_task_actions` 再通过 `on_action` 推送；达到轮次上限以可诊断摘要结束。
pub async fn run_agent_loop<F>(
    vault_path: &Path,
    task_id: &str,
    collection_id: Option<&str>,
    instruction: &str,
    cancelled: Arc<AtomicBool>,
    mut on_action: F,
) -> Result<OrganizeResult, String>
where
    F: FnMut(String),
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
            format!("仅整理目录「{path}」及其子目录，只能在授权范围内操作。")
        }
    };
    let tools = build_tools_json();
    let system = format!(
        "你是 KBOX 内置 AI 知识整理 Agent。使用提供的工具完成用户的整理要求。\n不得虚构对象 ID；只能操作授权范围内的内容；不永久删除，需要归档时用 archive_object 软归档。\n{scope_description}"
    );
    let mut messages: Vec<Value> = vec![
        json!({ "role": "system", "content": system }),
        json!({ "role": "user", "content": instruction.trim() }),
    ];
    let max_rounds = config.max_tool_rounds.clamp(1, 32) as usize;
    let mut round = 0_usize;
    let mut actions: Vec<String> = Vec::new();
    let (mut moved, mut created, mut archived) = (0, 0, 0);

    loop {
        ensure_not_cancelled(&cancelled)?;
        let first_round = round == 0;
        let request = chat_request(vault_path, json!(messages), 12000, Some(&tools));
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
            on_action(format!("完成：{summary}"));
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
            on_action(summary.clone());
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
pub async fn organize_with_agent<F>(
    vault_path: &Path,
    task_id: &str,
    collection_id: Option<&str>,
    instruction: &str,
    cancelled: Arc<AtomicBool>,
    mut on_action: F,
) -> Result<OrganizeResult, String>
where
    F: FnMut(String),
{
    match run_agent_loop(
        vault_path,
        task_id,
        collection_id,
        instruction,
        Arc::clone(&cancelled),
        &mut on_action,
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
    use super::{build_tools_json, parse_chat_response, resolved_plan_path};

    #[test]
    fn scoped_plan_paths_cannot_escape_the_selected_collection() {
        assert_eq!(
            resolved_plan_path(Some("工作/KBOX"), "技术/后端").as_deref(),
            Some("工作/KBOX/技术/后端")
        );
        assert_eq!(
            resolved_plan_path(Some("工作/KBOX"), "工作/KBOX/产品").as_deref(),
            Some("工作/KBOX/产品")
        );
        assert_eq!(
            resolved_plan_path(Some("工作/KBOX"), "其他目录").as_deref(),
            Some("工作/KBOX/其他目录")
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
        // 每个条目都应是 function 类型且带 parameters
        for t in arr {
            assert_eq!(t["type"], "function");
            assert!(t["function"]["parameters"].is_object());
        }
    }
}
