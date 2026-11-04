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

/// 影子笔记 Worker 的全局状态
#[derive(Default)]
struct ConceptNoteState {
    paused: Arc<AtomicBool>,
    enabled: Arc<AtomicBool>,
}

/// 影子笔记开关的持久化结构
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct ConceptNotePersisted {
    #[serde(default)]
    enabled: bool,
    #[serde(default)]
    paused: bool,
}

fn concept_note_state_path() -> PathBuf {
    resolve_vault_path().join(".kb").join("concept-note-state.json")
}

fn load_concept_note_persisted() -> ConceptNotePersisted {
    std::fs::read_to_string(concept_note_state_path())
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn save_concept_note_persisted(state: &ConceptNotePersisted) {
    let path = concept_note_state_path();
    if let Some(parent) = path.parent() {
        if std::fs::create_dir_all(parent).is_err() {
            return;
        }
    }
    let temp = path.with_extension("json.tmp");
    if let Ok(data) = serde_json::to_vec_pretty(state) {
        if std::fs::write(&temp, data).is_ok() {
            let _ = std::fs::rename(&temp, &path);
        }
    }
}

#[derive(Debug, Clone, Serialize)]
struct AiTaskEvent {
    task_id: String,
    kind: String,
    description: String,
    result: Option<ai::OrganizeResult>,
}

#[derive(Debug, Clone, Serialize)]
struct ConceptNoteEvent {
    task_id: Option<i64>,
    object_id: Option<String>,
    object_title: Option<String>,
    kind: String,       // started / completed / failed / progress
    description: String,
    pending: i64,
    running: i64,
    done: i64,
    failed: i64,
}

#[derive(Debug, Clone, Serialize)]
struct ConceptNoteStatus {
    pending: i64,
    running: i64,
    done: i64,
    failed: i64,
    paused: bool,
    enabled: bool,
    tasks: Vec<ConceptNoteTask>,
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

#[derive(Debug, Clone, Serialize)]
struct PasteImageResult {
    id: String,
    title: String,
    format: String,
    format_detail: String,
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

// 发现、导入、忽略共用一个状态文件，后台执行时仍需串行读写。
static DISCOVERY_STATE_LOCK: Mutex<()> = Mutex::new(());

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

fn vault_snapshot(vault_path: &Path) -> HashMap<PathBuf, (u64, u128)> {
    let mut snapshot = HashMap::new();
    for entry in WalkDir::new(vault_path.join("objects"))
        .into_iter()
        .filter_map(std::result::Result::ok)
        .filter(|entry| entry.file_type().is_file())
    {
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        let modified = metadata
            .modified()
            .ok()
            .and_then(|value| value.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        snapshot.insert(entry.path().to_path_buf(), (metadata.len(), modified));
    }
    snapshot
}

fn spawn_vault_watcher(app: tauri::AppHandle, vault_path: PathBuf) {
    std::thread::spawn(move || {
        let mut previous = vault_snapshot(&vault_path);
        loop {
            std::thread::sleep(std::time::Duration::from_secs(3));
            let current = vault_snapshot(&vault_path);
            if current == previous {
                continue;
            }
            previous = current;
            let kb = KnowledgeBase::new(&vault_path);
            match kb.sync_external_changes() {
                Ok(report) if report.rebuilt || !report.changed_object_ids.is_empty() => {
                    let _ = app.emit("vault-changed", report);
                }
                Ok(_) => {}
                Err(error) => eprintln!("外部文件同步失败: {error}"),
            }
        }
    });
}

// ===== 影子笔记（概念笔记）Worker =====

/// 判断 AI 是否已配置（base_url + model + auth_mode 有效）。
fn is_ai_configured(vault_path: &Path) -> bool {
    let config = ai::load_config(vault_path);
    config.auth_mode != "none"
        && !config.base_url.trim().is_empty()
        && !config.model.trim().is_empty()
}

/// 为对象计算 content_hash（与 embedding 缓存使用同一格式）。
fn object_content_hash(file_hash: &str, updated_at: i64) -> String {
    format!("{}:{}", file_hash, updated_at)
}

/// 入队单个对象（如果需要生成概念笔记）。
fn enqueue_concept_note_for_object(
    vault_path: &Path,
    object_id: &str,
) -> Result<(), String> {
    let kb = KnowledgeBase::new(vault_path);
    let conn = kb.conn().map_err(|e| e.to_string())?;
    let obj = db::get_object(&conn, object_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("对象不存在: {object_id}"))?;

    // 只对非文本格式生成
    if obj.format == "markdown" || obj.format == "code" {
        return Ok(());
    }

    // 检查是否已生成
    let metadata: serde_json::Value =
        serde_json::from_str(&obj.metadata).unwrap_or_else(|_| serde_json::json!({}));
    if metadata
        .get("concept_generated")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        // 已生成，检查 hash 是否变化
        let current_hash = object_content_hash(
            obj.file_hash.as_deref().unwrap_or(""),
            obj.updated_at,
        );
        if let Ok(Some((_id, stored_hash, _status))) =
            db::get_concept_note_for_object(&conn, object_id)
        {
            if stored_hash == current_hash {
                return Ok(()); // hash 相同，无需重新生成
            }
        }
    }

    let content_hash = object_content_hash(
        obj.file_hash.as_deref().unwrap_or(""),
        obj.updated_at,
    );
    db::enqueue_concept_note(&conn, object_id, &content_hash).map_err(|e| e.to_string())?;
    Ok(())
}

/// 启动影子笔记后台 Worker。
/// 在 Tauri setup 中调用，独立线程轮询队列。
fn spawn_concept_note_worker(app: tauri::AppHandle, vault_path: PathBuf) {
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(std::time::Duration::from_secs(2));

            // 检查是否暂停
            let state = app.state::<ConceptNoteState>();
            if state.paused.load(Ordering::Relaxed) {
                continue;
            }

            // 功能总开关关闭时不消费队列（手动/批量生成前需先开启）
            if !state.enabled.load(Ordering::Relaxed) {
                continue;
            }

            // 检查 AI 是否已配置
            if !is_ai_configured(&vault_path) {
                continue;
            }

            // 取下一个任务
            let kb = KnowledgeBase::new(&vault_path);
            let task = {
                let conn = match kb.conn() {
                    Ok(c) => c,
                    Err(e) => {
                        eprintln!("影子笔记 Worker 获取数据库连接失败: {e}");
                        continue;
                    }
                };
                match db::claim_next_concept_note(&conn) {
                    Ok(t) => t,
                    Err(e) => {
                        eprintln!("影子笔记 Worker 取任务失败: {e}");
                        continue;
                    }
                }
            };

            let Some(task) = task else {
                continue; // 队列为空
            };

            let task_id = task.id;
            let object_id = task.object_id.clone();
            let object_title = task.object_title.clone();

            // 发送 started 事件
            let (pending, running, done, failed) = {
                let conn = match kb.conn() {
                    Ok(c) => c,
                    Err(_) => continue,
                };
                db::count_concept_notes(&conn).unwrap_or((0, 0, 0, 0))
            };
            let _ = app.emit(
                "concept-note-event",
                ConceptNoteEvent {
                    task_id: Some(task_id),
                    object_id: Some(object_id.clone()),
                    object_title: Some(object_title.clone()),
                    kind: "started".to_string(),
                    description: format!("正在为《{}》生成概念笔记", object_title),
                    pending,
                    running,
                    done,
                    failed,
                },
            );

            // 读取对象提取文本
            let mut extracted_text = match kb.read_extracted_text(&object_id) {
                Ok(text) => text,
                Err(e) => {
                    let error = format!("读取提取文本失败: {e}");
                    eprintln!("影子笔记 Worker: {error} (对象: {object_id})");
                    let conn = match kb.conn() {
                        Ok(c) => c,
                        Err(_) => continue,
                    };
                    let _ = db::fail_concept_note(&conn, task_id, &error, 3);
                    let _ = emit_concept_note_failed(&app, &kb, task_id, &object_id, &object_title, &error);
                    continue;
                }
            };

            // ===== 图片对象：优先走视觉大模型，失败降级 OCR + 文本 =====
            let is_image = task.object_format == "image";
            let mut concept_note: Option<String> = None;

            if is_image {
                // 尝试直接把图片发给视觉大模型
                let image_data = kb.read_image_base64(&object_id).unwrap_or(None);
                if let Some((mime, b64)) = image_data {
                    eprintln!("影子笔记 Worker: 对象 {object_id} 尝试视觉模型生成摘要");
                    match tauri::async_runtime::block_on(async {
                        ai::generate_concept_note_with_images(
                            &vault_path,
                            &object_title,
                            &[(mime, b64)],
                        )
                        .await
                    }) {
                        Ok(note) if !is_concept_note_refusal(&note) => {
                            concept_note = Some(note);
                        }
                        Ok(_) => {
                            eprintln!("影子笔记 Worker: 视觉模型返回拒答，将降级 OCR");
                        }
                        Err(e) => {
                            eprintln!("影子笔记 Worker: 视觉模型请求失败（{e}），将降级 OCR");
                        }
                    }
                }

                // 视觉模型未成功，降级 OCR + 文本
                if concept_note.is_none() {
                    match kb.ensure_extracted_text(&object_id) {
                        Ok(Some(ocr_text)) => {
                            eprintln!("影子笔记 Worker: 对象 {object_id} 已 OCR 补全文本");
                            extracted_text = ocr_text;
                        }
                        Ok(None) => {}
                        Err(e) => eprintln!("影子笔记 Worker: OCR 出错（对象 {object_id}）: {e}"),
                    }
                }
            } else {
                // 非图片对象：扫描件/图片型 PDF 没有文本层时，自动 OCR 补全
                match kb.ensure_extracted_text(&object_id) {
                    Ok(Some(ocr_text)) => {
                        eprintln!("影子笔记 Worker: 对象 {object_id} 已 OCR 补全文本");
                        extracted_text = ocr_text;
                    }
                    Ok(None) => {}
                    Err(e) => eprintln!("影子笔记 Worker: OCR 出错（对象 {object_id}）: {e}"),
                }
            }

            // 如果视觉模型已成功生成摘要，跳过文本路径
            if concept_note.is_none() {
                if extracted_text.trim().is_empty() {
                    let error = "对象没有可提取的文本内容".to_string();
                    let conn = match kb.conn() {
                        Ok(c) => c,
                        Err(_) => continue,
                    };
                    let _ = db::fail_concept_note(&conn, task_id, &error, 3);
                    let _ = emit_concept_note_failed(&app, &kb, task_id, &object_id, &object_title, &error);
                    continue;
                }

                // 调用 AI 生成概念笔记（在同步线程中阻塞等待 async 调用）
                match tauri::async_runtime::block_on(async {
                    ai::generate_concept_note(
                        &vault_path,
                        &object_title,
                        &task.object_format,
                        &extracted_text,
                        8000,
                    )
                    .await
                }) {
                    Ok(note) => { concept_note = Some(note); }
                    Err(e) => {
                        let error = e.to_string();
                        eprintln!("影子笔记生成失败: {error} (对象: {object_id})");
                        let conn = match kb.conn() {
                            Ok(c) => c,
                            Err(_) => continue,
                        };
                        let _ = db::fail_concept_note(&conn, task_id, &error, 3);
                        let _ = emit_concept_note_failed(&app, &kb, task_id, &object_id, &object_title, &error);
                        continue;
                    }
                }
            }

            // 至此 concept_note 必定有值
            let concept_note = match concept_note {
                Some(n) => n,
                None => {
                    let error = "摘要生成失败：未知原因".to_string();
                    let conn = match kb.conn() {
                        Ok(c) => c,
                        Err(_) => continue,
                    };
                    let _ = db::fail_concept_note(&conn, task_id, &error, 3);
                    let _ = emit_concept_note_failed(&app, &kb, task_id, &object_id, &object_title, &error);
                    continue;
                }
            };

            // AI 在文本无意义时会返回固定拒答句；这类结果不能当作摘要保存，
            // 标记为失败，便于在 OCR/文本补全后重试。
            if is_concept_note_refusal(&concept_note) {
                let error = "无法生成摘要：文件内容不足（如为扫描件，请先 OCR）".to_string();
                eprintln!("影子笔记 Worker: {error} (对象: {object_id})");
                let conn = match kb.conn() {
                    Ok(c) => c,
                    Err(_) => continue,
                };
                let _ = db::fail_concept_note(&conn, task_id, &error, 3);
                let _ = emit_concept_note_failed(&app, &kb, task_id, &object_id, &object_title, &error);
                continue;
            }

            // 写入 index.md
            if let Err(e) = kb.save_concept_note(&object_id, &concept_note) {
                let error = format!("写入概念笔记失败: {e}");
                eprintln!("影子笔记 Worker: {error} (对象: {object_id})");
                let conn = match kb.conn() {
                    Ok(c) => c,
                    Err(_) => continue,
                };
                let _ = db::fail_concept_note(&conn, task_id, &error, 3);
                let _ = emit_concept_note_failed(&app, &kb, task_id, &object_id, &object_title, &error);
                continue;
            }

            // 标记完成
            {
                let conn = match kb.conn() {
                    Ok(c) => c,
                    Err(_) => continue,
                };
                if let Err(e) = db::finish_concept_note(&conn, task_id) {
                    eprintln!("影子笔记 Worker 标记完成失败: {e}");
                }
            }

            // 发送 completed 事件
            let (pending, running, done, failed) = {
                let conn = match kb.conn() {
                    Ok(c) => c,
                    Err(_) => continue,
                };
                db::count_concept_notes(&conn).unwrap_or((0, 0, 0, 0))
            };
            let _ = app.emit(
                "concept-note-event",
                ConceptNoteEvent {
                    task_id: Some(task_id),
                    object_id: Some(object_id.clone()),
                    object_title: Some(object_title.clone()),
                    kind: "completed".to_string(),
                    description: format!("《{}》的概念笔记已生成", object_title),
                    pending,
                    running,
                    done,
                    failed,
                },
            );

            // 限速：任务间至少 2 秒
            std::thread::sleep(std::time::Duration::from_secs(2));
        }
    });
}

