mod ai;

use kb_core::{db, model::*, KnowledgeBase};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{Emitter, Manager};
use walkdir::WalkDir;

#[derive(Default)]
struct AiRuntimeState {
    cancellations: Arc<Mutex<HashMap<String, Arc<AtomicBool>>>>,
}

#[derive(Debug, Clone, Serialize)]
struct AiTaskEvent {
    task_id: String,
    kind: String,
    description: String,
    result: Option<ai::OrganizeResult>,
}

#[derive(Serialize)]
struct ObjectDetail {
    object: Object,
    tags: Vec<String>,
    relations: Vec<Relation>,
    content: String,
}

#[derive(Debug, Clone, Serialize)]
struct DiscoveryCandidate {
    path: String,
    name: String,
    size: u64,
    modified_at: u64,
    kind: String,
    status: String,
    existing_object_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct FolderImportResult {
    imported: usize,
    skipped: usize,
    failed: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct DiscoveryEntry {
    hash: String,
    object_id: Option<String>,
    ignored: bool,
    #[serde(default)]
    size: u64,
    #[serde(default)]
    modified_at: u64,
    #[serde(default)]
    kind: String,
    #[serde(default)]
    status: String,
    #[serde(default)]
    pending: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct DiscoveryState {
    entries: HashMap<String, DiscoveryEntry>,
}

fn discovery_state_path() -> PathBuf {
    resolve_vault_path().join(".kb").join("discovery.json")
}

fn load_discovery_state() -> DiscoveryState {
    std::fs::read_to_string(discovery_state_path())
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn save_discovery_state(state: &DiscoveryState) -> Result<(), String> {
    let path = discovery_state_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let temp = path.with_extension("json.tmp");
    let data = serde_json::to_vec_pretty(state).map_err(|e| e.to_string())?;
    std::fs::write(&temp, data).map_err(|e| e.to_string())?;
    std::fs::rename(temp, path).map_err(|e| e.to_string())
}

fn discovery_hash(path: &Path) -> Result<String, String> {
    let mut file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(format!("sha256:{:x}", hasher.finalize()))
}

fn discovery_kind(path: &Path) -> Option<&'static str> {
    match path.extension()?.to_str()?.to_lowercase().as_str() {
        "md" | "markdown" | "txt" | "doc" | "docx" | "xls" | "xlsx" | "ppt" | "pptx" => {
            Some("document")
        }
        "pdf" => Some("pdf"),
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "svg" => Some("image"),
        "rs" | "py" | "js" | "jsx" | "ts" | "tsx" | "go" | "java" | "c" | "h" | "cpp" | "hpp"
        | "swift" | "kt" | "rb" | "php" | "sh" | "sql" | "html" | "css" | "json" | "yaml"
        | "yml" | "toml" => Some("code"),
        "mp3" | "wav" | "m4a" | "mp4" | "mov" => Some("media"),
        _ => None,
    }
}

fn should_skip_discovery(entry: &walkdir::DirEntry, vault_path: &Path) -> bool {
    let path = entry.path();
    if path.starts_with(vault_path) {
        return true;
    }
    let name = entry.file_name().to_string_lossy();
    name.starts_with('.')
        || matches!(
            name.as_ref(),
            "node_modules" | "target" | "Library" | "Applications"
        )
}

fn resolve_vault_path() -> PathBuf {
    if cfg!(debug_assertions) {
        // 开发模式：使用项目目录下的 dev-vault（避免沙箱权限问题）
        let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let project_root = manifest
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."));
        project_root.join("dev-vault")
    } else {
        // 生产模式：使用 ~/KnowledgeBase
        dirs::home_dir().unwrap_or_default().join("KnowledgeBase")
    }
}

fn get_kb() -> KnowledgeBase {
    let vault_path = resolve_vault_path();
    KnowledgeBase::new(&vault_path)
}

// ===== 新增 Tauri 命令 =====

#[tauri::command]
fn get_vault_path() -> String {
    resolve_vault_path().display().to_string()
}

#[tauri::command]
fn get_ai_config() -> ai::AiConfigView {
    ai::get_config_view(&resolve_vault_path())
}

#[tauri::command]
fn save_ai_config(config: ai::AiConfig, api_key: Option<String>) -> Result<(), String> {
    ai::save_config(&resolve_vault_path(), config, api_key)
}

#[tauri::command]
async fn test_ai_connection() -> Result<String, String> {
    ai::test_connection(&resolve_vault_path()).await
}

#[tauri::command]
async fn ai_edit_text(
    instruction: String,
    text: String,
    document_context: Option<String>,
) -> Result<String, String> {
    ai::edit_text(
        &resolve_vault_path(),
        &instruction,
        &text,
        document_context.as_deref(),
    )
    .await
}

#[tauri::command]
fn start_ai_organize(
    app: tauri::AppHandle,
    state: tauri::State<'_, AiRuntimeState>,
    scope: String,
    collection_id: Option<String>,
    instruction: String,
) -> Result<String, String> {
    let instruction = instruction.trim().to_string();
    if instruction.is_empty() {
        return Err("请输入整理要求".to_string());
    }
    let config = ai::load_config(&resolve_vault_path());
    if config.model.trim().is_empty() {
        return Err("请先在设置中配置模型".to_string());
    }
    let normalized_scope = match scope.as_str() {
        "knowledge_base" => {
            if collection_id.is_some() {
                return Err("全库整理不应指定目录".to_string());
            }
            "knowledge_base".to_string()
        }
        "collection" => {
            let id = collection_id
                .as_deref()
                .ok_or_else(|| "请指定要整理的目录".to_string())?;
            let conn = get_kb().conn().map_err(|e| e.to_string())?;
            db::collection_path(&conn, id).map_err(|e| e.to_string())?;
            format!("collection:{id}")
        }
        _ => return Err("不支持的 AI 整理范围".to_string()),
    };

    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let task_id = format!("aitask_{nanos:x}");
    get_kb()
        .create_ai_task(
            &task_id,
            &normalized_scope,
            &instruction,
            &config.provider,
            &config.model,
        )
        .map_err(|e| e.to_string())?;

    let cancelled = Arc::new(AtomicBool::new(false));
    state
        .cancellations
        .lock()
        .map_err(|_| "AI 任务状态锁定失败".to_string())?
        .insert(task_id.clone(), Arc::clone(&cancelled));
    let _ = app.emit(
        "ai-task-event",
        AiTaskEvent {
            task_id: task_id.clone(),
            kind: "started".to_string(),
            description: "AI 整理任务已启动".to_string(),
            result: None,
        },
    );

    let spawned_task_id = task_id.clone();
    let cancellations = Arc::clone(&state.cancellations);
    tauri::async_runtime::spawn(async move {
        let vault_path = resolve_vault_path();
        let action_app = app.clone();
        let action_task_id = spawned_task_id.clone();
        let outcome = ai::organize_knowledge_base(
            &vault_path,
            &spawned_task_id,
            collection_id.as_deref(),
            &instruction,
            cancelled,
            move |description| {
                let _ = action_app.emit(
                    "ai-task-event",
                    AiTaskEvent {
                        task_id: action_task_id.clone(),
                        kind: "action".to_string(),
                        description,
                        result: None,
                    },
                );
            },
        )
        .await;

        let kb = KnowledgeBase::new(&vault_path);
        let event = match outcome {
            Ok(result) => {
                let summary = result.summary.clone();
                let _ = kb.finish_ai_task(&spawned_task_id, "completed", &summary);
                AiTaskEvent {
                    task_id: spawned_task_id.clone(),
                    kind: "completed".to_string(),
                    description: summary,
                    result: Some(result),
                }
            }
            Err(error) if error == ai::CANCELLED_ERROR => {
                let summary = "用户已停止任务".to_string();
                let _ = kb.finish_ai_task(&spawned_task_id, "cancelled", &summary);
                AiTaskEvent {
                    task_id: spawned_task_id.clone(),
                    kind: "cancelled".to_string(),
                    description: summary,
                    result: None,
                }
            }
            Err(error) => {
                let _ = kb.finish_ai_task(&spawned_task_id, "failed", &error);
                AiTaskEvent {
                    task_id: spawned_task_id.clone(),
                    kind: "failed".to_string(),
                    description: error,
                    result: None,
                }
            }
        };
        let _ = app.emit("ai-task-event", event);
        if let Ok(mut tasks) = cancellations.lock() {
            tasks.remove(&spawned_task_id);
        }
    });
    Ok(task_id)
}

#[tauri::command]
fn cancel_ai_task(
    state: tauri::State<'_, AiRuntimeState>,
    task_id: String,
) -> Result<bool, String> {
    let tasks = state
        .cancellations
        .lock()
        .map_err(|_| "AI 任务状态锁定失败".to_string())?;
    if let Some(cancelled) = tasks.get(&task_id) {
        cancelled.store(true, Ordering::Relaxed);
        Ok(true)
    } else {
        Ok(false)
    }
}

#[tauri::command]
fn get_ai_tasks(limit: Option<i64>) -> Result<Vec<AiTask>, String> {
    get_kb()
        .list_ai_tasks(limit.unwrap_or(30))
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn get_ai_task_actions(task_id: String) -> Result<Vec<AiTaskAction>, String> {
    get_kb()
        .list_ai_task_actions(&task_id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn reset_vault() -> Result<(), String> {
    let vault_path = resolve_vault_path();
    let kb = get_kb();

    // 删除 objects 目录下的所有内容
    let objects_dir = vault_path.join("objects");
    if objects_dir.exists() {
        std::fs::remove_dir_all(&objects_dir).map_err(|e| e.to_string())?;
    }

    // 删除数据库文件
    let db_dir = vault_path.join(".kb");
    if db_dir.exists() {
        std::fs::remove_dir_all(&db_dir).map_err(|e| e.to_string())?;
    }

    // 重新初始化
    kb.init().map_err(|e| e.to_string())?;

    Ok(())
}

#[tauri::command]
fn get_file_tree() -> Result<Vec<GroupNode>, String> {
    let kb = get_kb();
    kb.get_file_tree().map_err(|e| e.to_string())
}

#[tauri::command]
fn get_collection_tree() -> Result<Vec<CollectionNode>, String> {
    get_kb().get_collection_tree().map_err(|e| e.to_string())
}

#[tauri::command]
fn create_collection(
    name: String,
    parent_id: Option<String>,
    description: String,
) -> Result<String, String> {
    get_kb()
        .create_collection(
            name.trim(),
            parent_id.as_deref(),
            description.trim(),
            "human",
        )
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn move_object_to_collection(object_id: String, collection_id: String) -> Result<(), String> {
    get_kb()
        .move_object_to_collection(&object_id, &collection_id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn get_object(id: String) -> Result<ObjectDetail, String> {
    let kb = get_kb();
    let (object, tags, relations) = kb.get_object_detail(&id).map_err(|e| e.to_string())?;
    let content = kb.read_object_content(&id).map_err(|e| e.to_string())?;
    Ok(ObjectDetail {
        object,
        tags,
        relations,
        content,
    })
}

#[tauri::command]
fn create_note(title: String, obj_type: String, group: String) -> Result<Object, String> {
    let kb = get_kb();
    kb.create_note(&title, &obj_type, &group)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn create_group(name: String) -> Result<bool, String> {
    get_kb().create_group(&name).map_err(|e| e.to_string())
}

#[tauri::command]
fn rename_object(id: String, title: String) -> Result<(), String> {
    get_kb()
        .rename_object(&id, title.trim())
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn duplicate_object(id: String) -> Result<Object, String> {
    get_kb().duplicate_object(&id).map_err(|e| e.to_string())
}

#[tauri::command]
fn rename_group(old_group: String, new_group: String) -> Result<usize, String> {
    get_kb()
        .rename_group(&old_group, new_group.trim())
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn delete_group(group: String) -> Result<usize, String> {
    get_kb().delete_group(&group).map_err(|e| e.to_string())
}

#[tauri::command]
fn save_object(id: String, title: String, content: String) -> Result<(), String> {
    let kb = get_kb();
    kb.save_object_content(&id, &title, &content)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn search_objects(query: String, limit: Option<i64>) -> Result<Vec<ObjectSummary>, String> {
    let kb = get_kb();
    let limit = limit.unwrap_or(50);
    kb.search(&query, limit).map_err(|e| e.to_string())
}

#[tauri::command]
fn get_stats() -> Result<VaultStats, String> {
    let kb = get_kb();
    kb.get_stats().map_err(|e| e.to_string())
}

#[tauri::command]
fn update_object_group(id: String, group: String) -> Result<(), String> {
    let kb = get_kb();
    kb.update_object_group(&id, &group)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn delete_object(id: String) -> Result<(), String> {
    let kb = get_kb();
    kb.delete_object(&id).map_err(|e| e.to_string())
}

#[tauri::command]
fn link_objects(from_id: String, to_id: String, relation_type: String) -> Result<(), String> {
    let kb = get_kb();
    let conn = kb.conn().map_err(|e| e.to_string())?;
    db::link_objects(&conn, &from_id, &to_id, &relation_type).map_err(|e| e.to_string())
}

#[tauri::command]
fn unlink_objects(from_id: String, to_id: String, relation_type: String) -> Result<(), String> {
    let kb = get_kb();
    let conn = kb.conn().map_err(|e| e.to_string())?;
    db::unlink_objects(&conn, &from_id, &to_id, &relation_type).map_err(|e| e.to_string())
}

#[tauri::command]
fn get_all_relations() -> Result<Vec<serde_json::Value>, String> {
    let kb = get_kb();
    let conn = kb.conn().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare(
            "SELECT r.source_id, r.target_id, r.relation_type, s.title, t.title, s.type, t.type
             FROM relations r
             LEFT JOIN objects s ON s.id = r.source_id
             LEFT JOIN objects t ON t.id = r.target_id
             WHERE s.status != 'deleted' AND t.status != 'deleted'",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok(serde_json::json!({
                "source_id": row.get::<_, String>(0)?,
                "target_id": row.get::<_, String>(1)?,
                "relation_type": row.get::<_, String>(2)?,
                "source_title": row.get::<_, Option<String>>(3)?.unwrap_or_default(),
                "target_title": row.get::<_, Option<String>>(4)?.unwrap_or_default(),
                "source_type": row.get::<_, Option<String>>(5)?.unwrap_or_else(|| "document".to_string()),
                "target_type": row.get::<_, Option<String>>(6)?.unwrap_or_else(|| "document".to_string()),
            }))
        })
        .map_err(|e| e.to_string())?;
    let mut result = Vec::new();
    for r in rows {
        result.push(r.map_err(|e| e.to_string())?);
    }
    Ok(result)
}

#[tauri::command]
fn get_relations_recursive(id: String, max_depth: Option<u32>) -> Result<Vec<Relation>, String> {
    let kb = get_kb();
    let depth = max_depth.unwrap_or(3);
    kb.get_relations_recursive(&id, depth)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn check_consistency() -> Result<serde_json::Value, String> {
    let kb = get_kb();
    kb.check_consistency().map_err(|e| e.to_string())
}

#[tauri::command]
fn backup_db() -> Result<Option<String>, String> {
    let kb = get_kb();
    kb.backup_database()
        .map(|opt| opt.map(|p| p.display().to_string()))
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn rebuild_vault() -> Result<usize, String> {
    let kb = get_kb();
    kb.rebuild().map_err(|e| e.to_string())
}

#[tauri::command]
fn get_operation_log(limit: Option<i64>) -> Result<Vec<serde_json::Value>, String> {
    let kb = get_kb();
    let conn = kb.conn().map_err(|e| e.to_string())?;
    db::get_recent_operations(&conn, limit.unwrap_or(50)).map_err(|e| e.to_string())
}

#[tauri::command]
fn get_trash_objects() -> Result<Vec<ObjectSummary>, String> {
    let kb = get_kb();
    let conn = kb.conn().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare(
            "SELECT id, title, type, format, created_by FROM objects WHERE status = 'deleted' ORDER BY updated_at DESC",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok(ObjectSummary {
                id: row.get(0)?,
                title: row.get(1)?,
                r#type: row.get(2)?,
                format: row.get(3)?,
                score: 0.0,
                snippet: String::new(),
                created_by: row.get(4)?,
            })
        })
        .map_err(|e| e.to_string())?;
    let mut result = Vec::new();
    for r in rows {
        result.push(r.map_err(|e| e.to_string())?);
    }
    Ok(result)
}

#[tauri::command]
fn restore_object(id: String) -> Result<(), String> {
    let kb = get_kb();
    kb.restore_object(&id).map_err(|e| e.to_string())
}

#[tauri::command]
fn permanent_delete(id: String) -> Result<(), String> {
    let kb = get_kb();
    kb.permanent_delete_object(&id).map_err(|e| e.to_string())
}

#[tauri::command]
fn get_mcp_config() -> Result<String, String> {
    let kb_mcp_path = std::env::current_exe()
        .map_err(|e| e.to_string())?
        .parent()
        .ok_or("无法获取 exe 目录")?
        .join("kb-mcp");

    let vault_path = resolve_vault_path();

    let config = serde_json::json!({
        "mcpServers": {
            "knowledge-base": {
                "command": kb_mcp_path.to_str().unwrap_or("kb-mcp"),
                "args": [],
                "env": {
                    "KB_VAULT_PATH": vault_path.to_str().unwrap_or("")
                }
            }
        }
    });

    Ok(serde_json::to_string_pretty(&config).unwrap_or_default())
}

#[tauri::command]
fn check_claude_desktop_config() -> bool {
    // 检查 Claude Desktop 配置文件是否存在（macOS 路径）
    let path = dirs::home_dir()
        .unwrap_or_default()
        .join("Library/Application Support/Claude/claude_desktop_config.json");
    path.exists()
}

#[tauri::command]
fn check_agent_config() -> bool {
    let home = dirs::home_dir().unwrap_or_default();
    let candidates = [
        home.join("Library/Application Support/Claude/claude_desktop_config.json"),
        home.join(".cursor/mcp.json"),
        home.join(".codeium/windsurf/mcp_config.json"),
    ];
    candidates.iter().any(|path| {
        std::fs::read_to_string(path)
            .map(|text| text.contains("knowledge-base") || text.contains("kb-mcp"))
            .unwrap_or(false)
    })
}

#[tauri::command]
fn import_file(file_path: String, group: Option<String>) -> Result<serde_json::Value, String> {
    let kb = get_kb();
    let path = std::path::Path::new(&file_path);
    match kb.ingest_file(path, group.as_deref()) {
        Ok(obj) => serde_json::to_value(&obj).map_err(|e| e.to_string()),
        Err(e) => Err(e.to_string()),
    }
}

#[tauri::command]
fn import_folder(folder_path: String, group: Option<String>) -> Result<FolderImportResult, String> {
    let kb = get_kb();
    let root = PathBuf::from(folder_path);
    if !root.is_dir() {
        return Err("请选择文件夹路径".to_string());
    }

    let mut result = FolderImportResult {
        imported: 0,
        skipped: 0,
        failed: 0,
    };
    for entry in WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_map(std::result::Result::ok)
        .filter(|entry| entry.file_type().is_file())
    {
        match kb.ingest_file(entry.path(), group.as_deref()) {
            Ok(_) => result.imported += 1,
            Err(kb_core::KbError::AlreadyExists(_)) => result.skipped += 1,
            Err(_) => result.failed += 1,
        }
    }
    Ok(result)
}

#[tauri::command]
fn scan_discovery_folders(folders: Vec<String>) -> Result<Vec<DiscoveryCandidate>, String> {
    let kb = get_kb();
    let vault_path = resolve_vault_path();
    let mut state = load_discovery_state();
    let conn = kb.conn().map_err(|e| e.to_string())?;
    let known_objects = db::list_objects(&conn).map_err(|e| e.to_string())?;
    let by_hash: HashMap<String, String> = known_objects
        .iter()
        .filter_map(|obj| obj.file_hash.clone().map(|hash| (hash, obj.id.clone())))
        .collect();
    let mut candidates = Vec::new();

    for folder in folders {
        let root = PathBuf::from(folder);
        if !root.is_dir() || root.starts_with(&vault_path) {
            continue;
        }
        let walker = walkdir::WalkDir::new(&root)
            .max_depth(8)
            .follow_links(false)
            .into_iter()
            .filter_entry(|entry| !should_skip_discovery(entry, &vault_path));
        for entry in walker.filter_map(Result::ok) {
            let path = entry.path();
            if !entry.file_type().is_file() || discovery_kind(path).is_none() {
                continue;
            }
            let metadata = match entry.metadata() {
                Ok(metadata) => metadata,
                Err(_) => continue,
            };
            if metadata.len() == 0 || metadata.len() > 500 * 1024 * 1024 {
                continue;
            }
            let modified_at = metadata
                .modified()
                .ok()
                .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|duration| duration.as_secs())
                .unwrap_or(0);
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            if modified_at > 0 && now.saturating_sub(modified_at) < 10 {
                continue;
            }
            let path_string = path.to_string_lossy().to_string();
            let previous = state.entries.get(&path_string);
            if previous.map(|entry| entry.ignored).unwrap_or(false) {
                continue;
            }
            if let Some(previous) = previous {
                if previous.size == metadata.len() && previous.modified_at == modified_at {
                    if previous.pending {
                        candidates.push(DiscoveryCandidate {
                            path: path_string,
                            name: entry.file_name().to_string_lossy().to_string(),
                            size: metadata.len(),
                            modified_at,
                            kind: previous.kind.clone(),
                            status: previous.status.clone(),
                            existing_object_id: previous.object_id.clone(),
                        });
                    }
                    continue;
                }
            }
            let hash = match discovery_hash(path) {
                Ok(hash) => hash,
                Err(_) => continue,
            };
            if previous.map(|entry| entry.hash == hash).unwrap_or(false) {
                continue;
            }
            let (status, existing_object_id) = if let Some(object_id) = by_hash.get(&hash) {
                ("duplicate".to_string(), Some(object_id.clone()))
            } else if let Some(previous) = previous {
                if previous.object_id.is_some() {
                    ("updated".to_string(), previous.object_id.clone())
                } else {
                    ("new".to_string(), None)
                }
            } else {
                ("new".to_string(), None)
            };
            let kind = discovery_kind(path).unwrap_or("document").to_string();
            state.entries.insert(
                path_string.clone(),
                DiscoveryEntry {
                    hash,
                    object_id: existing_object_id.clone(),
                    ignored: false,
                    size: metadata.len(),
                    modified_at,
                    kind: kind.clone(),
                    status: status.clone(),
                    pending: true,
                },
            );
            candidates.push(DiscoveryCandidate {
                path: path_string,
                name: entry.file_name().to_string_lossy().to_string(),
                size: metadata.len(),
                modified_at,
                kind,
                status,
                existing_object_id,
            });
        }
    }
    candidates.sort_by_key(|item| std::cmp::Reverse(item.modified_at));
    candidates.truncate(300);
    save_discovery_state(&state)?;
    Ok(candidates)
}

#[tauri::command]
fn import_discovery_files(paths: Vec<String>, group: String) -> Result<usize, String> {
    let kb = get_kb();
    let mut state = load_discovery_state();
    let mut imported = 0;
    for path_string in paths {
        let path = PathBuf::from(&path_string);
        let hash = discovery_hash(&path)?;
        match kb.ingest_file(&path, Some(group.trim())) {
            Ok(obj) => {
                state.entries.insert(
                    path_string,
                    DiscoveryEntry {
                        hash,
                        object_id: Some(obj.id),
                        ignored: false,
                        size: std::fs::metadata(&path).map(|meta| meta.len()).unwrap_or(0),
                        modified_at: std::fs::metadata(&path)
                            .and_then(|meta| meta.modified())
                            .ok()
                            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                            .map(|duration| duration.as_secs())
                            .unwrap_or(0),
                        kind: discovery_kind(&path).unwrap_or("document").to_string(),
                        status: "processed".to_string(),
                        pending: false,
                    },
                );
                imported += 1;
            }
            Err(kb_core::KbError::AlreadyExists(object_id)) => {
                state.entries.insert(
                    path_string,
                    DiscoveryEntry {
                        hash,
                        object_id: Some(object_id),
                        ignored: false,
                        size: std::fs::metadata(&path).map(|meta| meta.len()).unwrap_or(0),
                        modified_at: std::fs::metadata(&path)
                            .and_then(|meta| meta.modified())
                            .ok()
                            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                            .map(|duration| duration.as_secs())
                            .unwrap_or(0),
                        kind: discovery_kind(&path).unwrap_or("document").to_string(),
                        status: "processed".to_string(),
                        pending: false,
                    },
                );
            }
            Err(error) => return Err(error.to_string()),
        }
    }
    save_discovery_state(&state)?;
    Ok(imported)
}

#[tauri::command]
fn ignore_discovery_files(paths: Vec<String>) -> Result<(), String> {
    let mut state = load_discovery_state();
    for path_string in paths {
        let path = PathBuf::from(&path_string);
        if !path.is_file() {
            continue;
        }
        state.entries.insert(
            path_string,
            DiscoveryEntry {
                hash: discovery_hash(&path)?,
                object_id: None,
                ignored: true,
                size: std::fs::metadata(&path).map(|meta| meta.len()).unwrap_or(0),
                modified_at: std::fs::metadata(&path)
                    .and_then(|meta| meta.modified())
                    .ok()
                    .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|duration| duration.as_secs())
                    .unwrap_or(0),
                kind: discovery_kind(&path).unwrap_or("document").to_string(),
                status: "ignored".to_string(),
                pending: false,
            },
        );
    }
    save_discovery_state(&state)
}

#[tauri::command]
fn get_asset_url(object_id: String) -> Result<Option<String>, String> {
    let kb = get_kb();
    let path = kb.get_asset_path(&object_id).map_err(|e| e.to_string())?;
    // 返回文件绝对路径，前端用 convertFileSrc 转换
    Ok(path.map(|p| p.to_string_lossy().to_string()))
}

/// 自动检测对象内容中的链接并建立关系
#[tauri::command]
fn auto_link(object_id: String) -> Result<usize, String> {
    let kb = get_kb();
    kb.auto_link_relations(&object_id)
        .map_err(|e| e.to_string())
}

/// 重建所有对象的自动关系
#[tauri::command]
fn rebuild_relations() -> Result<usize, String> {
    let kb = get_kb();
    kb.rebuild_all_relations().map_err(|e| e.to_string())
}

/// 读取二进制文件并返回 base64 data URL（用于图片预览）
#[tauri::command]
fn get_asset_base64(object_id: String) -> Result<Option<String>, String> {
    let kb = get_kb();
    let path = kb.get_asset_path(&object_id).map_err(|e| e.to_string())?;

    if let Some(p) = path {
        let ext = p
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("bin")
            .to_lowercase();
        let mime = match ext.as_str() {
            "png" => "image/png",
            "jpg" | "jpeg" => "image/jpeg",
            "gif" => "image/gif",
            "webp" => "image/webp",
            "bmp" => "image/bmp",
            "svg" => "image/svg+xml",
            "pdf" => "application/pdf",
            _ => "application/octet-stream",
        };
        let data = std::fs::read(&p).map_err(|e| e.to_string())?;
        let b64 = base64_encode(&data);
        Ok(Some(format!("data:{};base64,{}", mime, b64)))
    } else {
        Ok(None)
    }
}

fn base64_encode(data: &[u8]) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut result = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let n = (b0 << 16) | (b1 << 8) | b2;
        result.push(CHARS[((n >> 18) & 63) as usize] as char);
        result.push(CHARS[((n >> 12) & 63) as usize] as char);
        if chunk.len() > 1 {
            result.push(CHARS[((n >> 6) & 63) as usize] as char);
        } else {
            result.push('=');
        }
        if chunk.len() > 2 {
            result.push(CHARS[(n & 63) as usize] as char);
        } else {
            result.push('=');
        }
    }
    result
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(AiRuntimeState::default())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            // 当用户尝试启动第二个实例时，聚焦已有窗口
            let _ = app.get_webview_window("main").map(|w| {
                let _ = w.set_focus();
            });
        }))
        .invoke_handler(tauri::generate_handler![
            get_file_tree,
            get_collection_tree,
            create_collection,
            move_object_to_collection,
            get_object,
            create_note,
            create_group,
            rename_object,
            duplicate_object,
            rename_group,
            delete_group,
            save_object,
            search_objects,
            get_stats,
            update_object_group,
            delete_object,
            link_objects,
            unlink_objects,
            get_all_relations,
            get_trash_objects,
            restore_object,
            permanent_delete,
            get_mcp_config,
            check_claude_desktop_config,
            check_agent_config,
            import_file,
            import_folder,
            scan_discovery_folders,
            import_discovery_files,
            ignore_discovery_files,
            get_asset_url,
            get_asset_base64,
            auto_link,
            rebuild_relations,
            get_relations_recursive,
            check_consistency,
            backup_db,
            rebuild_vault,
            get_operation_log,
            get_vault_path,
            get_ai_config,
            save_ai_config,
            test_ai_connection,
            ai_edit_text,
            start_ai_organize,
            cancel_ai_task,
            get_ai_tasks,
            get_ai_task_actions,
            reset_vault,
        ])
        .setup(|app| {
            // 初始化知识库目录（失败不阻止启动）
            let vault_path = resolve_vault_path();
            let kb = KnowledgeBase::new(&vault_path);
            match kb.init() {
                Ok(()) => {
                    if let Err(error) = kb.recover_running_ai_tasks() {
                        eprintln!("AI 任务恢复失败: {error}");
                    }
                    println!("知识库已就绪: {}", vault_path.display());
                }
                Err(e) => eprintln!("数据库初始化失败（应用仍可运行）: {}", e),
            }

            // 注册全局快捷键 Cmd+Shift+K 唤起/隐藏应用
            use tauri_plugin_global_shortcut::GlobalShortcutExt;
            app.global_shortcut()
                .on_shortcut("CmdOrCtrl+Shift+K", |app, _event, _shortcut| {
                    if let Some(window) = app.get_webview_window("main") {
                        if window.is_visible().unwrap_or(false) {
                            let _ = window.hide();
                        } else {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                })?;

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
