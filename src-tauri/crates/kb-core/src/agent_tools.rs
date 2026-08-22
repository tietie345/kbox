// 共享 Knowledge Tool Registry
//
// 统一的工具定义 + 执行入口，供 kb-mcp 与内置 AI Agent 共用。
// 目标：让 kb-mcp 只做 JSON-RPC 协议转换，不再自己维护一份 `match name`；
// 后续内置 AI 的多轮 Tool Calling 也走同一份工具与执行语义。
//
// 行为约束：
// - 工具执行层必须同时完成 SQLite、FTS、objects/{id}/index.md 与操作日志的更新，
//   不允许 Agent 绕过 KnowledgeBase 只改数据库。
// - 只读 / 全局工具（search、get_object、list_objects、list_groups、list_tags 等）不受作用域限制。
// - 面向单个对象的写工具（update_content、delete_object、link_objects）必须经 TaskScope 校验，
//   越界即拒绝。

use crate::db;
use crate::ingest::KnowledgeBase;
use crate::model::CollectionNode;
use rusqlite::params;
use serde_json::{json, Value};
use std::collections::HashSet;

/// 工具元定义。`input_schema` 是 MCP tools/list 的 `inputSchema`，OpenAI-compatible 的 tool 定义也复用。
#[derive(Debug, Clone)]
pub struct ToolDefinition {
    pub name: &'static str,
    pub description: &'static str,
    pub input_schema: Value,
}

/// 一次工具调用：工具名 + JSON 参数。
#[derive(Debug, Clone)]
pub struct ToolCall {
    pub name: String,
    pub arguments: Value,
}

/// 工具执行结果。`content` 为 MCP content text 兼容形态，`is_error` 标记失败。
#[derive(Debug, Clone)]
pub struct ToolResult {
    pub content: String,
    pub is_error: bool,
}

impl ToolResult {
    pub fn ok(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            is_error: false,
        }
    }
    pub fn err(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            is_error: true,
        }
    }
}

/// 工具授权作用域。内置整理任务必须携带，MCP / CLI 默认整库无限制。
#[derive(Debug, Clone)]
pub enum TaskScope {
    /// 整个知识库（MCP / CLI 默认路径，不限对象）
    KnowledgeBase,
    /// 限定某个 Collection 子树（含后代），越界对象拒绝
    Collection { id: String },
}

impl TaskScope {
    /// 是否不限对象
    pub fn is_global(&self) -> bool {
        matches!(self, TaskScope::KnowledgeBase)
    }

    /// 解析 Collection scope 的子树对象 id 集合；全局返回 None。
    fn collection_object_ids(&self, kb: &KnowledgeBase) -> Result<Option<HashSet<String>>, String> {
        let id = match self {
            TaskScope::KnowledgeBase => return Ok(None),
            TaskScope::Collection { id } => id.clone(),
        };
        let tree = kb.get_collection_tree().map_err(|e| e.to_string())?;

        fn find<'a>(node: &'a CollectionNode, id: &str) -> Option<&'a CollectionNode> {
            if node.id == id {
                return Some(node);
            }
            node.children.iter().find_map(|c| find(c, id))
        }
        fn collect(node: &CollectionNode, ids: &mut HashSet<String>) {
            ids.extend(node.objects.iter().map(|o| o.id.clone()));
            for child in &node.children {
                collect(child, ids);
            }
        }

        let root = tree
            .iter()
            .find_map(|n| find(n, &id))
            .ok_or_else(|| format!("目录不存在或已被删除: {id}"))?;
        let mut ids = HashSet::new();
        collect(root, &mut ids);
        Ok(Some(ids))
    }

    /// 校验对象是否在作用域内；越界返回 Err。全局作用域恒通过。
    pub fn ensure_object_in_scope(
        &self,
        kb: &KnowledgeBase,
        object_id: &str,
    ) -> Result<(), String> {
        let Some(ids) = self.collection_object_ids(kb)? else {
            return Ok(());
        };
        if ids.contains(object_id) {
            Ok(())
        } else {
            Err(format!("对象 {object_id} 不在当前整理作用域内"))
        }
    }
}