/// 发送失败事件
fn emit_concept_note_failed(
    app: &tauri::AppHandle,
    kb: &KnowledgeBase,
    task_id: i64,
    object_id: &str,
    object_title: &str,
    error: &str,
) -> Result<(), String> {
    let (pending, running, done, failed) = kb
        .conn()
        .ok()
        .and_then(|conn| db::count_concept_notes(&conn).ok())
        .unwrap_or((0, 0, 0, 0));
    let _ = app.emit(
        "concept-note-event",
        ConceptNoteEvent {
            task_id: Some(task_id),
            object_id: Some(object_id.to_string()),
            object_title: Some(object_title.to_string()),
            kind: "failed".to_string(),
            description: format!("《{}》概念笔记生成失败: {error}", object_title),
            pending,
            running,
            done,
            failed,
        },
    );
    Ok(())
}

// ===== 影子笔记 Tauri 命令 =====

/// 为指定对象入队生成概念笔记。
#[tauri::command]
fn start_concept_note(object_ids: Vec<String>) -> Result<usize, String> {
    let vault_path = resolve_vault_path();
    let mut count = 0;
    for id in &object_ids {
        if enqueue_concept_note_for_object(&vault_path, id).is_ok() {
            count += 1;
        }
    }
    Ok(count)
}

