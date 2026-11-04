use rusqlite::{params, Connection, OptionalExtension};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::error::{KbError, Result};
use crate::model::{
    AiTask, AiTaskAction, CollectionNode, ConceptNoteTask, GroupNode, Object, ObjectSummary,
    Relation, VaultStats,
};
use serde_json;

pub fn open(path: &Path) -> Result<Connection> {
    let conn = Connection::open(path)?;
    conn.execute_batch(
        "PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON; PRAGMA busy_timeout=5000;",
    )?;
    Ok(conn)
}

pub fn init_schema(conn: &Connection) -> Result<()> {
    // 版本 7 起，迁移只执行一次。正常启动不重建触发器，也不扫描全文索引。
    let has_version_table: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'schema_version')",
        [],
        |row| row.get(0),
    )?;
    if has_version_table {
        let version: i64 = conn.query_row(
            "SELECT COALESCE(MAX(version), 0) FROM schema_version",
            [],
            |row| row.get(0),
        )?;
        if version >= 7 {
            return Ok(());
        }
    }
    // DDL、历史数据补齐和版本号一起提交，失败后下次启动可安全重试。
    let tx = conn.unchecked_transaction()?;
    migrate_schema(&tx)?;
    tx.commit()?;
    Ok(())
}

fn migrate_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS schema_version (
            version INTEGER PRIMARY KEY,
            applied_at INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS objects (
            id TEXT PRIMARY KEY,
            type TEXT NOT NULL,
            format TEXT NOT NULL,
            title TEXT NOT NULL,
            group_name TEXT DEFAULT '',
            primary_file TEXT,
            file_hash TEXT,
            file_size INTEGER DEFAULT 0,
            created_by TEXT NOT NULL,
            status TEXT DEFAULT 'draft',
            metadata TEXT DEFAULT '{}',
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_objects_type ON objects(type);
        CREATE INDEX IF NOT EXISTS idx_objects_group ON objects(group_name);
        CREATE INDEX IF NOT EXISTS idx_objects_status ON objects(status);
        CREATE INDEX IF NOT EXISTS idx_objects_created_by ON objects(created_by);
        CREATE INDEX IF NOT EXISTS idx_objects_created_at ON objects(created_at);

        CREATE TABLE IF NOT EXISTS groups (
            name TEXT PRIMARY KEY,
            created_at INTEGER NOT NULL,
            is_explicit INTEGER NOT NULL DEFAULT 1
        );

        CREATE TABLE IF NOT EXISTS tags (
            object_id TEXT NOT NULL,
            tag TEXT NOT NULL,
            PRIMARY KEY (object_id, tag),
            FOREIGN KEY (object_id) REFERENCES objects(id) ON DELETE CASCADE
        );
        CREATE INDEX IF NOT EXISTS idx_tags_tag ON tags(tag);

        CREATE TABLE IF NOT EXISTS relations (
            source_id TEXT NOT NULL,
            target_id TEXT NOT NULL,
            relation_type TEXT NOT NULL,
            source TEXT NOT NULL DEFAULT 'automatic',
            created_at INTEGER NOT NULL,
            PRIMARY KEY (source_id, target_id, relation_type),
            FOREIGN KEY (source_id) REFERENCES objects(id) ON DELETE CASCADE,
            FOREIGN KEY (target_id) REFERENCES objects(id) ON DELETE CASCADE
        );
        CREATE INDEX IF NOT EXISTS idx_relations_target ON relations(target_id);
        CREATE INDEX IF NOT EXISTS idx_relations_type ON relations(relation_type);

        CREATE VIRTUAL TABLE IF NOT EXISTS objects_fts USING fts5(
            id UNINDEXED,
            title,
            content,
            tags,
            format_detail,
            tokenize='unicode61 remove_diacritics 2'
        );

        -- 中文等无空格语言使用 trigram 索引支持稳定的子串检索；短于 3 字符
        -- 的查询仍由安全 LIKE 兜底。
        CREATE VIRTUAL TABLE IF NOT EXISTS objects_fts_cjk USING fts5(
            id UNINDEXED,
            title,
            content,
            tags,
            tokenize='trigram'
        );

        -- 只同步 title 和 format_detail，content 由 DAO 手动管理
        DROP TRIGGER IF EXISTS objects_ai;
        CREATE TRIGGER objects_ai AFTER INSERT ON objects BEGIN
            INSERT INTO objects_fts(id, title, content, tags, format_detail)
            VALUES (new.id, new.title, '', '', new.format);
            INSERT INTO objects_fts_cjk(id, title, content, tags)
            VALUES (new.id, new.title, '', '');
        END;

        DROP TRIGGER IF EXISTS objects_ad;
        CREATE TRIGGER objects_ad AFTER DELETE ON objects BEGIN
            DELETE FROM objects_fts WHERE id = old.id;
            DELETE FROM objects_fts_cjk WHERE id = old.id;
        END;

        -- 旧版本在任意 metadata 更新时重建 FTS 行，导致正文被清空。
        -- 只在 title/format 变化时原位同步，保留 content/tags。
        DROP TRIGGER IF EXISTS objects_au;
        CREATE TRIGGER objects_au AFTER UPDATE OF title, format ON objects BEGIN
            UPDATE objects_fts
            SET title = new.title, format_detail = new.format
            WHERE id = old.id;
            UPDATE objects_fts_cjk SET title = new.title WHERE id = old.id;
        END;

        CREATE TABLE IF NOT EXISTS operation_log (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            timestamp INTEGER NOT NULL,
            operation TEXT NOT NULL,
            object_id TEXT,
            before_state TEXT,
            after_state TEXT,
            file_changes TEXT,
            status TEXT NOT NULL DEFAULT 'pending',
            error_message TEXT,
            rollback_id INTEGER REFERENCES operation_log(id)
        );
        CREATE INDEX IF NOT EXISTS idx_oplog_timestamp ON operation_log(timestamp);
        CREATE INDEX IF NOT EXISTS idx_oplog_object ON operation_log(object_id);
        CREATE INDEX IF NOT EXISTS idx_oplog_status ON operation_log(status);

        CREATE TABLE IF NOT EXISTS object_embeddings (
            object_id TEXT NOT NULL REFERENCES objects(id) ON DELETE CASCADE,
            model TEXT NOT NULL,
            content_hash TEXT NOT NULL,
            vector TEXT NOT NULL,
            updated_at INTEGER NOT NULL,
            PRIMARY KEY (object_id, model)
        );
        CREATE INDEX IF NOT EXISTS idx_object_embeddings_model
            ON object_embeddings(model);

        CREATE TABLE IF NOT EXISTS trash (
            object_id TEXT PRIMARY KEY,
            original_path TEXT NOT NULL,
            trash_path TEXT NOT NULL,
            deleted_at INTEGER NOT NULL,
            auto_delete_at INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS collections (
            id TEXT PRIMARY KEY,
            parent_id TEXT REFERENCES collections(id) ON DELETE CASCADE,
            name TEXT NOT NULL,
            description TEXT NOT NULL DEFAULT '',
            created_by TEXT NOT NULL DEFAULT 'human',
            sort_order INTEGER NOT NULL DEFAULT 0,
            archived_at INTEGER,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL
        );
        CREATE UNIQUE INDEX IF NOT EXISTS idx_collections_root_name
            ON collections(name) WHERE parent_id IS NULL;
        CREATE UNIQUE INDEX IF NOT EXISTS idx_collections_child_name
            ON collections(parent_id, name) WHERE parent_id IS NOT NULL;
        CREATE INDEX IF NOT EXISTS idx_collections_parent ON collections(parent_id);

        CREATE TABLE IF NOT EXISTS object_collections (
            object_id TEXT NOT NULL REFERENCES objects(id) ON DELETE CASCADE,
            collection_id TEXT NOT NULL REFERENCES collections(id) ON DELETE CASCADE,
            is_primary INTEGER NOT NULL DEFAULT 0,
            created_at INTEGER NOT NULL,
            PRIMARY KEY (object_id, collection_id)
        );
        CREATE INDEX IF NOT EXISTS idx_object_collections_collection
            ON object_collections(collection_id);

        CREATE TABLE IF NOT EXISTS ai_tasks (
            id TEXT PRIMARY KEY,
            scope TEXT NOT NULL,
            instruction TEXT NOT NULL,
            provider TEXT NOT NULL,
            model TEXT NOT NULL,
            status TEXT NOT NULL,
            summary TEXT NOT NULL DEFAULT '',
            started_at INTEGER NOT NULL,
            finished_at INTEGER
        );

        CREATE TABLE IF NOT EXISTS ai_task_actions (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            task_id TEXT NOT NULL REFERENCES ai_tasks(id) ON DELETE CASCADE,
            action_type TEXT NOT NULL,
            object_id TEXT,
            collection_id TEXT,
            description TEXT NOT NULL,
            created_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_ai_task_actions_task ON ai_task_actions(task_id);

        CREATE TABLE IF NOT EXISTS concept_note_queue (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            object_id TEXT NOT NULL REFERENCES objects(id) ON DELETE CASCADE,
            content_hash TEXT NOT NULL,
            status TEXT NOT NULL DEFAULT 'pending',
            retry_count INTEGER NOT NULL DEFAULT 0,
            error_message TEXT,
            enqueued_at INTEGER NOT NULL,
            finished_at INTEGER
        );
        CREATE INDEX IF NOT EXISTS idx_concept_queue_status ON concept_note_queue(status);
        CREATE UNIQUE INDEX IF NOT EXISTS idx_concept_queue_object_pending
            ON concept_note_queue(object_id) WHERE status IN ('pending', 'running');
        ",
    )?;
    let relation_columns = conn
        .prepare("PRAGMA table_info(relations)")?
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    if !relation_columns.iter().any(|name| name == "source") {
        conn.execute(
            "ALTER TABLE relations ADD COLUMN source TEXT NOT NULL DEFAULT 'automatic'",
            [],
        )?;
    }
    // 历史上只有这三类关系明确由用户创建；其余旧记录按自动关系处理。
    conn.execute(
        "UPDATE relations SET source = 'manual' WHERE relation_type IN ('depends_on', 'part_of', 'implements')",
        [],
    )?;
    conn.execute(
        "INSERT INTO objects_fts_cjk(id, title, content, tags)
         SELECT f.id, f.title, f.content, f.tags FROM objects_fts f
         WHERE f.id NOT IN (SELECT id FROM objects_fts_cjk WHERE id IS NOT NULL)",
        [],
    )?;
    let has_explicit_column = conn
        .prepare("PRAGMA table_info(groups)")?
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<std::result::Result<Vec<_>, _>>()?
        .iter()
        .any(|name| name == "is_explicit");
    if !has_explicit_column {
        conn.execute(
            "ALTER TABLE groups ADD COLUMN is_explicit INTEGER NOT NULL DEFAULT 1",
            [],
        )?;
    }
    conn.execute(
        "INSERT OR IGNORE INTO groups (name, created_at, is_explicit)
         SELECT DISTINCT group_name, CAST(strftime('%s', 'now') AS INTEGER), 0
         FROM objects WHERE group_name != ''",
        [],
    )?;
    sync_legacy_collections(conn)?;
    conn.execute(
        "INSERT OR IGNORE INTO schema_version (version, applied_at)
         VALUES (7, CAST(strftime('%s', 'now') AS INTEGER))",
        [],
    )?;
    Ok(())
}

fn sync_legacy_collections(conn: &Connection) -> Result<()> {
    conn.execute(
        "INSERT INTO collections
            (id, parent_id, name, description, created_by, sort_order, created_at, updated_at)
         SELECT 'col_' || lower(hex(randomblob(8))), NULL, g.name, '', 'human', 0,
                CAST(strftime('%s', 'now') AS INTEGER), CAST(strftime('%s', 'now') AS INTEGER)
         FROM groups g
         WHERE g.name != ''
           AND NOT EXISTS (
               SELECT 1 FROM collections c WHERE c.parent_id IS NULL AND c.name = g.name
           )
           AND EXISTS (
               SELECT 1 FROM objects o
               WHERE o.group_name = g.name AND o.status != 'deleted'
                 AND NOT EXISTS (
                     SELECT 1 FROM object_collections oc WHERE oc.object_id = o.id
                 )
           )",
        [],
    )?;
    conn.execute(
        "INSERT OR IGNORE INTO object_collections (object_id, collection_id, is_primary, created_at)
         SELECT o.id, c.id, 1, CAST(strftime('%s', 'now') AS INTEGER)
         FROM objects o
         JOIN collections c ON c.parent_id IS NULL AND c.name = o.group_name
         WHERE o.group_name != '' AND o.status != 'deleted'
           AND NOT EXISTS (
               SELECT 1 FROM object_collections oc WHERE oc.object_id = o.id
           )",
        [],
    )?;
    Ok(())
}

// ===== 对象 CRUD =====

pub fn create_object(conn: &Connection, obj: &Object) -> Result<()> {
    if !obj.group_name.trim().is_empty() {
        ensure_group(conn, &obj.group_name)?;
    }
    conn.execute(
        "INSERT INTO objects (id, type, format, title, group_name, primary_file, file_hash, file_size, created_by, status, metadata, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
        params![
            obj.id,
            obj.r#type,
            obj.format,
            obj.title,
            obj.group_name,
            obj.primary_file,
            obj.file_hash,
            obj.file_size,
            obj.created_by,
            obj.status,
            obj.metadata,
            obj.created_at,
            obj.updated_at,
        ],
    )?;
    // 触发器已插入基础 FTS 记录（content 为空），手动更新 content
    conn.execute(
        "UPDATE objects_fts SET content = ?1 WHERE id = ?2",
        params!["", obj.id],
    )?;
    conn.execute(
        "UPDATE objects_fts_cjk SET content = ?1 WHERE id = ?2",
        params!["", obj.id],
    )?;
    let after = serde_json::to_string(obj).unwrap_or_default();
    log_operation(conn, "create_object", Some(&obj.id), None, Some(&after))?;
    Ok(())
}

pub fn get_object(conn: &Connection, id: &str) -> Result<Option<Object>> {
    let mut stmt = conn.prepare(
        "SELECT id, type, format, title, group_name, primary_file, file_hash, file_size, created_by, status, metadata, created_at, updated_at
         FROM objects WHERE id = ?1",
    )?;
    let mut rows = stmt.query(params![id])?;
    if let Some(row) = rows.next()? {
        return Ok(Some(Object {
            id: row.get(0)?,
            r#type: row.get(1)?,
            format: row.get(2)?,
            title: row.get(3)?,
            group_name: row.get(4)?,
            primary_file: row.get(5)?,
            file_hash: row.get(6)?,
            file_size: row.get(7)?,
            created_by: row.get(8)?,
            status: row.get(9)?,
            metadata: row.get(10)?,
            created_at: row.get(11)?,
            updated_at: row.get(12)?,
        }));
    }
    Ok(None)
}

pub fn list_objects(conn: &Connection) -> Result<Vec<Object>> {
    list_objects_with_status(conn, false)
}

pub fn list_all_objects(conn: &Connection) -> Result<Vec<Object>> {
    list_objects_with_status(conn, true)
}

fn list_objects_with_status(conn: &Connection, include_deleted: bool) -> Result<Vec<Object>> {
    let where_clause = if include_deleted {
        ""
    } else {
        " WHERE status != 'deleted'"
    };
    let sql = format!(
        "SELECT id, type, format, title, group_name, primary_file, file_hash, file_size, created_by, status, metadata, created_at, updated_at FROM objects{} ORDER BY updated_at DESC",
        where_clause
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], |row| {
        Ok(Object {
            id: row.get(0)?,
            r#type: row.get(1)?,
            format: row.get(2)?,
            title: row.get(3)?,
            group_name: row.get(4)?,
            primary_file: row.get(5)?,
            file_hash: row.get(6)?,
            file_size: row.get(7)?,
            created_by: row.get(8)?,
            status: row.get(9)?,
            metadata: row.get(10)?,
            created_at: row.get(11)?,
            updated_at: row.get(12)?,
        })
    })?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

pub fn update_object_content(
    conn: &mut Connection,
    id: &str,
    title: &str,
    content: &str,
    file_hash: Option<&str>,
    file_size: i64,
) -> Result<()> {
    // Object row, both FTS indexes, and the operation log must commit together.
    // The caller handles file/index.md rollback when this transaction fails.
    let tx = conn.transaction()?;
    let before = get_object(&tx, id)?.and_then(|obj| serde_json::to_string(&obj).ok());
    let before_content = tx
        .query_row(
            "SELECT content FROM objects_fts WHERE id = ?1",
            params![id],
            |row| row.get::<_, String>(0),
        )
        .unwrap_or_default();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    tx.execute(
        "UPDATE objects SET title = ?1, file_hash = ?2, file_size = ?3, updated_at = ?4 WHERE id = ?5",
        params![title, file_hash, file_size, now, id],
    )?;
    // 同步 FTS：title 和 content
    tx.execute(
        "UPDATE objects_fts SET content = ?1, title = ?2 WHERE id = ?3",
        params![content, title, id],
    )?;
    tx.execute(
        "UPDATE objects_fts_cjk SET content = ?1, title = ?2 WHERE id = ?3",
        params![content, title, id],
    )?;
    let after = get_object(&tx, id)?.and_then(|obj| serde_json::to_string(&obj).ok());
    log_operation_with_files(
        &tx,
        "update_content",
        Some(id),
        before.as_deref(),
        after.as_deref(),
        Some(&serde_json::json!({ "content": before_content }).to_string()),
    )?;
    tx.commit()?;
    Ok(())
}

pub fn update_object_group(conn: &Connection, id: &str, group: &str) -> Result<()> {
    if !group.trim().is_empty() {
        ensure_group(conn, group)?;
    }
    let before = get_object(conn, id)?.and_then(|obj| serde_json::to_string(&obj).ok());
    let old_group = get_object(conn, id)?
        .map(|obj| obj.group_name)
        .unwrap_or_default();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    conn.execute(
        "UPDATE objects SET group_name = ?1, updated_at = ?2 WHERE id = ?3",
        params![group, now, id],
    )?;
    if !old_group.is_empty() && old_group != group {
        conn.execute(
            "DELETE FROM groups
             WHERE name = ?1 AND is_explicit = 0
               AND NOT EXISTS (
                 SELECT 1 FROM objects WHERE group_name = ?1 AND status != 'deleted'
               )",
            params![old_group],
        )?;
    }
    let after = get_object(conn, id)?.and_then(|obj| serde_json::to_string(&obj).ok());
    log_operation(
        conn,
        "update_group",
        Some(id),
        before.as_deref(),
        after.as_deref(),
    )?;
    Ok(())
}

pub fn update_object_title(conn: &Connection, id: &str, title: &str) -> Result<()> {
    let before = get_object(conn, id)?.and_then(|obj| serde_json::to_string(&obj).ok());
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    conn.execute(
        "UPDATE objects SET title = ?1, updated_at = ?2 WHERE id = ?3",
        params![title, now, id],
    )?;
    let after = get_object(conn, id)?.and_then(|obj| serde_json::to_string(&obj).ok());
    log_operation(
        conn,
        "rename_object",
        Some(id),
        before.as_deref(),
        after.as_deref(),
    )?;
    Ok(())
}

pub fn update_object_creator(conn: &Connection, id: &str, created_by: &str) -> Result<()> {
    let before = get_object(conn, id)?.and_then(|obj| serde_json::to_string(&obj).ok());
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    conn.execute(
        "UPDATE objects SET created_by = ?1, updated_at = ?2 WHERE id = ?3",
        params![created_by, now, id],
    )?;
    let after = get_object(conn, id)?.and_then(|obj| serde_json::to_string(&obj).ok());
    log_operation(
        conn,
        "update_creator",
        Some(id),
        before.as_deref(),
        after.as_deref(),
    )?;
    Ok(())
}

pub fn update_object_metadata(conn: &Connection, id: &str, metadata: &str) -> Result<()> {
    let before = get_object(conn, id)?.and_then(|obj| serde_json::to_string(&obj).ok());
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    conn.execute(
        "UPDATE objects SET metadata = ?1, updated_at = ?2 WHERE id = ?3",
        params![metadata, now, id],
    )?;
    let after = get_object(conn, id)?.and_then(|obj| serde_json::to_string(&obj).ok());
    log_operation(
        conn,
        "update_metadata",
        Some(id),
        before.as_deref(),
        after.as_deref(),
    )?;
    Ok(())
}

pub fn delete_object(conn: &Connection, id: &str) -> Result<()> {
    let before = get_object(conn, id)?.and_then(|obj| serde_json::to_string(&obj).ok());
    // 软删除：将状态置为 deleted
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    conn.execute(
        "UPDATE objects SET status = 'deleted', updated_at = ?1 WHERE id = ?2",
        params![now, id],
    )?;
    let after = get_object(conn, id)?.and_then(|obj| serde_json::to_string(&obj).ok());
    log_operation(
        conn,
        "delete_object",
        Some(id),
        before.as_deref(),
        after.as_deref(),
    )?;
    Ok(())
}

/// 删除两个对象之间的关系
pub fn unlink_objects(
    conn: &Connection,
    from_id: &str,
    to_id: &str,
    relation_type: &str,
) -> Result<()> {
    conn.execute(
        "DELETE FROM relations WHERE source_id = ?1 AND target_id = ?2 AND relation_type = ?3",
        params![from_id, to_id, relation_type],
    )?;
    Ok(())
}

/// 恢复已删除对象（status: deleted -> draft）
pub fn restore_object(conn: &Connection, id: &str) -> Result<()> {
    let before = get_object(conn, id)?.and_then(|obj| serde_json::to_string(&obj).ok());
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    conn.execute(
        "UPDATE objects SET status = 'draft', updated_at = ?1 WHERE id = ?2",
        params![now, id],
    )?;
    let after = get_object(conn, id)?.and_then(|obj| serde_json::to_string(&obj).ok());
    log_operation(
        conn,
        "restore_object",
        Some(id),
        before.as_deref(),
        after.as_deref(),
    )?;
    Ok(())
}

/// 永久删除对象：清除所有相关数据库记录
pub fn permanent_delete_db(conn: &Connection, id: &str) -> Result<()> {
    let before = get_object(conn, id)?.and_then(|obj| serde_json::to_string(&obj).ok());
    conn.execute("DELETE FROM objects WHERE id = ?1", params![id])?;
    conn.execute("DELETE FROM tags WHERE object_id = ?1", params![id])?;
    conn.execute(
        "DELETE FROM relations WHERE source_id = ?1 OR target_id = ?1",
        params![id],
    )?;
    conn.execute("DELETE FROM objects_fts WHERE id = ?1", params![id])?;
    log_operation(conn, "permanent_delete", Some(id), before.as_deref(), None)?;
    Ok(())
}

// ===== 搜索 =====

pub fn search_objects(conn: &Connection, query: &str, limit: i64) -> Result<Vec<ObjectSummary>> {
    // 空查询返回空结果
    if query.trim().is_empty() {
        return Ok(Vec::new());
    }
    // MATCH 接收 FTS5 查询语法，而产品搜索框接收普通文本。将输入作为
    // 一个安全短语传入，避免 `-`、`*`、`OR` 等字符被解释为运算符。
    let normalized = query.trim().trim_matches('"');
    if normalized.is_empty() {
        return Ok(Vec::new());
    }
    let fts_query = format!("\"{}\"", normalized.replace('"', "\"\""));
    let has_cjk = normalized.chars().any(is_cjk_character);
    let mut out = if has_cjk && normalized.chars().count() >= 3 {
        search_cjk_trigram(conn, normalized, limit)?
    } else {
        Vec::new()
    };
    if !out.is_empty() {
        return Ok(out);
    }
    let mut stmt = conn.prepare(
        "SELECT f.id, f.title, o.type, o.format, bm25(objects_fts) AS score, snippet(objects_fts, 2, '<mark>', '</mark>', '...', 16) AS snippet, o.created_by
         FROM objects_fts f
         JOIN objects o ON o.id = f.id
         WHERE objects_fts MATCH ?1 AND o.status != 'deleted'
         ORDER BY score
         LIMIT ?2",
    )?;
    let rows = stmt.query_map(params![fts_query, limit.max(1)], |row| {
        Ok(ObjectSummary {
            id: row.get(0)?,
            title: row.get(1)?,
            r#type: row.get(2)?,
            format: row.get(3)?,
            score: row.get::<_, f64>(4)?,
            snippet: row.get::<_, Option<String>>(5)?.unwrap_or_default(),
            created_by: row.get(6)?,
        })
    })?;
    out.clear();
    for r in rows {
        out.push(r?);
    }
    if out.is_empty() {
        let like_query = format!("%{}%", escape_like(normalized));
        let mut stmt = conn.prepare(
            "SELECT f.id, f.title, o.type, o.format, 0.0 AS score, f.content AS snippet, o.created_by
             FROM objects_fts f
             JOIN objects o ON o.id = f.id
             WHERE o.status != 'deleted'
               AND (f.title LIKE ?1 ESCAPE '\\'
                    OR f.content LIKE ?1 ESCAPE '\\'
                    OR f.tags LIKE ?1 ESCAPE '\\')
             ORDER BY o.updated_at DESC
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![like_query, limit.max(1)], |row| {
            Ok(ObjectSummary {
                id: row.get(0)?,
                title: row.get(1)?,
                r#type: row.get(2)?,
                format: row.get(3)?,
                score: row.get::<_, f64>(4)?,
                snippet: row.get::<_, Option<String>>(5)?.unwrap_or_default(),
                created_by: row.get(6)?,
            })
        })?;
        for r in rows {
            out.push(r?);
        }
    }
    Ok(out)
}

fn is_cjk_character(ch: char) -> bool {
    matches!(
        ch as u32,
        0x3400..=0x4DBF
            | 0x4E00..=0x9FFF
            | 0xF900..=0xFAFF
            | 0x3040..=0x30FF
            | 0xAC00..=0xD7AF
    )
}

fn search_cjk_trigram(conn: &Connection, query: &str, limit: i64) -> Result<Vec<ObjectSummary>> {
    let fts_query = format!("\"{}\"", query.replace('"', "\"\""));
    let mut stmt = conn.prepare(
        "SELECT c.id, c.title, o.type, o.format, bm25(objects_fts_cjk) AS score,
                snippet(objects_fts_cjk, 2, '<mark>', '</mark>', '...', 24), o.created_by
         FROM objects_fts_cjk c
         JOIN objects o ON o.id = c.id
         WHERE objects_fts_cjk MATCH ?1 AND o.status != 'deleted'
         ORDER BY score
         LIMIT ?2",
    )?;
    let rows = stmt.query_map(params![fts_query, limit.max(1)], |row| {
        Ok(ObjectSummary {
            id: row.get(0)?,
            title: row.get(1)?,
            r#type: row.get(2)?,
            format: row.get(3)?,
            score: row.get(4)?,
            snippet: row.get::<_, Option<String>>(5)?.unwrap_or_default(),
            created_by: row.get(6)?,
        })
    })?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(Into::into)
}

fn escape_like(input: &str) -> String {
    let mut out = String::new();
    for ch in input.chars() {
        match ch {
            '%' | '_' | '\\' => {
                out.push('\\');
                out.push(ch);
            }
            _ => out.push(ch),
        }
    }
    out
}

// ===== 标签 =====

pub fn add_tags(conn: &Connection, object_id: &str, tags: &[String]) -> Result<()> {
    for tag in tags {
        conn.execute(
            "INSERT OR IGNORE INTO tags (object_id, tag) VALUES (?1, ?2)",
            params![object_id, tag],
        )?;
    }
    let joined = get_tags(conn, object_id)?.join(" ");
    conn.execute(
        "UPDATE objects_fts SET tags = ?1 WHERE id = ?2",
        params![joined, object_id],
    )?;
    conn.execute(
        "UPDATE objects_fts_cjk SET tags = ?1 WHERE id = ?2",
        params![joined, object_id],
    )?;
    Ok(())
}

pub fn replace_tags(conn: &Connection, object_id: &str, tags: &[String]) -> Result<()> {
    conn.execute("DELETE FROM tags WHERE object_id = ?1", params![object_id])?;
    add_tags(conn, object_id, tags)
}

pub fn get_object_embedding(
    conn: &Connection,
    object_id: &str,
    model: &str,
    content_hash: &str,
) -> Result<Option<Vec<f32>>> {
    let value = conn
        .query_row(
            "SELECT vector FROM object_embeddings
             WHERE object_id = ?1 AND model = ?2 AND content_hash = ?3",
            params![object_id, model, content_hash],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    value
        .map(|encoded| {
            serde_json::from_str(&encoded)
                .map_err(|error| crate::error::KbError::ParseFailed(error.to_string()))
        })
        .transpose()
}

pub fn upsert_object_embedding(
    conn: &Connection,
    object_id: &str,
    model: &str,
    content_hash: &str,
    vector: &[f32],
) -> Result<()> {
    let encoded = serde_json::to_string(vector)
        .map_err(|error| crate::error::KbError::ParseFailed(error.to_string()))?;
    conn.execute(
        "INSERT INTO object_embeddings (object_id, model, content_hash, vector, updated_at)
         VALUES (?1, ?2, ?3, ?4, CAST(strftime('%s', 'now') AS INTEGER))
         ON CONFLICT(object_id, model) DO UPDATE SET
           content_hash = excluded.content_hash,
           vector = excluded.vector,
           updated_at = excluded.updated_at",
        params![object_id, model, content_hash, encoded],
    )?;
    Ok(())
}

pub fn get_tags(conn: &Connection, object_id: &str) -> Result<Vec<String>> {
    let mut stmt = conn.prepare("SELECT tag FROM tags WHERE object_id = ?1 ORDER BY tag")?;
    let rows = stmt.query_map(params![object_id], |row| row.get::<_, String>(0))?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

/// 批量查询所有对象的标签，返回 (object_id -> Vec<tag>) 映射
/// 用于自动关联检测时避免 N+1 查询
pub fn get_all_tags_map(conn: &Connection) -> Result<HashMap<String, Vec<String>>> {
    let mut stmt = conn.prepare("SELECT object_id, tag FROM tags ORDER BY tag")?;
    let mut map: HashMap<String, Vec<String>> = HashMap::new();
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    for r in rows {
        let (oid, tag) = r?;
        map.entry(oid).or_default().push(tag);
    }
    Ok(map)
}

// ===== 关系 =====

pub fn link_objects(conn: &Connection, from: &str, to: &str, relation: &str) -> Result<()> {
    link_objects_with_source(conn, from, to, relation, "automatic")
}

pub fn link_objects_manual(conn: &Connection, from: &str, to: &str, relation: &str) -> Result<()> {
    link_objects_with_source(conn, from, to, relation, "manual")?;
    log_operation(
        conn,
        "link_objects",
        Some(from),
        Some(&serde_json::json!({"target_id": to, "relation_type": relation}).to_string()),
        None,
    )?;
    Ok(())
}

pub fn unlink_manual_relation(
    conn: &Connection,
    from: &str,
    to: &str,
    relation: &str,
) -> Result<()> {
    conn.execute(
        "DELETE FROM relations WHERE source_id = ?1 AND target_id = ?2 AND relation_type = ?3 AND source = 'manual'",
        params![from, to, relation],
    )?;
    log_operation(
        conn,
        "unlink_objects",
        Some(from),
        Some(&serde_json::json!({"target_id": to, "relation_type": relation}).to_string()),
        None,
    )?;
    Ok(())
}

fn link_objects_with_source(
    conn: &Connection,
    from: &str,
    to: &str,
    relation: &str,
    source: &str,
) -> Result<()> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    conn.execute(
        "INSERT INTO relations (source_id, target_id, relation_type, source, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(source_id, target_id, relation_type) DO UPDATE SET
           source = CASE WHEN excluded.source = 'manual' THEN 'manual' ELSE relations.source END",
        params![from, to, relation, source, now],
    )?;
    Ok(())
}

pub fn get_relations(conn: &Connection, id: &str) -> Result<Vec<Relation>> {
    let mut stmt = conn.prepare(
        "SELECT r.target_id, o.title, r.relation_type
         FROM relations r
         LEFT JOIN objects o ON o.id = r.target_id
         WHERE r.source_id = ?1
         ORDER BY r.created_at DESC",
    )?;
    let rows = stmt.query_map(params![id], |row| {
        Ok(Relation {
            target_id: row.get(0)?,
            target_title: row.get::<_, Option<String>>(1)?.unwrap_or_default(),
            relation_type: row.get(2)?,
        })
    })?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

/// 从指定对象出发，递归查询深度 N 的所有关联关系（BFS，避免递归深度过大）
pub fn get_relations_recursive(
    conn: &Connection,
    id: &str,
    max_depth: u32,
) -> Result<Vec<Relation>> {
    let mut visited = std::collections::HashSet::new();
    let mut queue = std::collections::VecDeque::new();
    let mut results = Vec::new();

    visited.insert(id.to_string());
    queue.push_back((id.to_string(), 0u32));

    while let Some((current, depth)) = queue.pop_front() {
        if depth >= max_depth {
            continue;
        }
        let mut stmt = conn.prepare(
            "SELECT r.target_id, o.title, r.relation_type
             FROM relations r
             LEFT JOIN objects o ON o.id = r.target_id
             WHERE r.source_id = ?1
             ORDER BY r.relation_type",
        )?;
        let rows = stmt.query_map(params![current], |row| {
            Ok(Relation {
                target_id: row.get(0)?,
                target_title: row.get::<_, Option<String>>(1)?.unwrap_or_default(),
                relation_type: row.get(2)?,
            })
        })?;
        for r in rows {
            let rel = r?;
            let target_id = rel.target_id.clone();
            let is_new = visited.insert(target_id.clone());
            results.push(rel);
            if is_new {
                queue.push_back((target_id, depth + 1));
            }
        }
    }
    Ok(results)
}

// ===== 统计 =====

pub fn get_stats(conn: &Connection) -> Result<VaultStats> {
    let total: i64 = conn.query_row(
        "SELECT COUNT(*) FROM objects WHERE status != 'deleted'",
        [],
        |row| row.get(0),
    )?;
    let mut stmt = conn.prepare(
        "SELECT group_name, COUNT(*) FROM objects WHERE status != 'deleted' GROUP BY group_name",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)? as usize))
    })?;
    let mut by_group: HashMap<String, usize> = HashMap::new();
    for r in rows {
        let (k, v) = r?;
        by_group.insert(k, v);
    }
    Ok(VaultStats {
        total_objects: total as usize,
        by_group,
    })
}

