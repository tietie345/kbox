use std::io::Read;
use std::path::{Path, PathBuf};

use chrono::DateTime;
use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;
use rusqlite::{params, Connection};
use sha2::{Digest, Sha256};
use zip::ZipArchive;

use crate::db;
use crate::error::{KbError, Result};
use crate::model::{
    AiTask, AiTaskAction, CollectionNode, GroupNode, Object, ObjectSummary, Relation, VaultStats,
};

pub struct KnowledgeBase {
    pub vault_path: PathBuf,
    pub db_path: PathBuf,
}

impl KnowledgeBase {
    pub fn new(vault_path: &Path) -> Self {
        let db_path = vault_path.join(".kb").join("index.db");
        Self {
            vault_path: vault_path.to_path_buf(),
            db_path,
        }
    }

    pub fn init(&self) -> Result<()> {
        std::fs::create_dir_all(self.vault_path.join(".kb"))?;
        std::fs::create_dir_all(self.vault_path.join("objects"))?;

        // 获取文件锁（超时 5 秒）
        let _lock = crate::lock::FileLock::acquire(&self.vault_path, 5)?;

        // 打开数据库并初始化 schema
        let conn = db::open(&self.db_path)?;
        db::init_schema(&conn)?;

        // 启动时自动备份（每天第一次启动备份）
        let _ = self.backup_database();

        // 启动时一致性检查（记录警告但不阻止启动）
        let _ = self.check_consistency();

        // 清理过期操作日志
        let _ = db::cleanup_operation_log(&conn);

        Ok(())
    }

    pub fn conn(&self) -> Result<rusqlite::Connection> {
        db::open(&self.db_path)
    }

    // 生成对象 ID：obj_YYYYMMDD_xxxxxx
    fn generate_id() -> String {
        let dur = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or(std::time::Duration::ZERO);
        let secs = dur.as_secs();
        let dt = DateTime::from_timestamp(secs as i64, 0)
            .unwrap_or_else(|| DateTime::from_timestamp(0, 0).expect("valid epoch"));
        let date = dt.format("%Y%m%d").to_string();
        // 使用纳秒作为简单熵源生成 6 字节 hex（12 个字符）
        let nanos = dur.subsec_nanos() as u128;
        // 混合一个静态计数器避免同纳秒冲突（无锁，简化处理）
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let c = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let mixed = (nanos << 20) ^ (c as u128).wrapping_mul(0x9E3779B97F4A7C15);
        let hex = format!("{:012x}", mixed & 0xFFFF_FFFF_FFFF);
        format!("obj_{}_{}", date, hex)
    }

    // 计算 sha256
    fn file_hash(path: &Path) -> Result<String> {
        let mut hasher = Sha256::new();
        let mut file = std::fs::File::open(path)?;
        std::io::copy(&mut file, &mut hasher)?;
        Ok(format!("sha256:{}", hex::encode(hasher.finalize())))
    }

    fn file_display_name(path: &Path, fallback: &str) -> String {
        path.file_name()
            .and_then(|s| s.to_str())
            .filter(|s| !s.trim().is_empty())
            .unwrap_or(fallback)
            .to_string()
    }