/// 全部工具的元定义（与旧 MCP 完全同名、同参数、同返回）。
pub fn tool_definitions() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            name: "ingest_file",
            description: "将本地文件入库到知识库，支持 MD/PDF/代码/图片等多种格式",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "file_path": { "type": "string", "description": "本地文件路径" },
                    "group": { "type": "string", "description": "分组名（可选）" }
                },
                "required": ["file_path"]
            }),
        },
        ToolDefinition {
            name: "ingest_url",
            description: "将远程 URL 内容入库到知识库（占位，后续实现）",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "url": { "type": "string", "description": "远程 URL" },
                    "group": { "type": "string", "description": "分组名（可选）" }
                },
                "required": ["url"]
            }),
        },
        ToolDefinition {
            name: "create_note",
            description: "创建新笔记",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "title": { "type": "string" },
                    "obj_type": { "type": "string", "description": "concept/project/code/document/inbox" },
                    "group": { "type": "string" }
                },
                "required": ["title", "obj_type", "group"]
            }),
        },
        ToolDefinition {
            name: "search",
            description: "全文搜索知识库",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string" },
                    "limit": { "type": "integer", "default": 20 }
                },
                "required": ["query"]
            }),
        },
        ToolDefinition {
            name: "get_object",
            description: "获取对象详情和内容",
            input_schema: json!({
                "type": "object",
                "properties": { "id": { "type": "string" } },
                "required": ["id"]
            }),
        },
        ToolDefinition {
            name: "update_content",
            description: "更新对象内容",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string" },
                    "title": { "type": "string" },
                    "content": { "type": "string" }
                },
                "required": ["id", "title", "content"]
            }),
        },
        ToolDefinition {
            name: "delete_object",
            description: "软删除对象",
            input_schema: json!({
                "type": "object",
                "properties": { "id": { "type": "string" } },
                "required": ["id"]
            }),
        },
        ToolDefinition {
            name: "list_objects",
            description: "列出所有对象或指定分组的对象",
            input_schema: json!({
                "type": "object",
                "properties": { "group": { "type": "string" } }
            }),
        },
        ToolDefinition {
            name: "get_stats",
            description: "获取知识库统计信息",
            input_schema: json!({ "type": "object", "properties": {} }),
        },
        ToolDefinition {
            name: "link_objects",
            description: "建立两个对象之间的关系",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "from_id": { "type": "string" },
                    "to_id": { "type": "string" },
                    "relation_type": { "type": "string", "description": "depends_on/related_to/part_of/implements" }
                },
                "required": ["from_id", "to_id", "relation_type"]
            }),
        },
        ToolDefinition {
            name: "get_relations",
            description: "查询某个对象的所有关联关系（用于 AI 顺着关系图谱探索相关信息，类似 Obsidian 的反向链接和出链）",
            input_schema: json!({
                "type": "object",
                "properties": { "id": { "type": "string" } },
                "required": ["id"]
            }),
        },
        ToolDefinition {
            name: "rebuild_relations",
            description: "重新扫描所有对象的内容，自动建立 [[标题]] 双向链接和标题提及关系。当用户觉得关系缺失或不准时调用。",
            input_schema: json!({ "type": "object", "properties": {} }),
        },
        ToolDefinition {
            name: "get_incoming_relations",
            description: "查询哪些对象引用了指定对象（反向关系/入链，类似 Obsidian 的 backlinks）。AI 探索关系图时有用。",
            input_schema: json!({
                "type": "object",
                "properties": { "id": { "type": "string" } },
                "required": ["id"]
            }),
        },
        ToolDefinition {
            name: "list_groups",
            description: "列出所有分组及其对象数量，作为遍历知识库的入口索引。",
            input_schema: json!({ "type": "object", "properties": {} }),
        },
        ToolDefinition {
            name: "list_tags",
            description: "列出所有标签及其命中对象数量，作为按主题遍历的索引。",
            input_schema: json!({ "type": "object", "properties": {} }),
        },
        ToolDefinition {
            name: "get_object_preview",
            description: "获取对象的轻量预览（标题、分组、标签、关系数、内容前 300 字），AI 遍历时用这个避免 token 爆炸，决定是否要 get_object 读全文。",
            input_schema: json!({
                "type": "object",
                "properties": { "id": { "type": "string" } },
                "required": ["id"]
            }),
        },
    ]
}

/// 生成 MCP tools/list 所需结构：`{ "tools": [ { name, description, inputSchema }, ... ] }`
pub fn tools_list_json() -> Value {
    json!({
        "tools": tool_definitions().iter().map(|d| json!({
            "name": d.name,
            "description": d.description,
            "inputSchema": d.input_schema,
        })).collect::<Vec<_>>()
    })
}

// ---------------------------------------------------------------------------
// 执行层：所有工具通过 execute_tool 统一分发
// ---------------------------------------------------------------------------