// ===== 文件树 =====

pub fn create_group(conn: &Connection, name: &str) -> Result<bool> {
    let created_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    let existed: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM groups WHERE name = ?1)",
        params![name],
        |row| row.get(0),
    )?;
    conn.execute(
        "INSERT INTO groups (name, created_at, is_explicit) VALUES (?1, ?2, 1)
         ON CONFLICT(name) DO UPDATE SET is_explicit = 1",
        params![name, created_at],
    )?;
    Ok(!existed)
}

fn ensure_group(conn: &Connection, name: &str) -> Result<()> {
    let created_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    conn.execute(
        "INSERT OR IGNORE INTO groups (name, created_at, is_explicit) VALUES (?1, ?2, 0)",
        params![name, created_at],
    )?;
    Ok(())
}

pub fn rename_group_entry(conn: &Connection, old_name: &str, new_name: &str) -> Result<()> {
    if new_name.trim().is_empty() {
        conn.execute("DELETE FROM groups WHERE name = ?1", params![old_name])?;
        return Ok(());
    }
    conn.execute(
        "INSERT INTO groups (name, created_at, is_explicit)
         SELECT ?1, created_at, is_explicit FROM groups WHERE name = ?2
         ON CONFLICT(name) DO UPDATE SET is_explicit = MAX(is_explicit, excluded.is_explicit)",
        params![new_name, old_name],
    )?;
    conn.execute("DELETE FROM groups WHERE name = ?1", params![old_name])?;
    Ok(())
}