    fn now_secs() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
    }

    fn atomic_write(path: &Path, content: &[u8]) -> Result<()> {
        let parent = path
            .parent()
            .ok_or_else(|| KbError::ParseFailed("目标文件没有父目录".to_string()))?;
        std::fs::create_dir_all(parent)?;
        let temp_name = format!(
            ".{}.{}.tmp",
            path.file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("kb"),
            std::process::id()
        );
        let temp_path = parent.join(temp_name);
        {
            use std::io::Write;
            let mut file = std::fs::File::create(&temp_path)?;
            file.write_all(content)?;
            file.sync_all()?;
        }
        if let Err(error) = std::fs::rename(&temp_path, path) {
            let _ = std::fs::remove_file(&temp_path);
            return Err(KbError::Io(error));
        }
        Ok(())
    }

    fn yaml_string(value: &str) -> String {
        serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_string())
    }

    fn render_index(obj: &Object, tags: &[String], body: &str) -> String {
        let primary_file = obj.primary_file.as_deref().unwrap_or("");
        let file_hash = obj.file_hash.as_deref().unwrap_or("");
        let tags = serde_json::to_string(tags).unwrap_or_else(|_| "[]".to_string());
        format!(
            "---\nid: {}\ntype: {}\nformat: {}\ntitle: {}\ncreated: {}\nupdated_at: {}\ncreated_by: {}\nstatus: {}\ngroup: {}\nprimary_file: {}\nfile_hash: {}\nfile_size: {}\ntags: {}\nmetadata: {}\n---\n\n{}",
            Self::yaml_string(&obj.id),
            Self::yaml_string(&obj.r#type),
            Self::yaml_string(&obj.format),
            Self::yaml_string(&obj.title),
            obj.created_at,
            obj.updated_at,
            Self::yaml_string(&obj.created_by),
            Self::yaml_string(&obj.status),
            Self::yaml_string(&obj.group_name),
            Self::yaml_string(primary_file),
            Self::yaml_string(file_hash),
            obj.file_size,
            tags,
            obj.metadata,
            body
        )
    }

    fn index_body(content: &str) -> &str {
        if !content.starts_with("---") {
            return content;
        }
        content[3..]
            .find("---")
            .map(|end| content[end + 6..].trim_start_matches(['\r', '\n']))
            .unwrap_or("")
    }

    fn read_index_body(&self, id: &str) -> Result<String> {
        let index_path = self.vault_path.join("objects").join(id).join("index.md");
        let content = std::fs::read_to_string(index_path)?;
        Ok(Self::index_body(&content).to_string())
    }

    fn write_index(&self, obj: &Object, tags: &[String], body: &str) -> Result<()> {
        let index_path = self
            .vault_path
            .join("objects")
            .join(&obj.id)
            .join("index.md");
        Self::atomic_write(&index_path, Self::render_index(obj, tags, body).as_bytes())
    }

    fn persist_new_object(
        conn: &mut Connection,
        obj: &Object,
        searchable_content: &str,
        obj_dir: &Path,
    ) -> Result<()> {
        let result = (|| -> Result<()> {
            let tx = conn.transaction()?;
            db::create_object(&tx, obj)?;
            tx.execute(
                "UPDATE objects_fts SET content = ?1 WHERE id = ?2",
                params![searchable_content, obj.id],
            )?;
            tx.commit()?;
            Ok(())
        })();
        if result.is_err() {
            let _ = std::fs::remove_dir_all(obj_dir);
        }
        result
    }

    // 入库 MD 文件
    pub fn ingest_markdown(&self, file_path: &Path, group: Option<&str>) -> Result<Object> {
        let _lock = crate::lock::FileLock::acquire(&self.vault_path, 5)?;
        let content = std::fs::read_to_string(file_path)?;
        let hash = Self::file_hash(file_path)?;

        // 检查去重
        let mut conn = self.conn()?;
        let existing: Option<String> = conn
            .query_row(
                "SELECT id FROM objects WHERE file_hash = ?1",
                params![hash],
                |row| row.get(0),
            )
            .ok();
        if let Some(id) = existing {
            return Err(KbError::AlreadyExists(id));
        }

        let id = Self::generate_id();
        let title = Self::file_display_name(file_path, "untitled.md");

        let file_size = std::fs::metadata(file_path)?.len() as i64;
        let now = Self::now_secs();
        let group_name = group.unwrap_or("收件箱").to_string();

        // 创建对象目录
        let obj_dir = self.vault_path.join("objects").join(&id);
        let assets_dir = obj_dir.join("assets");
        std::fs::create_dir_all(&assets_dir)?;

        // 复制原文件
        let file_name = file_path.file_name().unwrap_or_default();
        std::fs::copy(file_path, assets_dir.join(file_name))?;

        let file_name_str = file_name.to_str().unwrap_or("file.md");
        let primary_file = format!("assets/{}", file_name_str);

        // 创建 index.md
        let index_md = format!(
            "---\nid: {}\ntype: document\nformat: markdown\ntitle: {}\ncreated: {}\ncreated_by: human\nstatus: draft\ngroup: {}\nprimary_file: {}\nfile_hash: {}\nfile_size: {}\ntags: []\n---\n\n{}",
            id, title, now, group_name, primary_file, hash, file_size, content
        );
        std::fs::write(obj_dir.join("index.md"), &index_md)?;

        // 写入数据库
        let obj = Object {
            id: id.clone(),
            r#type: "document".to_string(),
            format: "markdown".to_string(),
            title: title.clone(),
            group_name: group_name.clone(),
            primary_file: Some(primary_file),
            file_hash: Some(hash),
            file_size,
            created_by: "human".to_string(),
            status: "draft".to_string(),
            metadata: "{}".to_string(),
            created_at: now,
            updated_at: now,
        };
        Self::persist_new_object(&mut conn, &obj, &content, &obj_dir)?;

        // 自动检测并建立关系
        let _ = self.auto_link_relations(&id);

        Ok(obj)
    }

    // 创建新笔记（空内容）
    pub fn create_note(&self, title: &str, obj_type: &str, group: &str) -> Result<Object> {
        let _lock = crate::lock::FileLock::acquire(&self.vault_path, 5)?;
        let id = Self::generate_id();
        let now = Self::now_secs();

        let obj_dir = self.vault_path.join("objects").join(&id);
        let assets_dir = obj_dir.join("assets");
        std::fs::create_dir_all(&assets_dir)?;

        // 过滤文件名非法字符
        let safe_title: String = title
            .chars()
            .map(|c| match c {
                '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '\\' => '_',
                _ => c,
            })
            .collect();
        let file_name = if safe_title.is_empty() {
            "untitled.md".to_string()
        } else {
            format!("{}.md", safe_title)
        };
        let content = format!("# {}\n\n", title);
        std::fs::write(assets_dir.join(&file_name), &content)?;

        let primary_file = format!("assets/{}", file_name);
        let index_md = format!(
            "---\nid: {}\ntype: {}\nformat: markdown\ntitle: {}\ncreated: {}\ncreated_by: human\nstatus: draft\ngroup: {}\nprimary_file: {}\nfile_hash: \nfile_size: {}\ntags: []\n---\n\n{}",
            id, obj_type, title, now, group, primary_file, content.len(), content
        );
        std::fs::write(obj_dir.join("index.md"), &index_md)?;

        let obj = Object {
            id: id.clone(),
            r#type: obj_type.to_string(),
            format: "markdown".to_string(),
            title: title.to_string(),
            group_name: group.to_string(),
            primary_file: Some(primary_file),
            file_hash: None,
            file_size: content.len() as i64,
            created_by: "human".to_string(),
            status: "draft".to_string(),
            metadata: "{}".to_string(),
            created_at: now,
            updated_at: now,
        };
        let mut conn = self.conn()?;
        Self::persist_new_object(&mut conn, &obj, &content, &obj_dir)?;

        // 自动检测并建立关系
        let _ = self.auto_link_relations(&id);

        Ok(obj)
    }

    /// 创建由指定来源产生的笔记。GUI 使用 human，MCP/Agent 使用 agent。
    pub fn create_note_as(
        &self,
        title: &str,
        obj_type: &str,
        group: &str,
        created_by: &str,
    ) -> Result<Object> {
        let obj = self.create_note(title, obj_type, group)?;
        self.set_object_creator(&obj.id, created_by)?;
        let mut updated = obj;
        updated.created_by = normalize_creator(created_by).to_string();
        Ok(updated)
    }

    // 读取对象内容
    pub fn read_object_content(&self, id: &str) -> Result<String> {
        let conn = self.conn()?;
        let obj = db::get_object(&conn, id)?.ok_or_else(|| KbError::NotFound(id.to_string()))?;

        // 对于文本格式，直接读取原文件
        match obj.format.as_str() {
            "markdown" | "code" => {
                let primary_file = obj.primary_file.unwrap_or_default();
                let path = self.vault_path.join("objects").join(id).join(&primary_file);
                std::fs::read_to_string(&path).map_err(KbError::Io)
            }
            // 非文本格式（pdf/image/attachment 等）：读取 index.md
            _ => {
                let index_path = self.vault_path.join("objects").join(id).join("index.md");
                std::fs::read_to_string(&index_path).map_err(KbError::Io)
            }
        }
    }

    /// 根据文件扩展名自动识别类型并入库
    pub fn ingest_file(&self, file_path: &Path, group: Option<&str>) -> Result<Object> {
        let ext = file_path
            .extension()
            .and_then(|s| s.to_str())
            .map(|s| s.to_lowercase())
            .unwrap_or_default();

        match ext.as_str() {
            // Markdown / 纯文本
            "md" | "markdown" | "txt" => self.ingest_markdown(file_path, group),
            // PDF
            "pdf" => self.ingest_pdf(file_path, group),
            // Office Open XML：保留原文件，并提取正文供全文检索
            "docx" | "xlsx" | "xlsm" => self.ingest_office(file_path, group, &ext),
            // 代码文件
            "rs" | "py" | "js" | "ts" | "tsx" | "jsx" | "go" | "java" | "c" | "cpp" | "h"
            | "hpp" | "cs" | "rb" | "php" | "swift" | "kt" | "scala" | "sh" | "bash" | "zsh"
            | "fish" | "sql" | "html" | "css" | "scss" | "less" | "vue" | "svelte" | "yaml"
            | "yml" | "toml" | "json" | "xml" | "ini" | "conf" | "cfg" | "lua" | "r" | "dart"
            | "elm" | "ex" | "exs" | "clj" | "cljs" | "hs" | "ml" | "fs" | "nim" | "v" | "zig"
            | "asm" | "s" => self.ingest_code(file_path, group, &ext),
            // 图片
            "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "svg" | "ico" | "tiff" | "tif" => {
                self.ingest_image(file_path, group, &ext)
            }
            // 其他：作为附件入库
            _ => self.ingest_attachment(file_path, group),
        }
    }

    /// 入库由指定来源提交的文件，并同步数据库与 index.md 的来源字段。
    pub fn ingest_file_as(
        &self,
        file_path: &Path,
        group: Option<&str>,
        created_by: &str,
    ) -> Result<Object> {
        let obj = self.ingest_file(file_path, group)?;
        self.set_object_creator(&obj.id, created_by)?;
        let mut updated = obj;
        updated.created_by = normalize_creator(created_by).to_string();
        Ok(updated)
    }

    // 入库 PDF 文件（保留原文件，并提取正文供全文检索）
    fn ingest_pdf(&self, file_path: &Path, group: Option<&str>) -> Result<Object> {
        let _lock = crate::lock::FileLock::acquire(&self.vault_path, 5)?;
        let hash = Self::file_hash(file_path)?;
        let mut conn = self.conn()?;

        let existing: Option<String> = conn
            .query_row(
                "SELECT id FROM objects WHERE file_hash = ?1",
                params![hash],
                |row| row.get(0),
            )
            .ok();
        if let Some(id) = existing {
            return Err(KbError::AlreadyExists(id));
        }

        let id = Self::generate_id();
        let title = Self::file_display_name(file_path, "untitled.pdf");
        let file_size = std::fs::metadata(file_path)?.len() as i64;
        let now = Self::now_secs();
        let group_name = group.unwrap_or("收件箱").to_string();

        let obj_dir = self.vault_path.join("objects").join(&id);
        let assets_dir = obj_dir.join("assets");
        std::fs::create_dir_all(&assets_dir)?;

        let file_name = file_path.file_name().unwrap_or_default();
        std::fs::copy(file_path, assets_dir.join(file_name))?;
        let file_name_str = file_name.to_str().unwrap_or("file.pdf");
        let primary_file = format!("assets/{}", file_name_str);

        let (content, extraction_status, extraction_error) =
            match pdf_extract::extract_text(file_path) {
                Ok(text) if !text.trim().is_empty() => (text, "ok", None),
                Ok(_) => (
                    format!(
                        "[PDF 文档: {}]\n\n未提取到可搜索文本；文件仍可正常预览。",
                        title
                    ),
                    "empty",
                    Some("PDF 不含文本层，可能是扫描件".to_string()),
                ),
                Err(error) => (
                    format!(
                        "[PDF 文档: {}]\n\n文本提取失败；原文件已安全保留，可正常预览。",
                        title
                    ),
                    "failed",
                    Some(error.to_string()),
                ),
            };

        let metadata = serde_json::json!({
            "pdf_pages": null,
            "pdf_size": file_size,
            "text_extraction": extraction_status,
            "text_extraction_error": extraction_error
        })
        .to_string();

        let index_md = format!(
            "---\nid: {}\ntype: document\nformat: pdf\ntitle: {}\ncreated: {}\ncreated_by: human\nstatus: draft\ngroup: {}\nprimary_file: {}\nfile_hash: {}\nfile_size: {}\ntags: []\n---\n\n{}",
            id, title, now, group_name, primary_file, hash, file_size, content
        );
        std::fs::write(obj_dir.join("index.md"), &index_md)?;

        let obj = Object {
            id: id.clone(),
            r#type: "document".to_string(),
            format: "pdf".to_string(),
            title,
            group_name,
            primary_file: Some(primary_file),
            file_hash: Some(hash),
            file_size,
            created_by: "human".to_string(),
            status: "draft".to_string(),
            metadata,
            created_at: now,
            updated_at: now,
        };
        Self::persist_new_object(&mut conn, &obj, &content, &obj_dir)?;

        // 自动检测并建立关系
        let _ = self.auto_link_relations(&id);

        Ok(obj)
    }

    fn ingest_office(&self, file_path: &Path, group: Option<&str>, ext: &str) -> Result<Object> {
        let _lock = crate::lock::FileLock::acquire(&self.vault_path, 5)?;
        let hash = Self::file_hash(file_path)?;
        let mut conn = self.conn()?;

        let existing: Option<String> = conn
            .query_row(
                "SELECT id FROM objects WHERE file_hash = ?1",
                params![hash],
                |row| row.get(0),
            )
            .ok();
        if let Some(id) = existing {
            return Err(KbError::AlreadyExists(id));
        }

        let id = Self::generate_id();
        let title = Self::file_display_name(file_path, "office-file");
        let file_size = std::fs::metadata(file_path)?.len() as i64;
        let now = Self::now_secs();
        let group_name = group.unwrap_or("收件箱").to_string();

        let obj_dir = self.vault_path.join("objects").join(&id);
        let assets_dir = obj_dir.join("assets");
        std::fs::create_dir_all(&assets_dir)?;

        let file_name = file_path.file_name().unwrap_or_default();
        std::fs::copy(file_path, assets_dir.join(file_name))?;
        let file_name_str = file_name.to_str().unwrap_or("office-file");
        let primary_file = format!("assets/{}", file_name_str);

        let extraction = match ext {
            "docx" => extract_docx_text(file_path),
            "xlsx" | "xlsm" => extract_xlsx_text(file_path),
            _ => Ok(String::new()),
        };
        let (content, extraction_status, extraction_error) = match extraction {
            Ok(text) if !text.trim().is_empty() => (text, "ok", None),
            Ok(_) => (
                format!(
                    "[Office 文档: {}]\n\n未提取到可搜索文本；原文件已安全保留。",
                    title
                ),
                "empty",
                Some("Office 文件中没有可识别文本".to_string()),
            ),
            Err(error) => (
                format!(
                    "[Office 文档: {}]\n\n文本提取失败；原文件已安全保留。",
                    title
                ),
                "failed",
                Some(error.to_string()),
            ),
        };

        let metadata = serde_json::json!({
            "format": ext,
            "text_extraction": extraction_status,
            "text_extraction_error": extraction_error
        })
        .to_string();

        let index_md = format!(
            "---\nid: {}\ntype: document\nformat: attachment\nformat_detail: {}\ntitle: {}\ncreated: {}\ncreated_by: human\nstatus: draft\ngroup: {}\nprimary_file: {}\nfile_hash: {}\nfile_size: {}\ntags: []\n---\n\n{}",
            id, ext, title, now, group_name, primary_file, hash, file_size, content
        );
        std::fs::write(obj_dir.join("index.md"), &index_md)?;

        let obj = Object {
            id: id.clone(),
            r#type: "document".to_string(),
            format: "attachment".to_string(),
            title,
            group_name,
            primary_file: Some(primary_file),
            file_hash: Some(hash),
            file_size,
            created_by: "human".to_string(),
            status: "draft".to_string(),
            metadata,
            created_at: now,
            updated_at: now,
        };
        Self::persist_new_object(&mut conn, &obj, &content, &obj_dir)?;

        let _ = self.auto_link_relations(&id);

        Ok(obj)
    }

    // 入库代码文件
    fn ingest_code(&self, file_path: &Path, group: Option<&str>, ext: &str) -> Result<Object> {
        let _lock = crate::lock::FileLock::acquire(&self.vault_path, 5)?;
        let content = std::fs::read_to_string(file_path)
            .map_err(|e| KbError::ParseFailed(format!("读取代码文件失败: {}", e)))?;
        let hash = Self::file_hash(file_path)?;
        let mut conn = self.conn()?;

        let existing: Option<String> = conn
            .query_row(
                "SELECT id FROM objects WHERE file_hash = ?1",
                params![hash],
                |row| row.get(0),
            )
            .ok();
        if let Some(id) = existing {
            return Err(KbError::AlreadyExists(id));
        }

        let id = Self::generate_id();
        let title = Self::file_display_name(file_path, "code.txt");
        let file_size = std::fs::metadata(file_path)?.len() as i64;
        let now = Self::now_secs();
        let group_name = group.unwrap_or("收件箱").to_string();

        let obj_dir = self.vault_path.join("objects").join(&id);
        let assets_dir = obj_dir.join("assets");
        std::fs::create_dir_all(&assets_dir)?;

        let file_name = file_path.file_name().unwrap_or_default();
        std::fs::copy(file_path, assets_dir.join(file_name))?;
        let file_name_str = file_name.to_str().unwrap_or("code.txt");
        let primary_file = format!("assets/{}", file_name_str);

        let language = ext_to_language(ext);
        let structure = extract_code_structure(&content, &language);
        let metadata = serde_json::json!({
            "language": language,
            "lines": content.lines().count(),
            "structure": structure
        })
        .to_string();

        let index_md = format!(
            "---\nid: {}\ntype: code\nformat: code\nformat_detail: {}\ntitle: {}\ncreated: {}\ncreated_by: human\nstatus: draft\ngroup: {}\nprimary_file: {}\nfile_hash: {}\nfile_size: {}\ntags: []\n---\n\n```{}\n{}\n```",
            id, language, title, now, group_name, primary_file, hash, file_size, language, content
        );
        std::fs::write(obj_dir.join("index.md"), &index_md)?;

        let obj = Object {
            id: id.clone(),
            r#type: "code".to_string(),
            format: "code".to_string(),
            title,
            group_name,
            primary_file: Some(primary_file),
            file_hash: Some(hash),
            file_size,
            created_by: "human".to_string(),
            status: "draft".to_string(),
            metadata,
            created_at: now,
            updated_at: now,
        };
        Self::persist_new_object(&mut conn, &obj, &content, &obj_dir)?;

        // 自动检测并建立关系
        let _ = self.auto_link_relations(&id);

        Ok(obj)
    }

    // 入库图片
    fn ingest_image(&self, file_path: &Path, group: Option<&str>, ext: &str) -> Result<Object> {
        let _lock = crate::lock::FileLock::acquire(&self.vault_path, 5)?;
        let hash = Self::file_hash(file_path)?;
        let mut conn = self.conn()?;

        let existing: Option<String> = conn
            .query_row(
                "SELECT id FROM objects WHERE file_hash = ?1",
                params![hash],
                |row| row.get(0),
            )
            .ok();
        if let Some(id) = existing {
            return Err(KbError::AlreadyExists(id));
        }

        let id = Self::generate_id();
        let title = Self::file_display_name(file_path, "image.png");
        let file_size = std::fs::metadata(file_path)?.len() as i64;
        let now = Self::now_secs();
        let group_name = group.unwrap_or("收件箱").to_string();

        let obj_dir = self.vault_path.join("objects").join(&id);
        let assets_dir = obj_dir.join("assets");
        std::fs::create_dir_all(&assets_dir)?;

        let file_name = file_path.file_name().unwrap_or_default();
        std::fs::copy(file_path, assets_dir.join(file_name))?;
        let file_name_str = file_name.to_str().unwrap_or("image.png");
        let primary_file = format!("assets/{}", file_name_str);

        let content = format!(
            "![{}]({})\n\n图片文件: {} | 格式: {} | 大小: {} 字节",
            title, file_name_str, title, ext, file_size
        );

        let metadata = serde_json::json!({
            "format": ext,
            "width": null,
            "height": null,
            "alt": title
        })
        .to_string();

        let index_md = format!(
            "---\nid: {}\ntype: image\nformat: image\nformat_detail: {}\ntitle: {}\ncreated: {}\ncreated_by: human\nstatus: draft\ngroup: {}\nprimary_file: {}\nfile_hash: {}\nfile_size: {}\ntags: []\n---\n\n{}",
            id, ext, title, now, group_name, primary_file, hash, file_size, content
        );
        std::fs::write(obj_dir.join("index.md"), &index_md)?;

        let obj = Object {
            id: id.clone(),
            r#type: "image".to_string(),
            format: "image".to_string(),
            title,
            group_name,
            primary_file: Some(primary_file),
            file_hash: Some(hash),
            file_size,
            created_by: "human".to_string(),
            status: "draft".to_string(),
            metadata,
            created_at: now,
            updated_at: now,
        };
        Self::persist_new_object(&mut conn, &obj, &content, &obj_dir)?;

        // 自动检测并建立关系
        let _ = self.auto_link_relations(&id);

        Ok(obj)
    }

    // 入库通用附件
    fn ingest_attachment(&self, file_path: &Path, group: Option<&str>) -> Result<Object> {
        let _lock = crate::lock::FileLock::acquire(&self.vault_path, 5)?;
        let hash = Self::file_hash(file_path)?;
        let mut conn = self.conn()?;

        let existing: Option<String> = conn
            .query_row(
                "SELECT id FROM objects WHERE file_hash = ?1",
                params![hash],
                |row| row.get(0),
            )
            .ok();
        if let Some(id) = existing {
            return Err(KbError::AlreadyExists(id));
        }

        let id = Self::generate_id();
        let title = Self::file_display_name(file_path, "file.bin");
        let file_size = std::fs::metadata(file_path)?.len() as i64;
        let now = Self::now_secs();
        let group_name = group.unwrap_or("收件箱").to_string();
        let ext = file_path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("bin")
            .to_string();

        let obj_dir = self.vault_path.join("objects").join(&id);
        let assets_dir = obj_dir.join("assets");
        std::fs::create_dir_all(&assets_dir)?;

        let file_name = file_path.file_name().unwrap_or_default();
        std::fs::copy(file_path, assets_dir.join(file_name))?;
        let file_name_str = file_name.to_str().unwrap_or("file.bin");
        let primary_file = format!("assets/{}", file_name_str);

        let content = format!("[附件: {} ({}格式, {} 字节)]", title, ext, file_size);

        let index_md = format!(
            "---\nid: {}\ntype: document\nformat: attachment\nformat_detail: {}\ntitle: {}\ncreated: {}\ncreated_by: human\nstatus: draft\ngroup: {}\nprimary_file: {}\nfile_hash: {}\nfile_size: {}\ntags: []\n---\n\n{}",
            id, ext, title, now, group_name, primary_file, hash, file_size, content
        );
        std::fs::write(obj_dir.join("index.md"), &index_md)?;

        let obj = Object {
            id: id.clone(),
            r#type: "document".to_string(),
            format: "attachment".to_string(),
            title,
            group_name,
            primary_file: Some(primary_file),
            file_hash: Some(hash),
            file_size,
            created_by: "human".to_string(),
            status: "draft".to_string(),
            metadata: "{}".to_string(),
            created_at: now,
            updated_at: now,
        };
        Self::persist_new_object(&mut conn, &obj, &content, &obj_dir)?;

        // 自动检测并建立关系
        let _ = self.auto_link_relations(&id);

        Ok(obj)
    }

    /// 获取对象主资产的绝对路径（供前端预览图片/PDF 等使用）
    pub fn get_asset_path(&self, object_id: &str) -> Result<Option<PathBuf>> {
        let conn = self.conn()?;
        let obj = db::get_object(&conn, object_id)?
            .ok_or_else(|| KbError::NotFound(object_id.to_string()))?;
        let primary_file = match obj.primary_file {
            Some(f) if !f.is_empty() => f,
            _ => return Ok(None),
        };
        let path = self
            .vault_path
            .join("objects")
            .join(object_id)
            .join(&primary_file);
        Ok(Some(path))
    }

    // 保存对象内容
    pub fn save_object_content(&self, id: &str, title: &str, content: &str) -> Result<()> {
        let _lock = crate::lock::FileLock::acquire(&self.vault_path, 5)?;
        let conn = self.conn()?;
        let old_obj =
            db::get_object(&conn, id)?.ok_or_else(|| KbError::NotFound(id.to_string()))?;
        if old_obj.format != "markdown" && old_obj.format != "code" {
            return Err(KbError::ParseFailed("该对象格式为只读".to_string()));
        }
        let primary_file = old_obj.primary_file.clone().unwrap_or_default();
        let path = self.vault_path.join("objects").join(id).join(&primary_file);
        let old_content = std::fs::read(&path)?;
        let index_path = self.vault_path.join("objects").join(id).join("index.md");
        let old_index = std::fs::read(&index_path)?;

        Self::atomic_write(&path, content.as_bytes())?;
        let new_hash = Self::file_hash(&path)?;
        let mut new_obj = old_obj.clone();
        new_obj.title = title.to_string();
        new_obj.file_hash = Some(new_hash.clone());
        new_obj.file_size = content.len() as i64;
        new_obj.updated_at = Self::now_secs();
        let tags = db::get_tags(&conn, id)?;
        if let Err(error) = self.write_index(&new_obj, &tags, content) {
            let _ = Self::atomic_write(&path, &old_content);
            return Err(error);
        }
        if let Err(error) = db::update_object_content(
            &conn,
            id,
            title,
            content,
            Some(&new_hash),
            content.len() as i64,
        ) {
            let _ = Self::atomic_write(&path, &old_content);
            let _ = Self::atomic_write(&index_path, &old_index);
            return Err(error);
        }
        drop(conn);
        drop(_lock);

        // 自动检测并建立关系（独立连接，错误被记录但不影响保存）
        if let Err(e) = self.auto_link_relations(id) {
            tracing::warn!("auto_link_relations failed for {}: {}", id, e);
        }
        Ok(())
    }

    pub fn update_object_group(&self, id: &str, group: &str) -> Result<()> {
        let _lock = crate::lock::FileLock::acquire(&self.vault_path, 5)?;
        let conn = self.conn()?;
        let old_obj =
            db::get_object(&conn, id)?.ok_or_else(|| KbError::NotFound(id.to_string()))?;
        let tags = db::get_tags(&conn, id)?;
        let body = self.read_index_body(id)?;
        let mut new_obj = old_obj.clone();
        new_obj.group_name = group.to_string();
        new_obj.updated_at = Self::now_secs();
        self.write_index(&new_obj, &tags, &body)?;
        if let Err(error) = db::update_object_group(&conn, id, group) {
            let _ = self.write_index(&old_obj, &tags, &body);
            return Err(error);
        }
        Ok(())
    }

    /// 仅修改标题，不依赖前端是否已经加载正文，避免用空缓存覆盖内容。
    pub fn rename_object(&self, id: &str, title: &str) -> Result<()> {
        let content = self.read_object_content(id)?;
        let conn = self.conn()?;
        let obj = db::get_object(&conn, id)?.ok_or_else(|| KbError::NotFound(id.to_string()))?;
        drop(conn);
        if matches!(obj.format.as_str(), "markdown" | "code") {
            self.save_object_content(id, title, &content)
        } else {
            let _lock = crate::lock::FileLock::acquire(&self.vault_path, 5)?;
            let conn = self.conn()?;
            let tags = db::get_tags(&conn, id)?;
            let body = self.read_index_body(id)?;
            let mut updated = obj.clone();
            updated.title = title.to_string();
            updated.updated_at = Self::now_secs();
            self.write_index(&updated, &tags, &body)?;
            if let Err(error) = db::update_object_title(&conn, id, title) {
                let _ = self.write_index(&obj, &tags, &body);
                return Err(error);
            }
            Ok(())
        }
    }

    pub fn set_object_creator(&self, id: &str, created_by: &str) -> Result<()> {
        let creator = normalize_creator(created_by);
        let _lock = crate::lock::FileLock::acquire(&self.vault_path, 5)?;
        let conn = self.conn()?;
        let old = db::get_object(&conn, id)?.ok_or_else(|| KbError::NotFound(id.to_string()))?;
        let tags = db::get_tags(&conn, id)?;
        let body = self.read_index_body(id)?;
        let mut updated = old.clone();
        updated.created_by = creator.to_string();
        updated.updated_at = Self::now_secs();
        self.write_index(&updated, &tags, &body)?;
        if let Err(error) = db::update_object_creator(&conn, id, creator) {
            let _ = self.write_index(&old, &tags, &body);
            return Err(error);
        }
        Ok(())
    }

    pub fn rename_group(&self, old_group: &str, new_group: &str) -> Result<usize> {
        let ids: Vec<String> = {
            let conn = self.conn()?;
            db::list_objects(&conn)?
                .into_iter()
                .filter(|obj| obj.group_name == old_group)
                .map(|obj| obj.id)
                .collect()
        };
        for id in &ids {
            self.update_object_group(id, new_group)?;
        }
        let conn = self.conn()?;
        db::rename_group_entry(&conn, old_group, new_group)?;
        Ok(ids.len())
    }

    pub fn create_group(&self, name: &str) -> Result<bool> {
        let name = name.trim();
        if name.is_empty() {
            return Err(KbError::ParseFailed("文件夹名称不能为空".to_string()));
        }
        let _lock = crate::lock::FileLock::acquire(&self.vault_path, 5)?;
        let conn = self.conn()?;
        db::create_group(&conn, name)
    }

    pub fn delete_group(&self, group: &str) -> Result<usize> {
        let count = self.rename_group(group, "")?;
        let conn = self.conn()?;
        db::delete_group_entry(&conn, group)?;
        Ok(count)
    }

    pub fn duplicate_object(&self, id: &str) -> Result<Object> {
        let conn = self.conn()?;
        let source = db::get_object(&conn, id)?.ok_or_else(|| KbError::NotFound(id.to_string()))?;
        let tags = db::get_tags(&conn, id)?;
        let searchable = if matches!(source.format.as_str(), "markdown" | "code") {
            self.read_object_content(id)?
        } else {
            self.read_index_body(id)?
        };
        drop(conn);

        let _lock = crate::lock::FileLock::acquire(&self.vault_path, 5)?;
        let new_id = Self::generate_id();
        let source_dir = self.vault_path.join("objects").join(id);
        let target_dir = self.vault_path.join("objects").join(&new_id);
        copy_directory(&source_dir, &target_dir)?;
        let mut duplicate = source.clone();
        duplicate.id = new_id.clone();
        duplicate.title = format!("{} 副本", source.title);
        duplicate.created_at = Self::now_secs();
        duplicate.updated_at = duplicate.created_at;
        duplicate.status = "draft".to_string();
        self.write_index(&duplicate, &tags, &searchable)?;
        let mut conn = self.conn()?;
        Self::persist_new_object(&mut conn, &duplicate, &searchable, &target_dir)?;
        db::add_tags(&conn, &new_id, &tags)?;
        Ok(duplicate)
    }

    pub fn delete_object(&self, id: &str) -> Result<()> {
        let _lock = crate::lock::FileLock::acquire(&self.vault_path, 5)?;
        let conn = self.conn()?;
        let old_obj =
            db::get_object(&conn, id)?.ok_or_else(|| KbError::NotFound(id.to_string()))?;
        let tags = db::get_tags(&conn, id)?;
        let body = self.read_index_body(id)?;
        let mut new_obj = old_obj.clone();
        new_obj.status = "deleted".to_string();
        new_obj.updated_at = Self::now_secs();
        self.write_index(&new_obj, &tags, &body)?;
        if let Err(error) = db::delete_object(&conn, id) {
            let _ = self.write_index(&old_obj, &tags, &body);
            return Err(error);
        }
        Ok(())
    }

    pub fn restore_object(&self, id: &str) -> Result<()> {
        let _lock = crate::lock::FileLock::acquire(&self.vault_path, 5)?;
        let conn = self.conn()?;
        let old_obj =
            db::get_object(&conn, id)?.ok_or_else(|| KbError::NotFound(id.to_string()))?;
        let tags = db::get_tags(&conn, id)?;
        let body = self.read_index_body(id)?;
        let mut new_obj = old_obj.clone();
        new_obj.status = "draft".to_string();
        new_obj.updated_at = Self::now_secs();
        self.write_index(&new_obj, &tags, &body)?;
        if let Err(error) = db::restore_object(&conn, id) {
            let _ = self.write_index(&old_obj, &tags, &body);
            return Err(error);
        }
        Ok(())
    }

    pub fn permanent_delete_object(&self, id: &str) -> Result<()> {
        let _lock = crate::lock::FileLock::acquire(&self.vault_path, 5)?;
        let obj_dir = self.vault_path.join("objects").join(id);
        let staging_dir = self.vault_path.join(".kb").join("staging");
        std::fs::create_dir_all(&staging_dir)?;
        let staged = staging_dir.join(format!("{}-delete-{}", id, std::process::id()));

        if obj_dir.exists() {
            std::fs::rename(&obj_dir, &staged)?;
        }

        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        if let Err(error) =
            db::permanent_delete_db(&tx, id).and_then(|_| tx.commit().map_err(Into::into))
        {
            if staged.exists() {
                let _ = std::fs::rename(&staged, &obj_dir);
            }
            return Err(error);
        }

        if staged.exists() {
            // 数据库删除已经提交；暂存目录即使清理失败，也会被一致性检查识别，数据不会失联。
            let _ = std::fs::remove_dir_all(&staged);
        }
        Ok(())
    }

    /// 自动检测对象的关联关系，对标 Obsidian 并扩展更智能的关联策略：
    ///
    /// 1. `[[标题]]` 双向链接（Obsidian 核心机制，related_to）
    ///    - 支持 `[[标题|别名]]` 和 `[[标题#章节]]`
    ///    - 大小写不敏感
    ///
    /// 2. 内容中纯文本提及对象标题（related_to，弱关联）
    ///    - 标题字符数 ≥ 3，避免短标题误判
    ///
    /// 3. 同分组自动关联（same_group）
    ///    - 同一 group_name 下的对象互相关联，适合主题聚类
    ///    - 排除「收件箱」等默认分组（避免无意义关联）
    ///
    /// 4. 共享标签关联（same_tag）
    ///    - 至少一个相同标签即建立关联
    ///
    /// 5. 标题包含关联（title_contains）
    ///    - 一个标题是另一个的子串（如「RAG」匹配「RAG 检索」）
    ///    - 短标题（< 3 字符）不参与，避免误判
    pub fn auto_link_relations(&self, object_id: &str) -> Result<usize> {
        let conn = self.conn()?;

        // 获取当前对象
        let obj = db::get_object(&conn, object_id)?
            .ok_or_else(|| KbError::NotFound(object_id.to_string()))?;

        // 读取内容
        let content = match obj.format.as_str() {
            "markdown" | "code" => {
                let primary_file = obj.primary_file.unwrap_or_default();
                let path = self
                    .vault_path
                    .join("objects")
                    .join(object_id)
                    .join(&primary_file);
                std::fs::read_to_string(&path).unwrap_or_default()
            }
            _ => {
                // 非文本格式，读取 index.md
                let index_path = self
                    .vault_path
                    .join("objects")
                    .join(object_id)
                    .join("index.md");
                std::fs::read_to_string(&index_path).unwrap_or_default()
            }
        };

        // 获取所有其他对象 + 标签映射（一次查询避免 N+1）
        let all_objects = db::list_objects(&conn)?;
        let other_objects: Vec<_> = all_objects
            .into_iter()
            .filter(|o| o.id != object_id)
            .collect();
        let all_tags = db::get_all_tags_map(&conn)?;
        let current_tags: Vec<String> = all_tags.get(object_id).cloned().unwrap_or_default();

        let mut linked_count = 0;

        // 幂等性检查辅助函数
        let relation_exists = |conn: &Connection, from: &str, to: &str, rel: &str| -> bool {
            conn.query_row(
                "SELECT COUNT(*) > 0 FROM relations WHERE source_id = ?1 AND target_id = ?2 AND relation_type = ?3",
                rusqlite::params![from, to, rel],
                |row| row.get(0),
            )
            .unwrap_or(false)
        };

        // ========== 策略 1: [[xxx]] 双向链接（Obsidian 核心） ==========
        let wiki_links = extract_wiki_links(&content);
        for link_target in wiki_links {
            // Obsidian 支持 [[标题|别名]] 和 [[标题#章节]]，取 | 和 # 之前的部分作为匹配标题
            let link_title = link_target
                .split('|')
                .next()
                .unwrap_or("")
                .split('#')
                .next()
                .unwrap_or("")
                .trim();
            if link_title.is_empty() {
                continue;
            }
            // 大小写不敏感匹配对象标题
            let lower = link_title.to_lowercase();
            for other in &other_objects {
                if other.title == link_title || other.title.to_lowercase() == lower {
                    if !relation_exists(&conn, object_id, &other.id, "related_to") {
                        db::link_objects(&conn, object_id, &other.id, "related_to")?;
                        linked_count += 1;
                    }
                    break;
                }
            }
        }

        // ========== 策略 2: 纯文本提及（弱关联） ==========
        for other in &other_objects {
            let title_len = other.title.chars().count();
            if title_len < 3 {
                continue;
            }
            // 跳过与当前对象同名的对象（避免自引用：内容里包含自己的标题不算关联）
            if other.title == obj.title {
                continue;
            }
            if content_contains_title(&content, &other.title)
                && !relation_exists(&conn, object_id, &other.id, "related_to")
            {
                db::link_objects(&conn, object_id, &other.id, "related_to")?;
                linked_count += 1;
            }
        }

        // ========== 策略 3: 同分组自动关联（same_group） ==========
        // 排除默认分组「收件箱」「未分组」——这些分组没有主题意义
        let group = obj.group_name.trim();
        let skip_groups = ["收件箱", "未分组", ""];
        if !skip_groups.contains(&group) {
            for other in &other_objects {
                if other.group_name.trim() == group
                    && !relation_exists(&conn, object_id, &other.id, "same_group")
                {
                    db::link_objects(&conn, object_id, &other.id, "same_group")?;
                    linked_count += 1;
                }
            }
        }

        // ========== 策略 4: 共享标签关联（same_tag） ==========
        if !current_tags.is_empty() {
            for other in &other_objects {
                if other.id == object_id {
                    continue;
                }
                let other_tags = all_tags.get(&other.id);
                if let Some(tags) = other_tags {
                    let has_common = current_tags.iter().any(|t| tags.contains(t));
                    if has_common && !relation_exists(&conn, object_id, &other.id, "same_tag") {
                        db::link_objects(&conn, object_id, &other.id, "same_tag")?;
                        linked_count += 1;
                    }
                }
            }
        }

        // ========== 策略 5: 标题包含关联（title_contains） ==========
        let current_title = obj.title.trim();
        let current_title_lower = current_title.to_lowercase();
        let current_len = current_title.chars().count();
        for other in &other_objects {
            let other_title = other.title.trim();
            let other_len = other_title.chars().count();
            // 短标题跳过，避免「AI」「222」这类误判
            if current_len < 3 || other_len < 3 {
                continue;
            }
            // 跳过完全相同的标题（同标题本身可能是重复对象，不在这里处理）
            if current_title_lower == other_title.to_lowercase() {
                continue;
            }
            // 单向包含：current 包含 other 或 other 包含 current
            let contains = current_title_lower.contains(&other_title.to_lowercase())
                || other_title.to_lowercase().contains(&current_title_lower);
            if contains && !relation_exists(&conn, object_id, &other.id, "title_contains") {
                db::link_objects(&conn, object_id, &other.id, "title_contains")?;
                linked_count += 1;
            }
        }

        Ok(linked_count)
    }

    /// 重建所有对象的关系
    /// 清除所有自动建立的关系（related_to / same_group / same_tag / title_contains），
    /// 保留手动建立的 depends_on / part_of / implements
    pub fn rebuild_all_relations(&self) -> Result<usize> {
        let conn = self.conn()?;
        // 清除所有自动建立的关系类型
        for rel_type in ["related_to", "same_group", "same_tag", "title_contains"] {
            conn.execute(
                &format!("DELETE FROM relations WHERE relation_type = '{}'", rel_type),
                [],
            )?;
        }

        // 获取所有对象
        let objects = db::list_objects(&conn)?;
        let mut total = 0;
        for obj in &objects {
            if let Ok(count) = self.auto_link_relations(&obj.id) {
                total += count;
            }
        }
        Ok(total)
    }

    // 搜索
    pub fn search(&self, query: &str, limit: i64) -> Result<Vec<ObjectSummary>> {
        let conn = self.conn()?;
        db::search_objects(&conn, query, limit)
    }

    // 获取文件树
    pub fn get_file_tree(&self) -> Result<Vec<GroupNode>> {
        let conn = self.conn()?;
        db::get_file_tree(&conn)
    }

    pub fn get_collection_tree(&self) -> Result<Vec<CollectionNode>> {
        let conn = self.conn()?;
        db::get_collection_tree(&conn)
    }

    pub fn create_collection(
        &self,
        name: &str,
        parent_id: Option<&str>,
        description: &str,
        created_by: &str,
    ) -> Result<String> {
        let _lock = crate::lock::FileLock::acquire(&self.vault_path, 5)?;
        let conn = self.conn()?;
        db::create_collection(&conn, name, parent_id, description, created_by)
    }

    pub fn move_object_to_collection(&self, object_id: &str, collection_id: &str) -> Result<()> {
        let path = {
            let conn = self.conn()?;
            db::collection_path(&conn, collection_id)?
        };
        self.update_object_group(object_id, &path)?;
        let _lock = crate::lock::FileLock::acquire(&self.vault_path, 5)?;
        let conn = self.conn()?;
        db::assign_object_to_collection(&conn, object_id, collection_id, true)
    }

    pub fn ensure_collection_path(
        &self,
        path: &str,
        description: &str,
        created_by: &str,
    ) -> Result<String> {
        let segments: Vec<&str> = path
            .split('/')
            .map(str::trim)
            .filter(|part| !part.is_empty())
            .collect();
        if segments.is_empty() {
            return Err(KbError::ParseFailed("目录路径不能为空".to_string()));
        }
        let _lock = crate::lock::FileLock::acquire(&self.vault_path, 5)?;
        let conn = self.conn()?;
        let mut parent_id: Option<String> = None;
        for (index, segment) in segments.iter().enumerate() {
            let leaf_description = if index + 1 == segments.len() {
                description
            } else {
                ""
            };
            let id = db::create_collection(
                &conn,
                segment,
                parent_id.as_deref(),
                leaf_description,
                created_by,
            )?;
            parent_id = Some(id);
        }
        parent_id.ok_or_else(|| KbError::ParseFailed("目录路径不能为空".to_string()))
    }

    pub fn create_ai_task(
        &self,
        id: &str,
        scope: &str,
        instruction: &str,
        provider: &str,
        model: &str,
    ) -> Result<()> {
        let conn = self.conn()?;
        db::create_ai_task(&conn, id, scope, instruction, provider, model)
    }

    pub fn add_ai_task_action(
        &self,
        task_id: &str,
        action_type: &str,
        object_id: Option<&str>,
        collection_id: Option<&str>,
        description: &str,
    ) -> Result<()> {
        let conn = self.conn()?;
        db::add_ai_task_action(
            &conn,
            task_id,
            action_type,
            object_id,
            collection_id,
            description,
        )
    }

    pub fn finish_ai_task(&self, task_id: &str, status: &str, summary: &str) -> Result<()> {
        let conn = self.conn()?;
        db::finish_ai_task(&conn, task_id, status, summary)
    }

    pub fn list_ai_tasks(&self, limit: i64) -> Result<Vec<AiTask>> {
        let conn = self.conn()?;
        db::list_ai_tasks(&conn, limit)
    }

    pub fn list_ai_task_actions(&self, task_id: &str) -> Result<Vec<AiTaskAction>> {
        let conn = self.conn()?;
        db::list_ai_task_actions(&conn, task_id)
    }

    pub fn recover_running_ai_tasks(&self) -> Result<usize> {
        let conn = self.conn()?;
        db::recover_running_ai_tasks(&conn)
    }

    pub fn prune_empty_collections(&self) -> Result<usize> {
        let _lock = crate::lock::FileLock::acquire(&self.vault_path, 5)?;
        let conn = self.conn()?;
        db::prune_empty_collections(&conn)
    }

    // 获取对象详情
    pub fn get_object_detail(&self, id: &str) -> Result<(Object, Vec<String>, Vec<Relation>)> {
        let conn = self.conn()?;
        let obj = db::get_object(&conn, id)?.ok_or_else(|| KbError::NotFound(id.to_string()))?;
        let tags = db::get_tags(&conn, id)?;
        let relations = db::get_relations(&conn, id)?;
        Ok((obj, tags, relations))
    }

    /// 递归查询关联关系（深度 N）
    /// 从指定对象出发，沿出链遍历，返回所有深度 ≤ max_depth 的关联对象
    pub fn get_relations_recursive(&self, id: &str, max_depth: u32) -> Result<Vec<Relation>> {
        let conn = self.conn()?;
        db::get_relations_recursive(&conn, id, max_depth)
    }

    // 统计
    pub fn get_stats(&self) -> Result<VaultStats> {
        let conn = self.conn()?;
        db::get_stats(&conn)
    }

    // ===== 健壮性方法 =====

    /// 备份数据库（保留最近 7 个备份）
    /// 每天只备份一次（检查是否已有今天的备份）
    pub fn backup_database(&self) -> Result<Option<PathBuf>> {
        let backup_dir = self.vault_path.join(".kb").join("backups");
        if backup_dir.exists() {
            // 检查今天是否已有备份
            let today = chrono::Local::now().format("%Y%m%d").to_string();
            if let Ok(entries) = std::fs::read_dir(&backup_dir) {
                for entry in entries.flatten() {
                    let name = entry.file_name().to_string_lossy().to_string();
                    if name.contains(&today) {
                        return Ok(None); // 今天已有备份
                    }
                }
            }
        }
        let path = db::backup_database(&self.db_path, &self.vault_path)?;
        tracing::info!("数据库已备份: {}", path.display());
        Ok(Some(path))
    }

    /// 一致性检查
    /// 返回检查报告（不影响运行）
    pub fn check_consistency(&self) -> Result<serde_json::Value> {
        let conn = self.conn()?;
        let orphan_objects = db::find_orphan_objects(&conn, &self.vault_path)?;
        let orphan_files = db::find_orphan_files(&conn, &self.vault_path)?;
        let changed_files = db::find_changed_files(&conn, &self.vault_path)?;

        if !orphan_objects.is_empty() {
            tracing::warn!(
                "发现 {} 个孤儿对象（数据库有记录但文件不存在）: {:?}",
                orphan_objects.len(),
                orphan_objects
            );
        }
        if !orphan_files.is_empty() {
            tracing::warn!(
                "发现 {} 个孤儿文件（文件存在但数据库无记录）: {:?}",
                orphan_files.len(),
                orphan_files
            );
        }
        if !changed_files.is_empty() {
            tracing::warn!(
                "发现 {} 个文件 hash 已变化: {:?}",
                changed_files.len(),
                changed_files
            );
        }

        Ok(serde_json::json!({
            "orphan_objects": orphan_objects,
            "orphan_files": orphan_files,
            "changed_files": changed_files,
        }))
    }

    /// 完整重建：从文件系统重新扫描，重建数据库
    pub fn rebuild(&self) -> Result<usize> {
        let _lock = crate::lock::FileLock::acquire(&self.vault_path, 10)?;
        let objects_dir = self.vault_path.join("objects");
        if !objects_dir.exists() {
            return Ok(0);
        }

        // 数据库仍可读时，保留旧版本尚未同步到 index.md 的最新元数据和人工关系。
        let existing_conn = self.conn()?;
        let existing_objects: std::collections::HashMap<String, Object> =
            db::list_all_objects(&existing_conn)
                .unwrap_or_default()
                .into_iter()
                .map(|obj| (obj.id.clone(), obj))
                .collect();
        let mut existing_relations = Vec::new();
        if let Ok(mut stmt) = existing_conn
            .prepare("SELECT source_id, target_id, relation_type, created_at FROM relations")
        {
            if let Ok(rows) = stmt.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            }) {
                existing_relations.extend(rows.filter_map(|row| row.ok()));
            }
        }
        drop(existing_conn);

        let mut recovered = Vec::new();
        let mut parse_errors = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&objects_dir) {
            for entry in entries.flatten() {
                let id = entry.file_name().to_string_lossy().to_string();
                let obj_dir = objects_dir.join(&id);
                if !obj_dir.is_dir() || !id.starts_with("obj_") {
                    continue;
                }
                let index_path = obj_dir.join("index.md");
                if !index_path.exists() {
                    continue;
                }
                match std::fs::read_to_string(&index_path) {
                    Ok(content) => match Self::parse_index_md(&id, &content) {
                        Ok((mut obj, tags)) => {
                            if let Some(existing) = existing_objects.get(&id) {
                                obj.title = existing.title.clone();
                                obj.group_name = existing.group_name.clone();
                                obj.status = existing.status.clone();
                                obj.created_by = existing.created_by.clone();
                                obj.metadata = existing.metadata.clone();
                                obj.updated_at = existing.updated_at;
                            }
                            let searchable_content =
                                if matches!(obj.format.as_str(), "markdown" | "code") {
                                    obj.primary_file
                                        .as_deref()
                                        .and_then(|file| {
                                            std::fs::read_to_string(obj_dir.join(file)).ok()
                                        })
                                        .unwrap_or_else(|| Self::index_body(&content).to_string())
                                } else {
                                    Self::index_body(&content).to_string()
                                };
                            recovered.push((obj, tags, searchable_content));
                        }
                        Err(error) => parse_errors.push(format!("{}: {}", id, error)),
                    },
                    Err(error) => parse_errors.push(format!("{}: {}", id, error)),
                }
            }
        }

        if !parse_errors.is_empty() {
            return Err(KbError::ParseFailed(format!(
                "重建已中止，{} 个对象无法解析：{}",
                parse_errors.len(),
                parse_errors.join("; ")
            )));
        }

        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        tx.execute("DELETE FROM relations", [])?;
        tx.execute("DELETE FROM tags", [])?;
        tx.execute("DELETE FROM objects", [])?;
        tx.execute("DELETE FROM objects_fts", [])?;
        for (obj, tags, content) in &recovered {
            db::create_object(&tx, obj)?;
            db::add_tags(&tx, &obj.id, tags)?;
            tx.execute(
                "UPDATE objects_fts SET content = ?1, tags = ?2 WHERE id = ?3",
                params![content, tags.join(" "), obj.id],
            )?;
        }
        for (source, target, relation_type, created_at) in existing_relations {
            tx.execute(
                "INSERT OR IGNORE INTO relations (source_id, target_id, relation_type, created_at) VALUES (?1, ?2, ?3, ?4)",
                params![source, target, relation_type, created_at],
            )?;
        }
        tx.commit()?;

        let count = recovered.len();
        // 在保留已有关系的基础上补齐自动关系。
        if count > 0 {
            for (obj, _, _) in &recovered {
                let _ = self.auto_link_relations(&obj.id);
            }
        }

        Ok(count)
    }

    /// 解析 index.md 中的 YAML 头部
    fn parse_index_md(id: &str, content: &str) -> Result<(Object, Vec<String>)> {
        if !content.starts_with("---") {
            return Err(KbError::ParseFailed("缺少 YAML 头部".to_string()));
        }
        let end = content[3..]
            .find("---")
            .map(|i| i + 3)
            .ok_or_else(|| KbError::ParseFailed("YAML 头部未闭合".to_string()))?;
        let header = &content[3..end];
        let mut obj = Object {
            id: id.to_string(),
            r#type: "document".to_string(),
            format: "markdown".to_string(),
            title: "untitled".to_string(),
            group_name: "收件箱".to_string(),
            primary_file: None,
            file_hash: None,
            file_size: 0,
            created_by: "human".to_string(),
            status: "draft".to_string(),
            metadata: "{}".to_string(),
            created_at: 0,
            updated_at: 0,
        };
        let mut tags = Vec::new();
        for line in header.lines() {
            let line = line.trim();
            if let Some((key, value)) = line.split_once(':') {
                let key = key.trim();
                let raw_value = value.trim();
                let value = serde_json::from_str::<String>(raw_value)
                    .unwrap_or_else(|_| raw_value.trim_matches('"').to_string());
                match key {
                    "type" => obj.r#type = value,
                    "format" => obj.format = value,
                    "title" => obj.title = value,
                    "group" => obj.group_name = value,
                    "primary_file" if !value.is_empty() => obj.primary_file = Some(value),
                    "file_hash" if !value.is_empty() => obj.file_hash = Some(value),
                    "file_size" => obj.file_size = value.parse().unwrap_or(0),
                    "created_by" => obj.created_by = value,
                    "status" => obj.status = value,
                    "metadata" => obj.metadata = raw_value.to_string(),
                    "tags" => {
                        tags = serde_json::from_str::<Vec<String>>(raw_value).unwrap_or_default();
                    }
                    "created" | "updated_at" => {
                        if let Ok(ts) = value.parse::<i64>() {
                            if key == "created" {
                                obj.created_at = ts;
                            } else {
                                obj.updated_at = ts;
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        if obj.updated_at == 0 {
            obj.updated_at = obj.created_at;
        }
        Ok((obj, tags))
    }
}

fn normalize_creator(created_by: &str) -> &'static str {
    if created_by.eq_ignore_ascii_case("agent") {
        "agent"
    } else {
        "human"
    }
}

fn copy_directory(source: &Path, target: &Path) -> Result<()> {
    std::fs::create_dir_all(target)?;
    for entry in std::fs::read_dir(source)? {
        let entry = entry?;
        let source_path = entry.path();
        let target_path = target.join(entry.file_name());
        if source_path.is_dir() {
            copy_directory(&source_path, &target_path)?;
        } else {
            std::fs::copy(source_path, target_path)?;
        }
    }
    Ok(())
}

// ===== 代码文件辅助函数 =====

fn ext_to_language(ext: &str) -> String {
    match ext {
        "rs" => "rust",
        "py" => "python",
        "js" | "jsx" | "mjs" | "cjs" => "javascript",
        "ts" | "tsx" => "typescript",
        "go" => "go",
        "java" => "java",
        "c" | "h" => "c",
        "cpp" | "hpp" | "cc" | "cxx" => "cpp",
        "cs" => "csharp",
        "rb" => "ruby",
        "php" => "php",
        "swift" => "swift",
        "kt" | "kts" => "kotlin",
        "scala" | "sc" => "scala",
        "sh" | "bash" => "bash",
        "zsh" => "zsh",
        "fish" => "fish",
        "sql" => "sql",
        "html" | "htm" => "html",
        "css" => "css",
        "scss" => "scss",
        "less" => "less",
        "vue" => "vue",
        "svelte" => "svelte",
        "yaml" | "yml" => "yaml",
        "toml" => "toml",
        "json" => "json",
        "xml" => "xml",
        "ini" | "conf" | "cfg" => "ini",
        "lua" => "lua",
        "r" => "r",
        "dart" => "dart",
        "elm" => "elm",
        "ex" | "exs" => "elixir",
        "clj" | "cljs" => "clojure",
        "hs" => "haskell",
        "ml" => "ocaml",
        "fs" => "fsharp",
        "nim" => "nim",
        "v" => "vlang",
        "zig" => "zig",
        "asm" | "s" => "assembly",
        _ => "text",
    }
    .to_string()
}

fn extract_docx_text(path: &Path) -> Result<String> {
    let document = read_zip_entry(path, "word/document.xml")?;
    Ok(normalize_extracted_text(&xml_text_content(
        &document,
        |name| xml_name_eq(name, b"p") || xml_name_eq(name, b"br"),
    )?))
}

fn extract_xlsx_text(path: &Path) -> Result<String> {
    let file = std::fs::File::open(path)?;
    let mut archive = ZipArchive::new(file)
        .map_err(|error| KbError::ParseFailed(format!("读取 Office 压缩包失败: {}", error)))?;

    let shared_strings =
        if let Some(xml) = optional_zip_entry(&mut archive, "xl/sharedStrings.xml")? {
            parse_shared_strings(&xml)?
        } else {
            Vec::new()
        };
    let sheet_names = read_sheet_names(&mut archive)?;
    let worksheet_paths: Vec<String> = archive
        .file_names()
        .filter(|name| {
            name.starts_with("xl/worksheets/")
                && name.ends_with(".xml")
                && !name.contains("/_rels/")
        })
        .map(|name| name.to_string())
        .collect();

    let mut sections = Vec::new();
    for worksheet_path in worksheet_paths {
        let xml = optional_zip_entry(&mut archive, &worksheet_path)?.unwrap_or_default();
        let text = parse_worksheet_text(&xml, &shared_strings)?;
        if text.trim().is_empty() {
            continue;
        }
        let sheet_name = sheet_names
            .get(&worksheet_path)
            .cloned()
            .unwrap_or_else(|| worksheet_path.clone());
        sections.push(format!("## {}\n{}", sheet_name, text));
    }

    Ok(normalize_extracted_text(&sections.join("\n\n")))
}

fn read_zip_entry(path: &Path, name: &str) -> Result<String> {
    let file = std::fs::File::open(path)?;
    let mut archive = ZipArchive::new(file)
        .map_err(|error| KbError::ParseFailed(format!("读取 Office 压缩包失败: {}", error)))?;
    optional_zip_entry(&mut archive, name)?
        .ok_or_else(|| KbError::ParseFailed(format!("Office 文件缺少 {}", name)))
}

fn optional_zip_entry<R: std::io::Read + std::io::Seek>(
    archive: &mut ZipArchive<R>,
    name: &str,
) -> Result<Option<String>> {
    let mut entry = match archive.by_name(name) {
        Ok(entry) => entry,
        Err(zip::result::ZipError::FileNotFound) => return Ok(None),
        Err(error) => {
            return Err(KbError::ParseFailed(format!(
                "读取 Office 条目失败 {}: {}",
                name, error
            )))
        }
    };
    let mut content = String::new();
    entry.read_to_string(&mut content)?;
    Ok(Some(content))
}

fn xml_name_eq(name: &[u8], expected: &[u8]) -> bool {
    name.rsplit(|byte| *byte == b':').next() == Some(expected)
}

fn attr_value(start: &BytesStart<'_>, key: &[u8]) -> Option<String> {
    for attr in start.attributes().flatten() {
        if xml_name_eq(attr.key.as_ref(), key) {
            return Some(String::from_utf8_lossy(attr.value.as_ref()).to_string());
        }
    }
    None
}

fn xml_text_content<F>(xml: &str, mut line_break_after: F) -> Result<String>
where
    F: FnMut(&[u8]) -> bool,
{
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut out = String::new();

    loop {
        match reader.read_event() {
            Ok(Event::Text(text)) => {
                let decoded = text.decode().map_err(|error| {
                    KbError::ParseFailed(format!("XML 文本解码失败: {}", error))
                })?;
                out.push_str(&decoded);
            }
            Ok(Event::Empty(start)) => {
                let name = start.name();
                if xml_name_eq(name.as_ref(), b"tab") {
                    out.push('\t');
                } else if line_break_after(name.as_ref()) {
                    out.push('\n');
                }
            }
            Ok(Event::End(end)) if line_break_after(end.name().as_ref()) => out.push('\n'),
            Ok(Event::Eof) => break,
            Err(error) => {
                return Err(KbError::ParseFailed(format!(
                    "解析 Office XML 失败: {}",
                    error
                )))
            }
            _ => {}
        }
    }

    Ok(out)
}

fn parse_shared_strings(xml: &str) -> Result<Vec<String>> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut strings = Vec::new();
    let mut current = String::new();
    let mut in_si = false;
    let mut in_text = false;

    loop {
        match reader.read_event() {
            Ok(Event::Start(start)) if xml_name_eq(start.name().as_ref(), b"si") => {
                in_si = true;
                current.clear();
            }
            Ok(Event::Start(start)) if in_si && xml_name_eq(start.name().as_ref(), b"t") => {
                in_text = true;
            }
            Ok(Event::Text(text)) if in_text => {
                let decoded = text.decode().map_err(|error| {
                    KbError::ParseFailed(format!("XML 文本解码失败: {}", error))
                })?;
                current.push_str(&decoded);
            }
            Ok(Event::End(end)) if xml_name_eq(end.name().as_ref(), b"t") => {
                in_text = false;
            }
            Ok(Event::End(end)) if xml_name_eq(end.name().as_ref(), b"si") => {
                strings.push(current.clone());
                current.clear();
                in_si = false;
            }
            Ok(Event::Eof) => break,
            Err(error) => {
                return Err(KbError::ParseFailed(format!(
                    "解析共享字符串失败: {}",
                    error
                )))
            }
            _ => {}
        }
    }

    Ok(strings)
}

fn read_sheet_names<R: std::io::Read + std::io::Seek>(
    archive: &mut ZipArchive<R>,
) -> Result<std::collections::HashMap<String, String>> {
    let Some(workbook_xml) = optional_zip_entry(archive, "xl/workbook.xml")? else {
        return Ok(std::collections::HashMap::new());
    };
    let rels = optional_zip_entry(archive, "xl/_rels/workbook.xml.rels")?
        .map(|xml| parse_workbook_relationships(&xml))
        .transpose()?
        .unwrap_or_default();
    parse_workbook_sheets(&workbook_xml, &rels)
}

fn parse_workbook_relationships(xml: &str) -> Result<std::collections::HashMap<String, String>> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut rels = std::collections::HashMap::new();

    loop {
        match reader.read_event() {
            Ok(Event::Empty(start)) | Ok(Event::Start(start))
                if xml_name_eq(start.name().as_ref(), b"Relationship") =>
            {
                if let (Some(id), Some(target)) =
                    (attr_value(&start, b"Id"), attr_value(&start, b"Target"))
                {
                    rels.insert(id, normalize_sheet_target(&target));
                }
            }
            Ok(Event::Eof) => break,
            Err(error) => {
                return Err(KbError::ParseFailed(format!(
                    "解析工作簿关系失败: {}",
                    error
                )))
            }
            _ => {}
        }
    }

    Ok(rels)
}

fn parse_workbook_sheets(
    xml: &str,
    rels: &std::collections::HashMap<String, String>,
) -> Result<std::collections::HashMap<String, String>> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut sheets = std::collections::HashMap::new();

    loop {
        match reader.read_event() {
            Ok(Event::Empty(start)) | Ok(Event::Start(start))
                if xml_name_eq(start.name().as_ref(), b"sheet") =>
            {
                if let (Some(name), Some(rid)) =
                    (attr_value(&start, b"name"), attr_value(&start, b"id"))
                {
                    if let Some(path) = rels.get(&rid) {
                        sheets.insert(path.clone(), name);
                    }
                }
            }
            Ok(Event::Eof) => break,
            Err(error) => return Err(KbError::ParseFailed(format!("解析工作簿失败: {}", error))),
            _ => {}
        }
    }

    Ok(sheets)
}

fn normalize_sheet_target(target: &str) -> String {
    let target = target.replace('\\', "/");
    let target = target.trim_start_matches('/');
    if target.starts_with("xl/") {
        target.to_string()
    } else {
        format!("xl/{}", target)
    }
}

fn parse_worksheet_text(xml: &str, shared_strings: &[String]) -> Result<String> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut rows = Vec::new();
    let mut current_row: Vec<String> = Vec::new();
    let mut in_cell = false;
    let mut capture_value = false;
    let mut cell_type = String::new();
    let mut cell_text = String::new();

    loop {
        match reader.read_event() {
            Ok(Event::Start(start)) if xml_name_eq(start.name().as_ref(), b"row") => {
                current_row.clear();
            }
            Ok(Event::Start(start)) if xml_name_eq(start.name().as_ref(), b"c") => {
                in_cell = true;
                cell_type = attr_value(&start, b"t").unwrap_or_default();
                cell_text.clear();
            }
            Ok(Event::Start(start))
                if in_cell
                    && (xml_name_eq(start.name().as_ref(), b"v")
                        || xml_name_eq(start.name().as_ref(), b"t")) =>
            {
                capture_value = true;
            }
            Ok(Event::Text(text)) if capture_value => {
                let decoded = text.decode().map_err(|error| {
                    KbError::ParseFailed(format!("XML 文本解码失败: {}", error))
                })?;
                cell_text.push_str(&decoded);
            }
            Ok(Event::End(end))
                if xml_name_eq(end.name().as_ref(), b"v")
                    || xml_name_eq(end.name().as_ref(), b"t") =>
            {
                capture_value = false;
            }
            Ok(Event::End(end)) if xml_name_eq(end.name().as_ref(), b"c") => {
                let value = if cell_type == "s" {
                    cell_text
                        .trim()
                        .parse::<usize>()
                        .ok()
                        .and_then(|idx| shared_strings.get(idx).cloned())
                        .unwrap_or_default()
                } else {
                    cell_text.clone()
                };
                if !value.trim().is_empty() {
                    current_row.push(value.trim().to_string());
                }
                in_cell = false;
                capture_value = false;
                cell_text.clear();
            }
            Ok(Event::End(end)) if xml_name_eq(end.name().as_ref(), b"row") => {
                if !current_row.is_empty() {
                    rows.push(current_row.join("\t"));
                }
                current_row.clear();
            }
            Ok(Event::Eof) => break,
            Err(error) => return Err(KbError::ParseFailed(format!("解析工作表失败: {}", error))),
            _ => {}
        }
    }

    Ok(rows.join("\n"))
}