/// 批量入队所有需要生成概念笔记的对象（存量补全）。
#[tauri::command]
fn start_concept_note_batch() -> Result<usize, String> {
    let vault_path = resolve_vault_path();
    let kb = KnowledgeBase::new(&vault_path);
    let objects = kb
        .list_objects_without_concept_note()
        .map_err(|e| e.to_string())?;
    let conn = kb.conn().map_err(|e| e.to_string())?;
    let mut count = 0;
    for (id, _title, _format) in &objects {
        if let Ok(obj) = db::get_object(&conn, id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("对象不存在: {id}"))
        {
            let content_hash =
                object_content_hash(obj.file_hash.as_deref().unwrap_or(""), obj.updated_at);
            if db::enqueue_concept_note(&conn, id, &content_hash).is_ok() {
                count += 1;
            }
        }
    }
    Ok(count)
}

/// 获取队列状态。
#[tauri::command]
fn get_concept_note_status(limit: Option<i64>, app: tauri::AppHandle) -> Result<ConceptNoteStatus, String> {
    let vault_path = resolve_vault_path();
    let kb = KnowledgeBase::new(&vault_path);
    let conn = kb.conn().map_err(|e| e.to_string())?;
    let (pending, running, done, failed) =
        db::count_concept_notes(&conn).map_err(|e| e.to_string())?;
    let tasks = db::list_concept_notes(&conn, None, limit.unwrap_or(50))
        .map_err(|e| e.to_string())?;
    let paused = app.state::<ConceptNoteState>().paused.load(Ordering::Relaxed);
    let enabled = app.state::<ConceptNoteState>().enabled.load(Ordering::Relaxed);
    Ok(ConceptNoteStatus {
        pending,
        running,
        done,
        failed,
        paused,
        enabled,
        tasks,
    })
}

/// 暂停/恢复队列。
#[tauri::command]
fn set_concept_note_paused(paused: bool, app: tauri::AppHandle) -> Result<(), String> {
    let state = app.state::<ConceptNoteState>();
    state.paused.store(paused, Ordering::Relaxed);
    let enabled = state.enabled.load(Ordering::Relaxed);
    save_concept_note_persisted(&ConceptNotePersisted { enabled, paused });
    Ok(())
}

/// 设置影子笔记功能开关。
#[tauri::command]
fn set_concept_note_enabled(enabled: bool, app: tauri::AppHandle) -> Result<(), String> {
    let state = app.state::<ConceptNoteState>();
    state.enabled.store(enabled, Ordering::Relaxed);
    let paused = state.paused.load(Ordering::Relaxed);
    save_concept_note_persisted(&ConceptNotePersisted { enabled, paused });
    Ok(())
}