pub fn delete_group_entry(conn: &Connection, name: &str) -> Result<()> {
    conn.execute("DELETE FROM groups WHERE name = ?1", params![name])?;
    Ok(())
}

pub fn get_file_tree(conn: &Connection) -> Result<Vec<GroupNode>> {
    let mut stmt = conn.prepare(
        "SELECT id, title, type, format, group_name, created_by
         FROM objects
         WHERE status != 'deleted'
         ORDER BY group_name ASC, updated_at DESC",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, String>(5)?,
        ))
    })?;
    let mut groups: HashMap<String, Vec<ObjectSummary>> = HashMap::new();
    let mut order: Vec<String> = conn
        .prepare("SELECT name FROM groups ORDER BY created_at ASC, name ASC")?
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    for name in &order {
        groups.insert(name.clone(), Vec::new());
    }
    for r in rows {
        let (id, title, otype, format, group_name, created_by) = r?;
        let key = if group_name.is_empty() {
            "未分组".to_string()
        } else {
            group_name
        };
        if !groups.contains_key(&key) {
            order.push(key.clone());
            groups.insert(key.clone(), Vec::new());
        }
        groups.get_mut(&key).unwrap().push(ObjectSummary {
            id,
            title,
            r#type: otype,
            format,
            score: 0.0,
            snippet: String::new(),
            created_by,
        });
    }
    let mut result: Vec<GroupNode> = Vec::new();
    for name in order {
        let objects = groups.remove(&name).unwrap_or_default();
        result.push(GroupNode { name, objects });
    }
    Ok(result)
}