fn normalize_extracted_text(content: &str) -> String {
    let mut out = String::new();
    let mut blank = false;
    for line in content.lines().map(str::trim) {
        if line.is_empty() {
            if !blank && !out.is_empty() {
                out.push('\n');
                blank = true;
            }
            continue;
        }
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        out.push_str(line);
        blank = false;
    }
    out
}

/// 提取代码结构（函数名、类名等）
fn extract_code_structure(content: &str, language: &str) -> Vec<String> {
    let mut items = Vec::new();
    let re = match language {
        "rust" => regex::Regex::new(r"^\s*(pub\s+)?(fn|struct|enum|trait|impl|mod)\s+(\w+)").ok(),
        "python" => regex::Regex::new(r"^\s*(def|class)\s+(\w+)").ok(),
        "javascript" | "typescript" => {
            regex::Regex::new(r"^\s*(function|class|const|let|var)\s+(\w+)").ok()
        }
        "go" => regex::Regex::new(r"^\s*func\s+(\w+)").ok(),
        "java" | "csharp" | "cpp" | "c" => regex::Regex::new(
            r"^\s*(public|private|protected|static)?\s*(class|void|int|char|double|float|bool|auto)\s+(\w+)",
        )
        .ok(),
        "ruby" => regex::Regex::new(r"^\s*(def|class|module)\s+(\w+)").ok(),
        "php" => regex::Regex::new(r"^\s*(function|class)\s+(\w+)").ok(),
        "swift" => regex::Regex::new(r"^\s*(func|class|struct|enum|protocol)\s+(\w+)").ok(),
        "kotlin" => regex::Regex::new(r"^\s*(fun|class|object|interface|enum)\s+(\w+)").ok(),
        _ => None,
    };

    if let Some(re) = re {
        for line in content.lines() {
            if let Some(caps) = re.captures(line) {
                if let Some(name) = caps.get(caps.len() - 1) {
                    items.push(name.as_str().to_string());
                }
            }
            if items.len() >= 50 {
                break;
            }
        }
    }

    items
}