/// 重试失败的任务。
#[tauri::command]
fn retry_concept_notes(task_ids: Vec<i64>) -> Result<usize, String> {
    let vault_path = resolve_vault_path();
    let kb = KnowledgeBase::new(&vault_path);
    let conn = kb.conn().map_err(|e| e.to_string())?;
    db::retry_concept_notes(&conn, &task_ids).map_err(|e| e.to_string())
}

/// 重试所有失败任务。
#[tauri::command]
fn retry_all_concept_notes() -> Result<usize, String> {
    let vault_path = resolve_vault_path();
    let kb = KnowledgeBase::new(&vault_path);
    let conn = kb.conn().map_err(|e| e.to_string())?;
    let failed_tasks =
        db::list_concept_notes(&conn, Some("failed"), 1000).map_err(|e| e.to_string())?;
    let ids: Vec<i64> = failed_tasks.iter().map(|t| t.id).collect();
    db::retry_concept_notes(&conn, &ids).map_err(|e| e.to_string())
}

/// 检查对象是否已生成概念笔记。
#[tauri::command]
fn check_concept_note(object_id: String) -> Result<bool, String> {
    let vault_path = resolve_vault_path();
    let kb = KnowledgeBase::new(&vault_path);
    kb.has_concept_note(&object_id).map_err(|e| e.to_string())
}

/// 丢弃已有概念摘要并强制重新入队生成（用于修正 AI 拒答脏数据，
/// 或用户希望基于最新 OCR 文本重新生成）。
#[tauri::command]
fn regenerate_concept_note(object_id: String) -> Result<(), String> {
    let vault_path = resolve_vault_path();

    // 入队前先做前置校验，避免任务进队后因开关关闭/未配置 AI 而无任何反馈
    if !load_concept_note_persisted().enabled {
        return Err("请先在概念笔记面板开启“自动生成”开关".to_string());
    }
    if !is_ai_configured(&vault_path) {
        return Err("尚未配置 AI，请先在设置中保存可用的模型与 API Key".to_string());
    }

    let kb = KnowledgeBase::new(&vault_path);
    kb.discard_concept_note(&object_id).map_err(|e| e.to_string())?;
    enqueue_concept_note_for_object(&vault_path, &object_id)?;
    Ok(())
}