#[derive(Clone)]
struct CollectionRow {
    id: String,
    parent_id: Option<String>,
    name: String,
    description: String,
    created_by: String,
    sort_order: i64,
}

fn collection_nodes_for_parent(
    parent_id: Option<&str>,
    parent_path: &str,
    rows: &[CollectionRow],
    objects: &mut HashMap<String, Vec<ObjectSummary>>,
    depth: usize,
) -> Vec<CollectionNode> {
    if depth > 64 {
        return Vec::new();
    }
    let mut children: Vec<CollectionRow> = rows
        .iter()
        .filter(|row| row.parent_id.as_deref() == parent_id)
        .cloned()
        .collect();
    children.sort_by(|a, b| {
        a.sort_order
            .cmp(&b.sort_order)
            .then_with(|| a.name.cmp(&b.name))
    });
    children
        .into_iter()
        .map(|row| {
            let path = if parent_path.is_empty() {
                row.name.clone()
            } else {
                format!("{}/{}", parent_path, row.name)
            };
            let nested =
                collection_nodes_for_parent(Some(&row.id), &path, rows, objects, depth + 1);
            CollectionNode {
                id: row.id.clone(),
                name: row.name,
                description: row.description,
                created_by: row.created_by,
                sort_order: row.sort_order,
                path,
                objects: objects.remove(&row.id).unwrap_or_default(),
                children: nested,
            }
        })
        .collect()
}