/// 执行一次工具调用。全局/只读工具行为与旧 MCP 一致；
/// 对面向单个对象的写工具做 TaskScope 作用域校验。
pub fn execute_tool(kb: &KnowledgeBase, call: &ToolCall, scope: &TaskScope) -> ToolResult {
    let result = match call.name.as_str() {
        "ingest_file" => tool_ingest_file(kb, &call.arguments),
        "ingest_url" => tool_ingest_url(&call.arguments),
        "create_note" => tool_create_note(kb, &call.arguments),
        "search" => tool_search(kb, &call.arguments),
        "get_object" => tool_get_object(kb, &call.arguments),
        "update_content" => tool_update_content(kb, &call.arguments, scope),
        "delete_object" => tool_delete_object(kb, &call.arguments, scope),
        "list_objects" => tool_list_objects(kb, &call.arguments),
        "get_stats" => tool_get_stats(kb),
        "link_objects" => tool_link_objects(kb, &call.arguments, scope),
        "get_relations" => tool_get_relations(kb, &call.arguments),
        "rebuild_relations" => tool_rebuild_relations(kb),
        "get_incoming_relations" => tool_get_incoming_relations(kb, &call.arguments),
        "list_groups" => tool_list_groups(kb),
        "list_tags" => tool_list_tags(kb),
        "get_object_preview" => tool_get_object_preview(kb, &call.arguments),
        other => Err(format!("未知工具: {other}")),
    };
    match result {
        Ok(content) => ToolResult::ok(content),
        Err(content) => ToolResult::err(content),
    }
}

type ToolOut = Result<String, String>;

fn tool_ingest_file(kb: &KnowledgeBase, args: &Value) -> ToolOut {
    let file_path = args["file_path"].as_str().unwrap_or("");
    if file_path.is_empty() {
        return Err("缺少 file_path 参数".to_string());
    }
    let group = args.get("group").and_then(|v| v.as_str());
    match kb.ingest_file_as(std::path::Path::new(file_path), group, "agent") {
        Ok(obj) => Ok(format!(
            "已入库: {} (id: {}, 格式: {})",
            obj.title, obj.id, obj.format
        )),
        Err(e) => Err(format!("入库失败: {e}")),
    }
}

fn tool_ingest_url(args: &Value) -> ToolOut {
    let url = args["url"].as_str().unwrap_or("");
    Err(format!("ingest_url 暂未实现，收到 URL: {url}"))
}

fn tool_create_note(kb: &KnowledgeBase, args: &Value) -> ToolOut {
    let title = args["title"].as_str().unwrap_or("");
    let obj_type = args["obj_type"].as_str().unwrap_or("document");
    let group = args["group"].as_str().unwrap_or("收件箱");
    match kb.create_note_as(title, obj_type, group, "agent") {
        Ok(obj) => Ok(format!("已创建: {} (id: {})", obj.title, obj.id)),
        Err(e) => Err(format!("创建失败: {e}")),
    }
}