/// 从内容中提取 [[xxx]] 语法中的标题
pub fn extract_wiki_links(content: &str) -> Vec<String> {
    let mut links = Vec::new();
    let mut in_link = false;
    let mut current = String::new();

    let chars: Vec<char> = content.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        if i + 1 < chars.len() && chars[i] == '[' && chars[i + 1] == '[' {
            in_link = true;
            current.clear();
            i += 2;
            continue;
        }
        if in_link && i + 1 < chars.len() && chars[i] == ']' && chars[i + 1] == ']' {
            if !current.is_empty() {
                links.push(current.trim().to_string());
            }
            in_link = false;
            current.clear();
            i += 2;
            continue;
        }
        if in_link {
            current.push(chars[i]);
        }
        i += 1;
    }

    links
}

/// 检查内容中是否包含标题（排除 [[]] 内的部分）
fn content_contains_title(content: &str, title: &str) -> bool {
    // 移除所有 [[xxx]] 部分
    let mut clean = String::new();
    let mut in_link = false;
    let chars: Vec<char> = content.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        if i + 1 < chars.len() && chars[i] == '[' && chars[i + 1] == '[' {
            in_link = true;
            i += 2;
            continue;
        }
        if in_link && i + 1 < chars.len() && chars[i] == ']' && chars[i + 1] == ']' {
            in_link = false;
            i += 2;
            continue;
        }
        if !in_link {
            clean.push(chars[i]);
        }
        i += 1;
    }

    // 检查标题是否出现在清理后的内容中
    clean.contains(title)
}