pub fn get_collection_tree(conn: &Connection) -> Result<Vec<CollectionNode>> {
    sync_legacy_collections(conn)?;
    let rows: Vec<CollectionRow> = conn
        .prepare(
            "SELECT id, parent_id, name, description, created_by, sort_order
             FROM collections WHERE archived_at IS NULL",
        )?
        .query_map([], |row| {
            Ok(CollectionRow {
                id: row.get(0)?,
                parent_id: row.get(1)?,
                name: row.get(2)?,
                description: row.get(3)?,
                created_by: row.get(4)?,
                sort_order: row.get(5)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;

    let mut objects: HashMap<String, Vec<ObjectSummary>> = HashMap::new();
    let mut stmt = conn.prepare(
        "SELECT oc.collection_id, o.id, o.title, o.type, o.format, o.created_by
         FROM object_collections oc
         JOIN objects o ON o.id = oc.object_id
         WHERE o.status != 'deleted'
         ORDER BY oc.is_primary DESC, o.updated_at DESC",
    )?;
    let memberships = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            ObjectSummary {
                id: row.get(1)?,
                title: row.get(2)?,
                r#type: row.get(3)?,
                format: row.get(4)?,
                score: 0.0,
                snippet: String::new(),
                created_by: row.get(5)?,
            },
        ))
    })?;
    for membership in memberships {
        let (collection_id, object) = membership?;
        objects.entry(collection_id).or_default().push(object);
    }
    Ok(collection_nodes_for_parent(
        None,
        "",
        &rows,
        &mut objects,
        0,
    ))
}

pub fn create_collection(
    conn: &Connection,
    name: &str,
    parent_id: Option<&str>,
    description: &str,
    created_by: &str,
) -> Result<String> {
    let name = name.trim();
    if name.is_empty() {
        return Err(crate::error::KbError::ParseFailed(
            "目录名称不能为空".to_string(),
        ));
    }
    if let Some(existing) = conn
        .query_row(
            "SELECT id FROM collections
             WHERE name = ?1 AND ((parent_id = ?2) OR (parent_id IS NULL AND ?2 IS NULL))",
            params![name, parent_id],
            |row| row.get::<_, String>(0),
        )
        .optional()?
    {
        if !description.trim().is_empty() {
            conn.execute(
                "UPDATE collections SET description = ?1, updated_at = CAST(strftime('%s', 'now') AS INTEGER)
                 WHERE id = ?2",
                params![description.trim(), existing],
            )?;
        }
        return Ok(existing);
    }
    if let Some(parent_id) = parent_id {
        let exists: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM collections WHERE id = ?1 AND archived_at IS NULL)",
            params![parent_id],
            |row| row.get(0),
        )?;
        if !exists {
            return Err(crate::error::KbError::NotFound(parent_id.to_string()));
        }
    }
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let id = format!("col_{nanos:x}");
    let now = (nanos / 1_000_000_000) as i64;
    conn.execute(
        "INSERT INTO collections
            (id, parent_id, name, description, created_by, sort_order, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, 0, ?6, ?6)",
        params![id, parent_id, name, description.trim(), created_by, now],
    )?;
    Ok(id)
}