fn tool_search(kb: &KnowledgeBase, args: &Value) -> ToolOut {
    let query = args["query"].as_str().unwrap_or("");
    let limit = args.get("limit").and_then(|v| v.as_i64()).unwrap_or(20);
    match kb.search(query, limit) {
        Ok(results) => {
            if results.is_empty() {
                Ok("未找到匹配结果".to_string())
            } else {
                Ok(results
                    .iter()
                    .map(|r| {
                        format!(
                            "- [{}] {} (id: {}, score: {})\n  {}",
                            r.r#type, r.title, r.id, r.score, r.snippet
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n"))
            }
        }
        Err(e) => Err(format!("搜索失败: {e}")),
    }
}

fn tool_get_object(kb: &KnowledgeBase, args: &Value) -> ToolOut {
    let id = args["id"].as_str().unwrap_or("");
    match kb.get_object_detail(id) {
        Ok((obj, tags, relations)) => {
            let content = kb.read_object_content(id).unwrap_or_default();
            let relations_text = if relations.is_empty() {
                "（暂无关联关系）".to_string()
            } else {
                relations
                    .iter()
                    .map(|r| {
                        format!(
                            "  - --{}--> {} (id: {})",
                            r.relation_type, r.target_title, r.target_id
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            };
            Ok(format!(
                "ID: {}\n标题: {}\n类型: {}\n格式: {}\n分组: {}\n标签: {}\n状态: {}\n创建者: {}\n\n---\n\n{}\n\n--- 关联关系 ---\n{}",
                obj.id,
                obj.title,
                obj.r#type,
                obj.format,
                obj.group_name,
                tags.join(", "),
                obj.status,
                obj.created_by,
                content,
                relations_text
            ))
        }
        Err(e) => Err(format!("获取失败: {e}")),
    }
}

fn tool_update_content(kb: &KnowledgeBase, args: &Value, scope: &TaskScope) -> ToolOut {
    let id = args["id"].as_str().unwrap_or("");
    let title = args["title"].as_str().unwrap_or("");
    let content = args["content"].as_str().unwrap_or("");
    scope.ensure_object_in_scope(kb, id)?;
    match kb.save_object_content(id, title, content) {
        Ok(()) => Ok(format!("已更新: {title}")),
        Err(e) => Err(format!("更新失败: {e}")),
    }
}

fn tool_delete_object(kb: &KnowledgeBase, args: &Value, scope: &TaskScope) -> ToolOut {
    let id = args["id"].as_str().unwrap_or("");
    scope.ensure_object_in_scope(kb, id)?;
    match kb.delete_object(id) {
        Ok(()) => Ok(format!("已删除: {id}")),
        Err(e) => Err(format!("删除失败: {e}")),
    }
}

fn tool_list_objects(kb: &KnowledgeBase, args: &Value) -> ToolOut {
    let group = args.get("group").and_then(|v| v.as_str());
    match kb.get_file_tree() {
        Ok(groups) => {
            let filtered: Vec<_> = if let Some(g) = group {
                groups.into_iter().filter(|grp| grp.name == g).collect()
            } else {
                groups
            };
            if filtered.is_empty() {
                Ok("无对象".to_string())
            } else {
                Ok(filtered
                    .iter()
                    .map(|g| {
                        let items: Vec<String> = g
                            .objects
                            .iter()
                            .map(|o| format!("  - [{}] {} (id: {})", o.r#type, o.title, o.id))
                            .collect();
                        format!("📁 {} ({}):\n{}", g.name, g.objects.len(), items.join("\n"))
                    })
                    .collect::<Vec<_>>()
                    .join("\n\n"))
            }
        }
        Err(e) => Err(format!("列表失败: {e}")),
    }
}

fn tool_get_stats(kb: &KnowledgeBase) -> ToolOut {
    match kb.get_stats() {
        Ok(stats) => {
            let by_group: Vec<String> = stats
                .by_group
                .iter()
                .map(|(k, v)| format!("  {}: {}", k, v))
                .collect();
            Ok(format!(
                "对象总数: {}\n\n按分组:\n{}",
                stats.total_objects,
                by_group.join("\n")
            ))
        }
        Err(e) => Err(format!("统计失败: {e}")),
    }
}

fn tool_link_objects(kb: &KnowledgeBase, args: &Value, scope: &TaskScope) -> ToolOut {
    let from_id = args["from_id"].as_str().unwrap_or("");
    let to_id = args["to_id"].as_str().unwrap_or("");
    let relation_type = args["relation_type"].as_str().unwrap_or("related_to");
    scope.ensure_object_in_scope(kb, from_id)?;
    scope.ensure_object_in_scope(kb, to_id)?;
    let conn = kb.conn().map_err(|e| format!("数据库连接失败: {e}"))?;
    match db::link_objects(&conn, from_id, to_id, relation_type) {
        Ok(()) => Ok(format!(
            "已建立关系: {} --{}--> {}",
            from_id, relation_type, to_id
        )),
        Err(e) => Err(format!("建立关系失败: {e}")),
    }
}

fn tool_get_relations(kb: &KnowledgeBase, args: &Value) -> ToolOut {
    let id = args["id"].as_str().unwrap_or("");
    match kb.get_object_detail(id) {
        Ok((obj, _tags, relations)) => {
            if relations.is_empty() {
                Ok(format!("对象「{}」当前没有关联关系。", obj.title))
            } else {
                let lines: Vec<String> = relations
                    .iter()
                    .map(|r| {
                        format!(
                            "- --{}--> 「{}」(id: {})",
                            r.relation_type, r.target_title, r.target_id
                        )
                    })
                    .collect();
                Ok(format!(
                    "对象「{}」的关联关系（共 {} 条）：\n{}",
                    obj.title,
                    relations.len(),
                    lines.join("\n")
                ))
            }
        }
        Err(e) => Err(format!("查询失败: {e}")),
    }
}

fn tool_rebuild_relations(kb: &KnowledgeBase) -> ToolOut {
    match kb.rebuild_all_relations() {
        Ok(n) => Ok(format!("已重建所有关系，共建立 {} 条自动关系。", n)),
        Err(e) => Err(format!("重建失败: {e}")),
    }
}

fn tool_get_incoming_relations(kb: &KnowledgeBase, args: &Value) -> ToolOut {
    let id = args["id"].as_str().unwrap_or("");
    let conn = kb.conn().map_err(|e| format!("数据库连接失败: {e}"))?;
    let mut stmt = conn
        .prepare(
            "SELECT r.source_id, s.title, r.relation_type
             FROM relations r
             LEFT JOIN objects s ON s.id = r.source_id
             WHERE r.target_id = ?1
             ORDER BY r.created_at DESC",
        )
        .map_err(|e| format!("查询失败: {e}"))?;
    let rows = stmt.query_map(params![id], |row| {
        Ok(format!(
            "- 「{}」(id: {}) --{}--> 当前对象",
            row.get::<_, String>(1).unwrap_or_default(),
            row.get::<_, String>(0)?,
            row.get::<_, String>(2)?
        ))
    });
    match rows {
        Ok(rows) => {
            let lines: Vec<String> = rows.filter_map(|r| r.ok()).collect();
            if lines.is_empty() {
                Ok("没有对象引用当前对象（无入链/backlinks）。".to_string())
            } else {
                Ok(format!(
                    "引用当前对象的笔记（共 {} 条入链/backlinks）：\n{}",
                    lines.len(),
                    lines.join("\n")
                ))
            }
        }
        Err(e) => Err(format!("查询失败: {e}")),
    }
}

fn tool_list_groups(kb: &KnowledgeBase) -> ToolOut {
    match kb.get_file_tree() {
        Ok(groups) => {
            if groups.is_empty() {
                Ok("知识库为空，暂无分组。".to_string())
            } else {
                let lines: Vec<String> = groups
                    .iter()
                    .map(|g| format!("- 📁 {} ({} 个对象)", g.name, g.objects.len()))
                    .collect();
                Ok(format!(
                    "知识库共 {} 个分组：\n{}\n\n提示：用 list_objects --group <分组名> 查看具体分组下的对象。",
                    groups.len(),
                    lines.join("\n")
                ))
            }
        }
        Err(e) => Err(format!("查询失败: {e}")),
    }
}

fn tool_list_tags(kb: &KnowledgeBase) -> ToolOut {
    let conn = kb.conn().map_err(|e| format!("数据库连接失败: {e}"))?;
    let mut stmt = conn
        .prepare("SELECT tag, COUNT(*) as cnt FROM tags GROUP BY tag ORDER BY cnt DESC")
        .map_err(|e| format!("查询失败: {e}"))?;
    let rows = stmt
        .query_map([], |row| {
            Ok(format!(
                "- #{} ({} 个对象)",
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?
            ))
        })
        .map_err(|e| format!("查询失败: {e}"))?;
    let lines: Vec<String> = rows.filter_map(|r| r.ok()).collect();
    if lines.is_empty() {
        Ok("知识库中暂无标签。可以在对象元数据里加 tag 来建立主题索引。".to_string())
    } else {
        Ok(format!(
            "知识库共 {} 个标签：\n{}",
            lines.len(),
            lines.join("\n")
        ))
    }
}

fn tool_get_object_preview(kb: &KnowledgeBase, args: &Value) -> ToolOut {
    let id = args["id"].as_str().unwrap_or("");
    match kb.get_object_detail(id) {
        Ok((obj, tags, relations)) => {
            let content = kb.read_object_content(id).unwrap_or_default();
            let preview: String = content.chars().take(300).collect();
            let truncated = if content.chars().count() > 300 {
                format!("{}...(共 {} 字符)", preview, content.chars().count())
            } else {
                preview
            };
            let outgoing = relations.len();
            let incoming = kb
                .conn()
                .map(|conn| {
                    conn.query_row(
                        "SELECT COUNT(*) FROM relations WHERE target_id = ?1",
                        params![id],
                        |r| r.get::<_, i64>(0),
                    )
                    .unwrap_or(0)
                })
                .unwrap_or(0);
            Ok(format!(
                "📌 对象预览\n标题: {}\nID: {}\n类型: {}\n格式: {}\n分组: {}\n标签: {}\n关系: {} 出 / {} 入\n\n--- 内容预览 ---\n{}",
                obj.title,
                obj.id,
                obj.r#type,
                obj.format,
                obj.group_name,
                if tags.is_empty() { "无".to_string() } else { tags.join(", ") },
                outgoing,
                incoming,
                truncated
            ))
        }
        Err(e) => Err(format!("获取失败: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ingest::KnowledgeBase;

    // 与其它测试一致的内联 tempfile，避免额外依赖
    mod tempfile {
        use std::path::{Path, PathBuf};
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        pub struct TempDir(PathBuf);
        pub fn tempdir() -> std::io::Result<TempDir> {
            let path = std::env::temp_dir().join(format!(
                "kb-agent-tools-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
                    + COUNTER.fetch_add(1, Ordering::Relaxed) as u128
            ));
            std::fs::create_dir_all(&path)?;
            Ok(TempDir(path))
        }
        impl TempDir {
            pub fn path(&self) -> &Path {
                &self.0
            }
        }
        impl Drop for TempDir {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
    }

    fn setup_vault() -> (tempfile::TempDir, KnowledgeBase) {
        let tmp = tempfile::tempdir().expect("tmp");
        let kb = KnowledgeBase::new(&tmp.path().join("vault"));
        kb.init().expect("init");
        (tmp, kb)
    }

    #[test]
    fn global_scope_allows_everything() {
        let (_tmp, kb) = setup_vault();
        assert!(TaskScope::KnowledgeBase
            .ensure_object_in_scope(&kb, "obj_whatever")
            .is_ok());
    }

    #[test]
    fn collection_scope_rejects_out_of_scope_and_unknown_collection() {
        let (_tmp, kb) = setup_vault();
        let in_obj = kb
            .create_note_as("对象A", "concept", "收件箱", "human")
            .expect("A");
        let out_obj = kb
            .create_note_as("对象B", "concept", "收件箱", "human")
            .expect("B");
        let col_id = kb
            .ensure_collection_path("项目/KBOX", "测试目录", "human")
            .expect("collection");
        kb.move_object_to_collection(&in_obj.id, &col_id)
            .expect("move");

        let scope = TaskScope::Collection { id: col_id.clone() };
        assert!(scope.ensure_object_in_scope(&kb, &in_obj.id).is_ok());
        assert!(scope.ensure_object_in_scope(&kb, &out_obj.id).is_err());

        let bogus = TaskScope::Collection {
            id: "does-not-exist".to_string(),
        };
        assert!(bogus.ensure_object_in_scope(&kb, &in_obj.id).is_err());
    }

    #[test]
    fn delete_object_out_of_scope_is_rejected() {
        let (_tmp, kb) = setup_vault();
        let in_obj = kb
            .create_note_as("对象A", "concept", "收件箱", "human")
            .expect("A");
        let out_obj = kb
            .create_note_as("对象B", "concept", "收件箱", "human")
            .expect("B");
        let col_id = kb
            .ensure_collection_path("项目/KBOX", "测试目录", "human")
            .expect("collection");
        kb.move_object_to_collection(&in_obj.id, &col_id)
            .expect("move");

        let scope = TaskScope::Collection { id: col_id };
        let ok = execute_tool(
            &kb,
            &ToolCall {
                name: "delete_object".into(),
                arguments: json!({ "id": in_obj.id }),
            },
            &scope,
        );
        assert!(!ok.is_error, "scope 内对象应可删除: {}", ok.content);

        let denied = execute_tool(
            &kb,
            &ToolCall {
                name: "delete_object".into(),
                arguments: json!({ "id": out_obj.id }),
            },
            &scope,
        );
        assert!(denied.is_error, "越界对象应被拒绝");
        assert!(
            denied.content.contains("不在当前整理作用域内"),
            "{}",
            denied.content
        );
    }

    #[test]
    fn unknown_tool_is_error() {
        let (_tmp, kb) = setup_vault();
        let result = execute_tool(
            &kb,
            &ToolCall {
                name: "no_such_tool".into(),
                arguments: json!({}),
            },
            &TaskScope::KnowledgeBase,
        );
        assert!(result.is_error);
        assert!(result.content.contains("未知工具"));
    }

    #[test]
    fn get_stats_works_on_empty_vault() {
        let (_tmp, kb) = setup_vault();
        let result = execute_tool(
            &kb,
            &ToolCall {
                name: "get_stats".into(),
                arguments: json!({}),
            },
            &TaskScope::KnowledgeBase,
        );
        assert!(!result.is_error, "{}", result.content);
        assert!(result.content.contains("对象总数"));
    }
}