/// 判断 AI 返回内容是否为"内容不足"类拒答句（不能作为摘要保存）。
fn is_concept_note_refusal(note: &str) -> bool {
    let t = note.trim();
    if t.is_empty() {
        return true;
    }
    const REFUSAL_MARKERS: [&str; 4] = [
        "无法生成摘要",
        "文件内容不足",
        "内容不足",
        "没有可提取",
    ];
    // 拒答句通常很短（< 40 字）且包含标记词
    t.chars().count() < 40 && REFUSAL_MARKERS.iter().any(|m| t.contains(m))
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
    conversation_context: Option<Vec<ai::ConversationTurn>>,
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
    let conversation_context = conversation_context.unwrap_or_default();
    let cancellations = Arc::clone(&state.cancellations);
    tauri::async_runtime::spawn(async move {
        let vault_path = resolve_vault_path();
        let action_app = app.clone();
        let stream_app = app.clone();
        let action_task_id = spawned_task_id.clone();
        let stream_task_id = spawned_task_id.clone();
        let outcome = ai::organize_with_agent(
            &vault_path,
            &spawned_task_id,
            collection_id.as_deref(),
            &instruction,
            &conversation_context,
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
            move |description| {
                let _ = stream_app.emit(
                    "ai-task-event",
                    AiTaskEvent {
                        task_id: stream_task_id.clone(),
                        kind: "stream".to_string(),
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
fn delete_ai_task(task_id: String) -> Result<(), String> {
    let deleted = get_kb().delete_ai_task(&task_id).map_err(|e| e.to_string())?;
    if deleted {
        Ok(())
    } else {
        Err("任务不存在或正在运行，无法删除".to_string())
    }
}

#[tauri::command]
fn clear_ai_tasks() -> Result<usize, String> {
    get_kb()
        .clear_finished_ai_tasks()
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
async fn import_url(url: String, group: String) -> Result<Object, String> {
    tauri::async_runtime::spawn_blocking(move || {
        get_kb()
            .ingest_url(&url, Some(&group))
            .map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| format!("网页入库任务失败: {error}"))?
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
async fn search_objects(query: String, limit: Option<i64>) -> Result<Vec<ObjectSummary>, String> {
    ai::hybrid_search(&resolve_vault_path(), &query, limit.unwrap_or(50)).await
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
fn sync_external_changes() -> Result<SyncReport, String> {
    get_kb()
        .sync_external_changes()
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn get_object_versions(id: String, limit: Option<i64>) -> Result<Vec<ObjectVersion>, String> {
    get_kb()
        .get_object_versions(&id, limit.unwrap_or(30))
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn restore_object_version(id: String, operation_id: i64) -> Result<(), String> {
    get_kb()
        .restore_object_version(&id, operation_id)
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn ocr_object(id: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        get_kb().ocr_object(&id).map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| format!("OCR 任务失败: {error}"))?
}

#[tauri::command]
fn get_operation_log(limit: Option<i64>) -> Result<Vec<serde_json::Value>, String> {
    let kb = get_kb();
    let conn = kb.conn().map_err(|e| e.to_string())?;
    db::get_recent_operations(&conn, limit.unwrap_or(50)).map_err(|e| e.to_string())
}

#[tauri::command]
fn undo_last_operation() -> Result<String, String> {
    get_kb()
        .undo_last_operation()
        .map_err(|error| error.to_string())
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

// ---------------------------------------------------------------------------
// Agent 主动接入：检测各 AI 客户端配置 + 一键写入 kb-mcp
// ---------------------------------------------------------------------------

/// 单个 AI Agent 的接入状态。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentStatus {
    /// 稳定标识，link_agent 入参用
    id: &'static str,
    /// 展示名
    name: &'static str,
    /// 配置格式：json / toml
    format: &'static str,
    /// 配置文件绝对路径（可能尚不存在）
    config_path: String,
    /// 配置文件是否已存在（代表该客户端已在本机初始化）
    config_exists: bool,
    /// 是否已注册 KBOX 的 MCP server
    registered: bool,
}

/// 支持的 AI 客户端清单。新增客户端只需在此加一行 + 对应 home 相对路径。
fn agent_defs() -> Vec<(&'static str, &'static str, &'static str, &'static str)> {
    // (id, 名称, 配置格式, 相对 home 的路径)
    vec![
        ("codex", "Codex CLI", "toml", ".codex/config.toml"),
        (
            "claude",
            "Claude Desktop",
            "json",
            "Library/Application Support/Claude/claude_desktop_config.json",
        ),
        ("cursor", "Cursor", "json", ".cursor/mcp.json"),
        (
            "windsurf",
            "Windsurf",
            "json",
            ".codeium/windsurf/mcp_config.json",
        ),
        (
            "cline",
            "Cline (VS Code)",
            "json",
            "Library/Application Support/Code/User/globalStorage/saoudrizwan.claude-dev/settings/cline_mcp_settings.json",
        ),
        ("continue", "Continue", "json", ".continue/config.json"),
    ]
}

/// kb-mcp 可执行文件路径：与主程序同目录（开发期为 target/debug，安装期为 .app/Contents/MacOS）。
fn resolve_mcp_binary() -> Result<PathBuf, String> {
    let path = std::env::current_exe()
        .map_err(|e| e.to_string())?
        .parent()
        .ok_or("无法获取可执行文件目录")?
        .join("kb-mcp");
    Ok(path)
}

fn agent_config_path(rel: &str) -> PathBuf {
    dirs::home_dir().unwrap_or_default().join(rel)
}

/// 判断配置内容里是否已注册 KBOX MCP server。
fn config_is_registered(format: &str, text: &str) -> bool {
    match format {
        "toml" => {
            text.contains("[mcp_servers.knowledge-base]")
                || text.contains("[mcp_servers.kbox]")
                || text.contains("kb-mcp")
        }
        _json => {
            serde_json::from_str::<serde_json::Value>(text)
                .ok()
                .and_then(|value| {
                    value
                        .get("mcpServers")
                        .and_then(|servers| servers.as_object())
                        .map(|servers| {
                            servers.contains_key("knowledge-base")
                                || servers.contains_key("kbox")
                                || servers
                                    .values()
                                    .any(|server| server["command"].as_str().is_some_and(|cmd| cmd.contains("kb-mcp")))
                        })
                })
                .unwrap_or(false)
        }
    }
}

#[tauri::command]
fn detect_agents() -> Vec<AgentStatus> {
    agent_defs()
        .into_iter()
        .map(|(id, name, format, rel)| {
            let path = agent_config_path(rel);
            let exists = path.exists();
            let registered = exists
                && std::fs::read_to_string(&path)
                    .map(|text| config_is_registered(format, &text))
                    .unwrap_or(false);
            AgentStatus {
                id,
                name,
                format,
                config_path: path.to_string_lossy().to_string(),
                config_exists: exists,
                registered,
            }
        })
        .collect()
}

/// 生成要写入的 JSON mcpServers.knowledge-base 条目。
fn mcp_json_entry() -> Result<serde_json::Value, String> {
    let binary = resolve_mcp_binary()?;
    let vault = resolve_vault_path();
    Ok(serde_json::json!({
        "command": binary.to_string_lossy(),
        "args": [],
        "env": { "KB_VAULT_PATH": vault.to_string_lossy() }
    }))
}

/// 生成要写入的 TOML 块（Codex config.toml）。
fn mcp_toml_block() -> Result<String, String> {
    let binary = resolve_mcp_binary()?.to_string_lossy().replace('\\', "\\\\").replace('"', "\\\"");
    let vault = resolve_vault_path()
        .to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"");
    Ok(format!(
        "[mcp_servers.knowledge-base]\ncommand = \"{binary}\"\nenv = {{ \"KB_VAULT_PATH\" = \"{vault}\" }}\n"
    ))
}

/// 写入前备份原配置（仅当文件存在），返回备份路径。
fn backup_config(path: &Path) -> Result<Option<PathBuf>, String> {
    if !path.exists() {
        return Ok(None);
    }
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let backup = path.with_file_name(format!(
        "{}.bak.{}",
        path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(),
        ts
    ));
    std::fs::copy(path, &backup).map_err(|e| format!("备份配置失败: {e}"))?;
    Ok(Some(backup))
}

/// 原子写入：先写 .tmp 再 rename。
fn atomic_write(path: &Path, content: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("创建配置目录失败: {e}"))?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, content).map_err(|e| format!("写入配置失败: {e}"))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("保存配置失败: {e}"))?;
    Ok(())
}

/// 合并写入 JSON 配置（保留用户已有的其他 mcp server）。
fn write_json_config(path: &Path) -> Result<(), String> {
    let mut value: serde_json::Value = match std::fs::read_to_string(path) {
        Ok(text) if text.trim().is_empty() => serde_json::json!({}),
        Ok(text) => serde_json::from_str(&text).map_err(|e| format!("配置 JSON 解析失败: {e}"))?,
        Err(_) => serde_json::json!({}),
    };
    if !value.is_object() {
        return Err("配置文件根节点不是对象，已中止以免覆盖".to_string());
    }
    let entry = mcp_json_entry()?;
    value
        .as_object_mut()
        .unwrap()
        .entry("mcpServers".to_string())
        .or_insert_with(|| serde_json::json!({}))
        .as_object_mut()
        .ok_or("mcpServers 不是对象")?
        .insert("knowledge-base".to_string(), entry);
    let pretty = serde_json::to_string_pretty(&value).unwrap_or_default();
    atomic_write(path, &pretty)
}

/// 合并 TOML 文本：已有 knowledge-base/kbox 表块则原地替换，否则追加到末尾。
/// 不依赖 toml crate，仅按表声明行切分，保证保留用户其他配置。
fn merge_toml_block(existing: &str, block: &str) -> String {
    let lines: Vec<&str> = existing.lines().collect();

    let start = lines.iter().position(|line| {
        let t = line.trim();
        t == "[mcp_servers.knowledge-base]" || t == "[mcp_servers.kbox]"
    });

    let merged = if let Some(start_idx) = start {
        // 块结束：下一个以 '[' 开头的表声明行
        let end_idx = lines
            .iter()
            .enumerate()
            .skip(start_idx + 1)
            .find(|(_, line)| line.trim_start().starts_with('['))
            .map(|(i, _)| i)
            .unwrap_or(lines.len());
        let mut kept: Vec<&str> = lines[..start_idx].to_vec();
        kept.push(block.trim_end());
        kept.extend_from_slice(&lines[end_idx..]);
        kept.join("\n")
    } else if existing.trim().is_empty() {
        block.to_string()
    } else {
        format!("{}\n{}", existing.trim_end(), block)
    };

    merged.trim().to_string()
}

/// 合并写入 TOML 配置（不引入 toml 依赖，按表块做标记替换/追加）。
fn write_toml_config(path: &Path) -> Result<(), String> {
    let block = mcp_toml_block()?;
    let text = std::fs::read_to_string(path).unwrap_or_default();
    atomic_write(path, &merge_toml_block(&text, &block))
}

#[tauri::command]
fn link_agent(id: String) -> Result<AgentStatus, String> {
    let (agent_id, name, format, rel) = agent_defs()
        .into_iter()
        .find(|(agent_id, _, _, _)| *agent_id == id.as_str())
        .ok_or_else(|| format!("未知的 Agent: {id}"))?;

    let path = agent_config_path(rel);

    // 写入前确认 kb-mcp 二进制存在，避免配置生效后客户端拉起失败
    let binary = resolve_mcp_binary()?;
    if !binary.exists() {
        return Err(format!(
            "未找到 kb-mcp 可执行文件（{}），请先构建或重新安装 KBOX",
            binary.display()
        ));
    }

    let backup = backup_config(&path)?;

    match format {
        "toml" => write_toml_config(&path)?,
        _ => write_json_config(&path)?,
    }

    // 重新读取确认真的注册成功
    let text = std::fs::read_to_string(&path).map_err(|e| format!("回读配置失败: {e}"))?;
    if !config_is_registered(format, &text) {
        return Err("配置已写入但校验未通过，请手动检查".to_string());
    }

    let _ = backup; // 备份已落盘，路径仅用于将来在 UI 展示
    Ok(AgentStatus {
        id: agent_id,
        name,
        format,
        config_path: path.to_string_lossy().to_string(),
        config_exists: true,
        registered: true,
    })
}

#[tauri::command]
fn import_file(file_path: String, group: Option<String>, app: tauri::AppHandle) -> Result<serde_json::Value, String> {
    let kb = get_kb();
    let path = std::path::Path::new(&file_path);
    match kb.ingest_file(path, group.as_deref()) {
        Ok(obj) => {
            // 入库成功后，仅在“自动生成”开关开启时自动入队影子笔记
            let enabled = app
                .state::<ConceptNoteState>()
                .enabled
                .load(Ordering::Relaxed);
            if enabled {
                let vault_path = resolve_vault_path();
                let _ = enqueue_concept_note_for_object(&vault_path, &obj.id);
            }
            serde_json::to_value(&obj).map_err(|e| e.to_string())
        }
        Err(e) => Err(e.to_string()),
    }
}

/// 解析 `data:<mime>[;params];base64,<payload>` 为 (mime, bytes)；也接受裸 base64。
fn parse_data_url(data_url: &str) -> Result<(String, Vec<u8>), String> {
    if let Some(rest) = data_url.strip_prefix("data:") {
        let (meta, b64_part) = rest
            .split_once(";base64,")
            .ok_or("data URL 缺少 base64 段".to_string())?;
        let mime = meta.split(';').next().unwrap_or("").to_ascii_lowercase();
        Ok((mime, decode_base64_buf(b64_part)?))
    } else {
        Ok(("image/png".to_string(), decode_base64_buf(data_url)?))
    }
}

fn decode_base64_buf(input: &str) -> Result<Vec<u8>, String> {
    let cleaned: Vec<u8> = input.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    let mut out = Vec::with_capacity(cleaned.len() / 4 * 3 + 3);
    for chunk in cleaned.chunks(4) {
        let mut vals = [0u32; 4];
        let mut n = 0usize;
        for (idx, &c) in chunk.iter().enumerate() {
            let v = match c {
                b'A'..=b'Z' => (c - b'A') as u32,
                b'a'..=b'z' => (c - b'a' + 26) as u32,
                b'0'..=b'9' => (c - b'0' + 52) as u32,
                b'+' => 62,
                b'/' => 63,
                b'=' => break,
                _ => return Err(format!("非法的 Base64 字符: {}", c as char)),
            };
            vals[idx] = v;
            n += 1;
        }
        if n == 0 {
            break;
        }
        let triple = (vals[0] << 18) | (vals[1] << 12) | (vals[2] << 6) | vals[3];
        out.push((triple >> 16) as u8);
        if n > 2 {
            out.push((triple >> 8) as u8);
        }
        if n > 3 {
            out.push(triple as u8);
        }
    }
    Ok(out)
}

fn ext_from_mime(mime: &str) -> &'static str {
    match mime {
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/bmp" => "bmp",
        "image/svg+xml" => "svg",
        "image/tiff" | "image/tif" => "tif",
        _ => "png",
    }
}

fn paste_image_file_name(file_name: Option<&str>, ext: &str) -> String {
    let stem = file_name
        .and_then(|name| Path::new(name).file_stem())
        .and_then(|stem| stem.to_str())
        .map(|stem| {
            stem.chars()
                .filter(|character| {
                    character.is_alphanumeric()
                        || matches!(character, '-' | '_' | ' ' | '·' | '一'..='鿿')
                })
                .take(80)
                .collect::<String>()
        })
        .filter(|stem| !stem.trim().is_empty())
        .unwrap_or_else(|| "粘贴图片".to_string());
    format!("{}.{}", stem.trim(), ext)
}

fn paste_image_result(object: &Object, fallback_ext: &str) -> PasteImageResult {
    let format_detail = object
        .primary_file
        .as_deref()
        .and_then(|path| Path::new(path).extension())
        .and_then(|extension| extension.to_str())
        .unwrap_or(fallback_ext)
        .to_string();
    PasteImageResult {
        id: object.id.clone(),
        title: object.title.clone(),
        format: object.format.clone(),
        format_detail,
    }
}

fn import_image_bytes_into(
    kb: &KnowledgeBase,
    data_url: &str,
    group: Option<&str>,
    file_name: Option<&str>,
) -> Result<PasteImageResult, String> {
    let (mime, bytes) = parse_data_url(data_url)?;
    if !mime.starts_with("image/") {
        return Err("剪贴板内容不是图片".to_string());
    }
    if bytes.is_empty() {
        return Err("剪贴板图片为空".to_string());
    }
    if bytes.len() > 20 * 1024 * 1024 {
        return Err("图片超过 20 MB，无法粘贴".to_string());
    }
    let ext = ext_from_mime(&mime);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let tmp_dir = std::env::temp_dir()
        .join("kbox-paste")
        .join(format!("paste-{nanos}"));
    std::fs::create_dir_all(&tmp_dir).map_err(|error| error.to_string())?;
    let tmp_path = tmp_dir.join(paste_image_file_name(file_name, ext));
    std::fs::write(&tmp_path, &bytes).map_err(|error| error.to_string())?;
    let ingest_result = kb.ingest_file_as(&tmp_path, group, "human");
    let _ = std::fs::remove_dir_all(&tmp_dir);

    let object = match ingest_result {
        Ok(object) => object,
        Err(kb_core::KbError::AlreadyExists(object_id)) => {
            let (mut object, _, _) = kb
                .get_object_detail(&object_id)
                .map_err(|error| error.to_string())?;
            if object.format != "image" {
                return Err(format!("相同内容已存在，但对象 {object_id} 不是图片"));
            }
            if object.status == "deleted" {
                kb.restore_object(&object_id)
                    .map_err(|error| error.to_string())?;
                object.status = "draft".to_string();
            }
            object
        }
        Err(error) => return Err(error.to_string()),
    };
    Ok(paste_image_result(&object, ext))
}

/// 将粘贴的图片字节保存为库内图片对象，返回其 id 等信息供编辑器引用。
#[tauri::command]
fn import_image_bytes(
    data_url: String,
    group: Option<String>,
    file_name: Option<String>,
) -> Result<PasteImageResult, String> {
    import_image_bytes_into(&get_kb(), &data_url, group.as_deref(), file_name.as_deref())
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
async fn scan_discovery_folders(folders: Vec<String>) -> Result<Vec<DiscoveryCandidate>, String> {
    tauri::async_runtime::spawn_blocking(move || scan_discovery_folders_blocking(folders))
        .await
        .map_err(|error| error.to_string())?
}

fn scan_discovery_folders_blocking(
    folders: Vec<String>,
) -> Result<Vec<DiscoveryCandidate>, String> {
    let _guard = DISCOVERY_STATE_LOCK
        .lock()
        .map_err(|error| error.to_string())?;
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
async fn import_discovery_files(paths: Vec<String>, group: String) -> Result<usize, String> {
    tauri::async_runtime::spawn_blocking(move || import_discovery_files_blocking(paths, group))
        .await
        .map_err(|error| error.to_string())?
}

fn import_discovery_files_blocking(paths: Vec<String>, group: String) -> Result<usize, String> {
    let _guard = DISCOVERY_STATE_LOCK
        .lock()
        .map_err(|error| error.to_string())?;
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
async fn ignore_discovery_files(paths: Vec<String>) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || ignore_discovery_files_blocking(paths))
        .await
        .map_err(|error| error.to_string())?
}

fn ignore_discovery_files_blocking(paths: Vec<String>) -> Result<(), String> {
    let _guard = DISCOVERY_STATE_LOCK
        .lock()
        .map_err(|error| error.to_string())?;
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

#[tauri::command]
fn open_asset(object_id: String) -> Result<(), String> {
    let path = get_kb()
        .get_asset_path(&object_id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "对象没有可打开的原文件".to_string())?;

    #[cfg(target_os = "macos")]
    let mut command = std::process::Command::new("open");
    #[cfg(target_os = "windows")]
    let mut command = {
        let mut command = std::process::Command::new("cmd");
        command.args(["/C", "start", ""]);
        command
    };
    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    let mut command = std::process::Command::new("xdg-open");

    command
        .arg(path)
        .spawn()
        .map_err(|error| format!("打开原文件失败: {error}"))?;
    Ok(())
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
            import_url,
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
            detect_agents,
            link_agent,
            import_file,
            import_folder,
            import_image_bytes,
            scan_discovery_folders,
            import_discovery_files,
            ignore_discovery_files,
            get_asset_url,
            open_asset,
            get_asset_base64,
            auto_link,
            rebuild_relations,
            get_relations_recursive,
            check_consistency,
            backup_db,
            rebuild_vault,
            sync_external_changes,
            get_operation_log,
            undo_last_operation,
            get_object_versions,
            restore_object_version,
            ocr_object,
            get_vault_path,
            get_ai_config,
            save_ai_config,
            test_ai_connection,
            ai_edit_text,
            start_ai_organize,
            cancel_ai_task,
            get_ai_tasks,
            get_ai_task_actions,
            delete_ai_task,
            clear_ai_tasks,
            reset_vault,
            // 影子笔记
            start_concept_note,
            start_concept_note_batch,
            get_concept_note_status,
            set_concept_note_paused,
            set_concept_note_enabled,
            retry_all_concept_notes,
            retry_concept_notes,
            check_concept_note,
            regenerate_concept_note,
        ])
        .setup(|app| {
            // 注册影子笔记状态（从磁盘恢复开关与暂停状态）
            let persisted = load_concept_note_persisted();
            app.manage(ConceptNoteState {
                paused: Arc::new(AtomicBool::new(persisted.paused)),
                enabled: Arc::new(AtomicBool::new(persisted.enabled)),
            });

            // 初始化知识库目录（失败不阻止启动）
            let vault_path = resolve_vault_path();
            let kb = KnowledgeBase::new(&vault_path);
            match kb.init_storage() {
                Ok(()) => {
                    println!("知识库已就绪: {}", vault_path.display());
                    spawn_vault_watcher(app.handle().clone(), vault_path.clone());
                    // 任务恢复会额外访问数据库，放到后台执行，尽快让 Tauri 窗口显示。
                    let recovery_path = vault_path.clone();
                    let recovery_app = app.handle().clone();
                    std::thread::spawn(move || {
                        let recovery_kb = KnowledgeBase::new(&recovery_path);
                        if let Err(error) = recovery_kb.recover_running_ai_tasks() {
                            eprintln!("AI 任务恢复失败: {error}");
                        }
                        if let Ok(conn) = recovery_kb.conn() {
                            if let Err(error) = db::recover_concept_notes(&conn) {
                                eprintln!("影子笔记队列恢复失败: {error}");
                            }
                        }
                        spawn_concept_note_worker(recovery_app, recovery_path);
                    });
                    std::thread::spawn(move || kb.run_startup_maintenance());
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

#[cfg(test)]
mod tests {
    use super::{
        decode_base64_buf, ext_from_mime, import_image_bytes_into, merge_toml_block,
        parse_data_url, paste_image_file_name, KnowledgeBase,
    };

    #[test]
    fn toml_merge_creates_appends_and_replaces_block() {
        let block = "[mcp_servers.knowledge-base]\ncommand = \"/tmp/kb-mcp\"\nenv = {}";

        // 1) 空配置：直接写入块
        let created = merge_toml_block("", block);
        assert!(created.contains("[mcp_servers.knowledge-base]"));
        assert!(created.contains("command = \"/tmp/kb-mcp\""));

        // 2) 已有其他配置：追加且保留原有内容
        let existing = "model = \"gpt\"\n\n[other]\nfoo = 1\n";
        let appended = merge_toml_block(existing, block);
        assert!(appended.contains("model = \"gpt\""));
        assert!(appended.contains("[other]"));
        assert!(appended.contains("foo = 1"));
        assert!(appended.contains("[mcp_servers.knowledge-base]"));

        // 3) 已存在 knowledge-base 块：原地替换，不产生重复表
        let stale = [
            "model = \"gpt\"",
            "",
            "[mcp_servers.knowledge-base]",
            "command = \"/old/kb-mcp\"",
            "",
            "[mcp_servers.other_server]",
            "command = \"/x\"",
        ]
        .join("\n");
        let replaced = merge_toml_block(&stale, block);
        assert_eq!(
            replaced.matches("[mcp_servers.knowledge-base]").count(),
            1,
            "替换后不应出现重复表: {replaced}"
        );
        assert!(!replaced.contains("/old/kb-mcp"));
        assert!(replaced.contains("command = \"/tmp/kb-mcp\""));
        // 后面的其他 server 必须保留
        assert!(replaced.contains("[mcp_servers.other_server]"));
        assert!(replaced.contains("command = \"/x\""));
        // 块之前的内容保留
        assert!(replaced.contains("model = \"gpt\""));
    }

    #[test]
    fn decode_base64_handles_simple_and_padded() {
        assert_eq!(decode_base64_buf("aGVsbG8=").unwrap(), b"hello");
        assert_eq!(decode_base64_buf("aGVsbG8").unwrap(), b"hello");
        assert_eq!(decode_base64_buf("aGk=").unwrap(), b"hi");
        // 1x1 png 的 data URL base64 载荷
        let b = decode_base64_buf("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+M8AAAMBAQDJ/pLvAAAAAElFTkSuQmCC").unwrap();
        assert!(!b.is_empty());
    }

    #[test]
    fn parse_data_url_extracts_mime_and_bytes() {
        let (mime, bytes) = parse_data_url("data:image/png;base64,aGk=").unwrap();
        assert_eq!(mime, "image/png");
        assert_eq!(bytes, b"hi");
    }

    #[test]
    fn parse_data_url_accepts_raw_base64() {
        let (mime, bytes) = parse_data_url("aGk=").unwrap();
        assert_eq!(mime, "image/png");
        assert_eq!(bytes, b"hi");
    }

    #[test]
    fn ext_from_mime_maps_common_types() {
        assert_eq!(ext_from_mime("image/png"), "png");
        assert_eq!(ext_from_mime("image/jpeg"), "jpg");
        assert_eq!(ext_from_mime("image/svg+xml"), "svg");
        assert_eq!(ext_from_mime("unknown/application"), "png");
    }

    #[test]
    fn pasted_image_name_is_sanitized_and_uses_mime_extension() {
        assert_eq!(
            paste_image_file_name(Some("../../产品 截图.jpeg"), "png"),
            "产品 截图.png"
        );
        assert_eq!(paste_image_file_name(None, "webp"), "粘贴图片.webp");
    }

    #[test]
    fn repeated_image_paste_reuses_existing_object_and_restores_it() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let vault = std::env::temp_dir().join(format!("kbox-paste-test-{unique}"));
        let kb = KnowledgeBase::new(&vault);
        kb.init().expect("init");
        let data_url = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+M8AAAMBAQDJ/pLvAAAAAElFTkSuQmCC";

        let first = import_image_bytes_into(&kb, data_url, Some("测试"), Some("产品截图.png"))
            .expect("first paste");
        let repeated = import_image_bytes_into(&kb, data_url, Some("测试"), Some("另一个名称.png"))
            .expect("repeated paste");
        assert_eq!(repeated.id, first.id);
        assert_eq!(repeated.title, first.title);

        kb.delete_object(&first.id).expect("delete");
        let restored =
            import_image_bytes_into(&kb, data_url, Some("测试"), None).expect("restore by paste");
        assert_eq!(restored.id, first.id);
        assert_ne!(
            kb.get_object_detail(&first.id).expect("detail").0.status,
            "deleted"
        );
        drop(kb);
        std::fs::remove_dir_all(&vault).expect("cleanup");
    }
}