pub fn collection_path(conn: &Connection, collection_id: &str) -> Result<String> {
    let mut id = Some(collection_id.to_string());
    let mut names = Vec::new();
    for _ in 0..64 {
        let Some(current) = id.take() else { break };
        let row = conn
            .query_row(
                "SELECT name, parent_id FROM collections WHERE id = ?1",
                params![current],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
            )
            .optional()?;
        let Some((name, parent_id)) = row else {
            return Err(crate::error::KbError::NotFound(collection_id.to_string()));
        };
        names.push(name);
        id = parent_id;
    }
    names.reverse();
    Ok(names.join("/"))
}

pub fn rename_collection(conn: &Connection, collection_id: &str, new_name: &str) -> Result<()> {
    let name = new_name.trim();
    if name.is_empty() {
        return Err(crate::error::KbError::ParseFailed(
            "目录名称不能为空".to_string(),
        ));
    }
    let parent_id: Option<String> = conn
        .query_row(
            "SELECT parent_id FROM collections WHERE id = ?1 AND archived_at IS NULL",
            params![collection_id],
            |row| row.get(0),
        )
        .optional()?
        .ok_or_else(|| crate::error::KbError::NotFound(collection_id.to_string()))?;
    let duplicate: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM collections WHERE name = ?1 AND ((parent_id = ?2) OR (parent_id IS NULL AND ?2 IS NULL)) AND id != ?3)",
        params![name, parent_id, collection_id],
        |row| row.get(0),
    )?;
    if duplicate {
        return Err(crate::error::KbError::AlreadyExists(name.to_string()));
    }
    conn.execute(
        "UPDATE collections SET name = ?1, updated_at = CAST(strftime('%s', 'now') AS INTEGER) WHERE id = ?2",
        params![name, collection_id],
    )?;
    Ok(())
}

pub fn delete_collection(conn: &Connection, collection_id: &str) -> Result<()> {
    let exists: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM collections WHERE id = ?1)",
        params![collection_id],
        |row| row.get(0),
    )?;
    if !exists {
        return Err(crate::error::KbError::NotFound(collection_id.to_string()));
    }
    conn.execute(
        "DELETE FROM collections WHERE id = ?1",
        params![collection_id],
    )?;
    Ok(())
}

pub fn assign_object_to_collection(
    conn: &Connection,
    object_id: &str,
    collection_id: &str,
    primary: bool,
) -> Result<()> {
    if primary {
        conn.execute(
            "DELETE FROM object_collections
             WHERE object_id = ?1 AND is_primary = 1 AND collection_id != ?2",
            params![object_id, collection_id],
        )?;
    }
    conn.execute(
        "INSERT INTO object_collections (object_id, collection_id, is_primary, created_at)
         VALUES (?1, ?2, ?3, CAST(strftime('%s', 'now') AS INTEGER))
         ON CONFLICT(object_id, collection_id) DO UPDATE SET is_primary = excluded.is_primary",
        params![object_id, collection_id, if primary { 1 } else { 0 }],
    )?;
    log_operation(
        conn,
        "assign_collection",
        Some(object_id),
        None,
        Some(collection_id),
    )?;
    Ok(())
}

pub fn create_ai_task(
    conn: &Connection,
    id: &str,
    scope: &str,
    instruction: &str,
    provider: &str,
    model: &str,
) -> Result<()> {
    conn.execute(
        "INSERT INTO ai_tasks
            (id, scope, instruction, provider, model, status, started_at)
         VALUES (?1, ?2, ?3, ?4, ?5, 'running', CAST(strftime('%s', 'now') AS INTEGER))",
        params![id, scope, instruction, provider, model],
    )?;
    Ok(())
}

pub fn add_ai_task_action(
    conn: &Connection,
    task_id: &str,
    action_type: &str,
    object_id: Option<&str>,
    collection_id: Option<&str>,
    description: &str,
) -> Result<()> {
    conn.execute(
        "INSERT INTO ai_task_actions
            (task_id, action_type, object_id, collection_id, description, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, CAST(strftime('%s', 'now') AS INTEGER))",
        params![task_id, action_type, object_id, collection_id, description],
    )?;
    Ok(())
}

pub fn finish_ai_task(conn: &Connection, task_id: &str, status: &str, summary: &str) -> Result<()> {
    conn.execute(
        "UPDATE ai_tasks SET status = ?1, summary = ?2,
            finished_at = CAST(strftime('%s', 'now') AS INTEGER)
         WHERE id = ?3",
        params![status, summary, task_id],
    )?;
    Ok(())
}

pub fn list_ai_tasks(conn: &Connection, limit: i64) -> Result<Vec<AiTask>> {
    let mut stmt = conn.prepare(
        "SELECT id, scope, instruction, provider, model, status, summary, started_at, finished_at
         FROM ai_tasks ORDER BY started_at DESC, id DESC LIMIT ?1",
    )?;
    let rows = stmt.query_map(params![limit.clamp(1, 100)], |row| {
        Ok(AiTask {
            id: row.get(0)?,
            scope: row.get(1)?,
            instruction: row.get(2)?,
            provider: row.get(3)?,
            model: row.get(4)?,
            status: row.get(5)?,
            summary: row.get(6)?,
            started_at: row.get(7)?,
            finished_at: row.get(8)?,
        })
    })?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(Into::into)
}

pub fn list_ai_task_actions(conn: &Connection, task_id: &str) -> Result<Vec<AiTaskAction>> {
    let mut stmt = conn.prepare(
        "SELECT id, task_id, action_type, object_id, collection_id, description, created_at
         FROM ai_task_actions WHERE task_id = ?1 ORDER BY id ASC",
    )?;
    let rows = stmt.query_map(params![task_id], |row| {
        Ok(AiTaskAction {
            id: row.get(0)?,
            task_id: row.get(1)?,
            action_type: row.get(2)?,
            object_id: row.get(3)?,
            collection_id: row.get(4)?,
            description: row.get(5)?,
            created_at: row.get(6)?,
        })
    })?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(Into::into)
}

pub fn recover_running_ai_tasks(conn: &Connection) -> Result<usize> {
    conn.execute(
        "UPDATE ai_tasks SET status = 'interrupted', summary = '应用已退出，任务未完成',
            finished_at = CAST(strftime('%s', 'now') AS INTEGER)
         WHERE status = 'running'",
        [],
    )
    .map_err(Into::into)
}

/// 删除单条已结束的 AI 任务（其动作记录由外键级联删除）。
/// 正在运行的任务不允许删除，返回 Ok(false)。
pub fn delete_ai_task(conn: &Connection, task_id: &str) -> Result<bool> {
    let affected = conn.execute(
        "DELETE FROM ai_tasks WHERE id = ?1 AND status != 'running'",
        params![task_id],
    )?;
    Ok(affected > 0)
}

/// 清空所有已结束的 AI 任务（保留正在运行的）。返回删除条数。
pub fn clear_finished_ai_tasks(conn: &Connection) -> Result<usize> {
    conn.execute("DELETE FROM ai_tasks WHERE status != 'running'", [])
        .map(|n| n as usize)
        .map_err(Into::into)
}

pub fn prune_empty_collections(conn: &Connection) -> Result<usize> {
    let mut total = 0;
    loop {
        let deleted = conn.execute(
            "DELETE FROM collections
             WHERE archived_at IS NULL
               AND NOT EXISTS (
                   SELECT 1 FROM object_collections oc WHERE oc.collection_id = collections.id
               )
               AND NOT EXISTS (
                   SELECT 1 FROM collections child WHERE child.parent_id = collections.id
               )",
            [],
        )?;
        total += deleted;
        if deleted == 0 {
            break;
        }
    }
    Ok(total)
}

// ===== 操作日志 =====

/// 记录操作日志
pub fn log_operation(
    conn: &Connection,
    operation: &str,
    object_id: Option<&str>,
    before_state: Option<&str>,
    after_state: Option<&str>,
) -> Result<i64> {
    log_operation_with_files(conn, operation, object_id, before_state, after_state, None)
}

pub fn log_operation_with_files(
    conn: &Connection,
    operation: &str,
    object_id: Option<&str>,
    before_state: Option<&str>,
    after_state: Option<&str>,
    file_changes: Option<&str>,
) -> Result<i64> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    conn.execute(
        "INSERT INTO operation_log
            (timestamp, operation, object_id, before_state, after_state, file_changes, status)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'success')",
        params![
            now,
            operation,
            object_id,
            before_state,
            after_state,
            file_changes
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

pub type ReversibleOperation = (i64, String, Option<String>, Option<String>, Option<String>);

pub fn get_last_reversible_operation(conn: &Connection) -> Result<Option<ReversibleOperation>> {
    let operations = [
        "create_object",
        "update_content",
        "update_group",
        "rename_object",
        "update_creator",
        "update_metadata",
        "delete_object",
        "restore_object",
        "add_tags",
        "link_objects",
    ];
    let placeholders = std::iter::repeat_n("?", operations.len())
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!(
        "SELECT id, operation, object_id, before_state, file_changes
         FROM operation_log
         WHERE status = 'success' AND operation IN ({placeholders})
           AND (operation != 'update_content' OR file_changes IS NOT NULL)
         ORDER BY id DESC LIMIT 1"
    );
    let mut stmt = conn.prepare(&sql)?;
    let params = rusqlite::params_from_iter(operations);
    let mut rows = stmt.query(params)?;
    if let Some(row) = rows.next()? {
        Ok(Some((
            row.get(0)?,
            row.get(1)?,
            row.get(2)?,
            row.get(3)?,
            row.get(4)?,
        )))
    } else {
        Ok(None)
    }
}

pub fn max_operation_id(conn: &Connection) -> Result<i64> {
    conn.query_row(
        "SELECT COALESCE(MAX(id), 0) FROM operation_log",
        [],
        |row| row.get(0),
    )
    .map_err(Into::into)
}

pub fn mark_operation_rolled_back(
    conn: &Connection,
    operation_id: i64,
    generated_from: i64,
    generated_after: i64,
    description: &str,
) -> Result<()> {
    conn.execute(
        "UPDATE operation_log SET status = 'rolled_back' WHERE id = ?1",
        params![operation_id],
    )?;
    conn.execute(
        "UPDATE operation_log SET status = 'rollback', rollback_id = ?1
         WHERE id > ?2 AND id <= ?3 AND status = 'success'",
        params![operation_id, generated_from, generated_after],
    )?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0);
    conn.execute(
        "INSERT INTO operation_log
            (timestamp, operation, after_state, status, rollback_id)
         VALUES (?1, 'undo', ?2, 'success', ?3)",
        params![now, description, operation_id],
    )?;
    Ok(())
}

/// 获取最近的操作日志
pub fn get_recent_operations(conn: &Connection, limit: i64) -> Result<Vec<serde_json::Value>> {
    let mut stmt = conn.prepare(
        "SELECT id, timestamp, operation, object_id, status, error_message
         FROM operation_log
         ORDER BY timestamp DESC
         LIMIT ?1",
    )?;
    let rows = stmt.query_map(params![limit], |row| {
        Ok(serde_json::json!({
            "id": row.get::<_, i64>(0)?,
            "timestamp": row.get::<_, i64>(1)?,
            "operation": row.get::<_, String>(2)?,
            "object_id": row.get::<_, Option<String>>(3)?,
            "status": row.get::<_, String>(4)?,
            "error": row.get::<_, Option<String>>(5)?,
        }))
    })?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

pub fn get_object_versions(
    conn: &Connection,
    object_id: &str,
    limit: i64,
) -> Result<Vec<crate::model::ObjectVersion>> {
    let mut stmt = conn.prepare(
        "SELECT id, timestamp, before_state, file_changes
         FROM operation_log
         WHERE object_id = ?1 AND operation = 'update_content'
           AND file_changes IS NOT NULL AND status IN ('success', 'rolled_back')
         ORDER BY id DESC LIMIT ?2",
    )?;
    let rows = stmt.query_map(params![object_id, limit.clamp(1, 100)], |row| {
        let before_state = row.get::<_, Option<String>>(2)?.unwrap_or_default();
        let file_changes = row.get::<_, Option<String>>(3)?.unwrap_or_default();
        let title = serde_json::from_str::<serde_json::Value>(&before_state)
            .ok()
            .and_then(|value| {
                value
                    .get("title")
                    .and_then(|item| item.as_str())
                    .map(str::to_string)
            })
            .unwrap_or_default();
        let content = serde_json::from_str::<serde_json::Value>(&file_changes)
            .ok()
            .and_then(|value| {
                value
                    .get("content")
                    .and_then(|item| item.as_str())
                    .map(str::to_string)
            })
            .unwrap_or_default();
        Ok(crate::model::ObjectVersion {
            operation_id: row.get(0)?,
            timestamp: row.get(1)?,
            title,
            content,
        })
    })?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(Into::into)
}

pub fn get_object_version(
    conn: &Connection,
    object_id: &str,
    operation_id: i64,
) -> Result<crate::model::ObjectVersion> {
    get_object_versions(conn, object_id, 100)?
        .into_iter()
        .find(|version| version.operation_id == operation_id)
        .ok_or_else(|| crate::error::KbError::NotFound(operation_id.to_string()))
}

/// 清理过期操作日志（默认保留 30 天成功记录，90 天失败记录）
pub fn cleanup_operation_log(conn: &Connection) -> Result<i64> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let thirty_days = 30 * 24 * 3600;
    let ninety_days = 90 * 24 * 3600;
    let deleted = conn.execute(
        "DELETE FROM operation_log WHERE
            (status = 'success' AND ?1 - timestamp > ?2) OR
            (status = 'failed' AND ?1 - timestamp > ?3)",
        params![now, thirty_days, ninety_days],
    )?;
    Ok(deleted as i64)
}

// ===== 一致性检查 =====

/// 检查数据库中的对象是否有对应的文件目录（孤儿对象）
pub fn find_orphan_objects(conn: &Connection, vault_path: &Path) -> Result<Vec<String>> {
    let mut stmt = conn.prepare("SELECT id FROM objects WHERE status != 'deleted'")?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
    let mut orphans = Vec::new();
    for r in rows {
        let id = r?;
        let dir = vault_path.join("objects").join(&id);
        if !dir.exists() {
            orphans.push(id);
        }
    }
    Ok(orphans)
}

/// 检查文件系统中是否有对象目录但数据库中没有记录（孤儿文件）
pub fn find_orphan_files(conn: &Connection, vault_path: &Path) -> Result<Vec<String>> {
    let objects_dir = vault_path.join("objects");
    if !objects_dir.exists() {
        return Ok(Vec::new());
    }
    let mut orphans = Vec::new();
    let mut known_ids = std::collections::HashSet::new();
    let mut stmt = conn.prepare("SELECT id FROM objects")?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
    for r in rows {
        known_ids.insert(r?);
    }
    if let Ok(entries) = std::fs::read_dir(&objects_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy().to_string();
            if !known_ids.contains(&name_str) {
                orphans.push(name_str);
            }
        }
    }
    Ok(orphans)
}

/// 检查文件 hash 是否变化（需要重新索引）
pub fn find_changed_files(conn: &Connection, vault_path: &Path) -> Result<Vec<String>> {
    use sha2::{Digest, Sha256};
    let mut stmt = conn.prepare(
        "SELECT id, primary_file, file_hash FROM objects WHERE status != 'deleted' AND primary_file IS NOT NULL AND file_hash IS NOT NULL",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    })?;
    let mut changed = Vec::new();
    for r in rows {
        let (id, primary_file, stored_hash) = r?;
        let file_path = vault_path.join("objects").join(&id).join(&primary_file);
        if !file_path.exists() {
            continue;
        }
        if let Ok(mut file) = std::fs::File::open(&file_path) {
            let mut hasher = Sha256::new();
            if std::io::copy(&mut file, &mut hasher).is_ok() {
                let current_hash = format!("sha256:{}", hex::encode(hasher.finalize()));
                if current_hash != stored_hash {
                    changed.push(id);
                }
            }
        }
    }
    Ok(changed)
}

// ===== 数据库备份 =====

/// 备份数据库到指定路径
/// 保留最近 7 个备份，自动清理旧的
pub fn backup_database(db_path: &Path, vault_path: &Path) -> Result<PathBuf> {
    let backup_dir = vault_path.join(".kb").join("backups");
    std::fs::create_dir_all(&backup_dir)?;

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let backup_name = format!("index_{}.db", now);
    let backup_path = backup_dir.join(&backup_name);

    // 使用 SQLite 的 backup API 进行在线备份
    let temp_path = backup_path.with_extension("db.tmp");
    {
        let source = Connection::open(db_path)?;
        let mut dest = Connection::open(&temp_path)?;
        let backup = rusqlite::backup::Backup::new(&source, &mut dest)?;
        backup.run_to_completion(256, std::time::Duration::from_millis(10), None)?;
    }
    // 仅完整备份使用 .db 后缀，避免中断后的半成品被视为当天已备份。
    std::fs::rename(&temp_path, &backup_path)?;

    // 清理旧备份，保留最近 7 个
    let mut backups: Vec<_> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&backup_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with("index_") && name.ends_with(".db") {
                if let Ok(meta) = entry.metadata() {
                    backups.push((
                        meta.modified().unwrap_or(std::time::SystemTime::UNIX_EPOCH),
                        name,
                    ));
                }
            }
        }
    }
    backups.sort_by_key(|item| std::cmp::Reverse(item.0)); // 最新的在前
    for (_, name) in backups.iter().skip(7) {
        let _ = std::fs::remove_file(backup_dir.join(name));
    }

    Ok(backup_path)
}

// ===== 影子笔记（概念笔记）生成队列 =====

/// 入队一个概念笔记生成任务。
/// 如果该对象已有 pending/running 状态的任务，则跳过（唯一索引保证）。
pub fn enqueue_concept_note(
    conn: &Connection,
    object_id: &str,
    content_hash: &str,
) -> Result<()> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    conn.execute(
        "INSERT OR IGNORE INTO concept_note_queue (object_id, content_hash, status, enqueued_at)
         VALUES (?1, ?2, 'pending', ?3)",
        params![object_id, content_hash, now],
    )?;
    Ok(())
}

/// 取下一个 pending 任务，标记为 running。
/// 返回完整的任务信息（含对象标题和格式）。
pub fn claim_next_concept_note(conn: &Connection) -> Result<Option<ConceptNoteTask>> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    // 原子地取一个 pending 并标记为 running
    let tx = conn.unchecked_transaction()?;
    let task = tx
        .query_row(
            "SELECT q.id, q.object_id, q.content_hash, q.retry_count,
                    q.error_message, q.enqueued_at,
                    o.title, o.format
             FROM concept_note_queue q
             JOIN objects o ON o.id = q.object_id
             WHERE q.status = 'pending'
             ORDER BY q.enqueued_at ASC
             LIMIT 1",
            [],
            |row| {
                Ok(ConceptNoteTask {
                    id: row.get(0)?,
                    object_id: row.get(1)?,
                    content_hash: row.get(2)?,
                    status: "running".to_string(),
                    retry_count: row.get(3)?,
                    error_message: row.get(4)?,
                    enqueued_at: row.get(5)?,
                    finished_at: None,
                    object_title: row.get(6)?,
                    object_format: row.get(7)?,
                })
            },
        )
        .optional()?;
    if let Some(mut task) = task {
        tx.execute(
            "UPDATE concept_note_queue SET status = 'running' WHERE id = ?1",
            params![task.id],
        )?;
        tx.commit()?;
        task.finished_at = Some(now);
        Ok(Some(task))
    } else {
        // 显式回滚以释放事务（虽然没有任何修改，但保持一致性）
        let _ = tx.rollback();
        Ok(None)
    }
}

/// 标记任务完成。
pub fn finish_concept_note(conn: &Connection, task_id: i64) -> Result<()> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    conn.execute(
        "UPDATE concept_note_queue SET status = 'done', finished_at = ?1 WHERE id = ?2",
        params![now, task_id],
    )?;
    Ok(())
}

/// 标记任务失败，递增重试计数。
/// retry_count 达到上限（3）时标记为 failed，否则回退为 pending。
pub fn fail_concept_note(
    conn: &Connection,
    task_id: i64,
    error: &str,
    max_retries: i64,
) -> Result<bool> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let retry_count: i64 = conn.query_row(
        "SELECT retry_count FROM concept_note_queue WHERE id = ?1",
        params![task_id],
        |row| row.get(0),
    )?;
    let new_retry = retry_count + 1;
    if new_retry >= max_retries {
        conn.execute(
            "UPDATE concept_note_queue
             SET status = 'failed', error_message = ?1, finished_at = ?2
             WHERE id = ?3",
            params![error, now, task_id],
        )?;
        Ok(true) // 最终失败
    } else {
        conn.execute(
            "UPDATE concept_note_queue
             SET status = 'pending', retry_count = ?1, error_message = ?2
             WHERE id = ?3",
            params![new_retry, error, task_id],
        )?;
        Ok(false) // 仍可重试
    }
}

/// 重置 failed 任务为 pending（用户手动重试）。
pub fn retry_concept_notes(conn: &Connection, task_ids: &[i64]) -> Result<usize> {
    let json_ids = serde_json::to_string(task_ids)
        .map_err(|e| KbError::ParseFailed(e.to_string()))?;
    let count = conn.execute(
        "UPDATE concept_note_queue
         SET status = 'pending', retry_count = 0, error_message = NULL
         WHERE id IN (SELECT value FROM json_each(?1)) AND status = 'failed'",
        params![json_ids],
    )?;
    Ok(count)
}

/// 暂停所有 running 任务（回退为 pending）。
pub fn pause_running_concept_notes(conn: &Connection) -> Result<usize> {
    let count = conn.execute(
        "UPDATE concept_note_queue SET status = 'pending' WHERE status = 'running'",
        [],
    )?;
    Ok(count)
}

/// 查询队列状态（按状态过滤）。
pub fn list_concept_notes(
    conn: &Connection,
    status_filter: Option<&str>,
    limit: i64,
) -> Result<Vec<ConceptNoteTask>> {
    let (sql, params_values): (&str, Vec<Box<dyn rusqlite::ToSql>>) = if let Some(status) =
        status_filter
    {
        (
            "SELECT q.id, q.object_id, q.content_hash, q.status, q.retry_count,
                    q.error_message, q.enqueued_at, q.finished_at,
                    o.title, o.format
             FROM concept_note_queue q
             JOIN objects o ON o.id = q.object_id
             WHERE q.status = ?1
             ORDER BY q.enqueued_at DESC
             LIMIT ?2",
            vec![Box::new(status.to_string()), Box::new(limit)],
        )
    } else {
        (
            "SELECT q.id, q.object_id, q.content_hash, q.status, q.retry_count,
                    q.error_message, q.enqueued_at, q.finished_at,
                    o.title, o.format
             FROM concept_note_queue q
             JOIN objects o ON o.id = q.object_id
             ORDER BY q.enqueued_at DESC
             LIMIT ?1",
            vec![Box::new(limit)],
        )
    };
    let mut stmt = conn.prepare(sql)?;
    let param_refs: Vec<&dyn rusqlite::ToSql> = params_values.iter().map(|p| p.as_ref()).collect();
    let rows = stmt.query_map(param_refs.as_slice(), |row| {
        Ok(ConceptNoteTask {
            id: row.get(0)?,
            object_id: row.get(1)?,
            content_hash: row.get(2)?,
            status: row.get(3)?,
            retry_count: row.get(4)?,
            error_message: row.get(5)?,
            enqueued_at: row.get(6)?,
            finished_at: row.get(7)?,
            object_title: row.get(8)?,
            object_format: row.get(9)?,
        })
    })?;
    rows.collect::<std::result::Result<Vec<_>, _>>().map_err(Into::into)
}

/// 统计各状态数量。
pub fn count_concept_notes(conn: &Connection) -> Result<(i64, i64, i64, i64)> {
    let get_count = |status: &str| -> i64 {
        conn.query_row(
            "SELECT COUNT(*) FROM concept_note_queue WHERE status = ?1",
            params![status],
            |row| row.get(0),
        )
        .unwrap_or(0)
    };
    Ok((
        get_count("pending"),
        get_count("running"),
        get_count("done"),
        get_count("failed"),
    ))
}

/// 清理已完成的任务（保留最近 N 条）。
pub fn cleanup_done_concept_notes(conn: &Connection, keep: i64) -> Result<usize> {
    let count = conn.execute(
        "DELETE FROM concept_note_queue
         WHERE status = 'done' AND id NOT IN (
             SELECT id FROM concept_note_queue WHERE status = 'done'
             ORDER BY finished_at DESC LIMIT ?1
         )",
        params![keep],
    )?;
    Ok(count)
}

/// 恢复 running 状态的任务为 pending（应用启动时调用）。
pub fn recover_concept_notes(conn: &Connection) -> Result<usize> {
    let count = conn.execute(
        "UPDATE concept_note_queue SET status = 'pending' WHERE status = 'running'",
        [],
    )?;
    Ok(count)
}

/// 检查对象是否已有完成的概念笔记（通过 content_hash 判断是否需要重新生成）。
pub fn get_concept_note_for_object(
    conn: &Connection,
    object_id: &str,
) -> Result<Option<(i64, String, String)>> {
    conn.query_row(
        "SELECT id, content_hash, status FROM concept_note_queue
         WHERE object_id = ?1 AND status = 'done'
         ORDER BY finished_at DESC LIMIT 1",
        params![object_id],
        |row| Ok((row.get(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?)),
    )
    .optional()
    .map_err(Into::into)
}
