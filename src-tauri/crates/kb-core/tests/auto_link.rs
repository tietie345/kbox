use kb_core::KnowledgeBase;

// 内联 tempfile（与 md_loop.rs 相同的方案）
mod tempfile {
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    pub struct TempDir(PathBuf);
    pub fn tempdir() -> std::io::Result<TempDir> {
        let mut p = std::env::temp_dir();
        p.push(format!(
            "kb-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
                + COUNTER.fetch_add(1, Ordering::Relaxed) as u128
        ));
        std::fs::create_dir_all(&p)?;
        Ok(TempDir(p))
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

#[test]
fn test_auto_link_wiki_syntax() {
    let tmp = tempfile::tempdir().expect("tmp");
    let vault = tmp.path().join("vault");
    let kb = KnowledgeBase::new(&vault);
    kb.init().expect("init");

    // 创建两个笔记
    let obj1 = kb
        .create_note("MCP 协议", "concept", "技术概念")
        .expect("create obj1");

    let obj2 = kb
        .create_note("RAG 检索", "concept", "技术概念")
        .expect("create obj2");

    // 在 obj2 中引用 obj1 的标题
    let content = "# RAG 检索\n\n提到了 [[MCP 协议]]。\n".to_string();
    // save_object_content 内部会触发 auto_link_relations
    kb.save_object_content(&obj2.id, "RAG 检索", &content)
        .expect("save obj2");

    // 验证数据库中有关系记录
    let conn = kb.conn().expect("conn");
    let count_in_db: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM relations WHERE source_id = ?1 AND target_id = ?2",
            rusqlite::params![&obj2.id, &obj1.id],
            |row| row.get(0),
        )
        .unwrap_or(0);
    println!("数据库中关系记录数: {}", count_in_db);
    assert!(count_in_db > 0, "数据库中应该有关系记录");

    println!("✅ 测试通过：[[标题]] 双向链接关系建立成功");
}

#[test]
fn test_auto_link_title_mention() {
    let tmp = tempfile::tempdir().expect("tmp");
    let vault = tmp.path().join("vault");
    let kb = KnowledgeBase::new(&vault);
    kb.init().expect("init");

    let obj1 = kb
        .create_note("向量数据库", "concept", "技术概念")
        .expect("create obj1");

    let obj2 = kb
        .create_note("RAG 检索", "concept", "技术概念")
        .expect("create obj2");

    // 在 obj2 中直接提到 obj1 的标题（不用 [[]]）
    let content = "# RAG 检索\n\nRAG 检索需要用到向量数据库来存储嵌入。\n";
    // save_object_content 内部会触发 auto_link_relations
    kb.save_object_content(&obj2.id, "RAG 检索", content)
        .expect("save obj2");

    // 验证数据库中是否真的有关系记录（save_object_content 内部已经触发 auto_link）
    let conn = kb.conn().expect("conn");
    let count_in_db: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM relations WHERE source_id = ?1 AND target_id = ?2",
            rusqlite::params![&obj2.id, &obj1.id],
            |row| row.get(0),
        )
        .unwrap_or(0);
    println!("数据库中关系记录数: {}", count_in_db);
    assert!(count_in_db > 0, "save 后应该自动建立标题提及关系");

    println!("✅ 测试通过：标题提及关系建立成功");
}

#[test]
fn test_extract_wiki_links() {
    let content = "这是一个 [[测试链接]] 和 [[另一个链接]] 的内容";
    let links = kb_core::ingest::extract_wiki_links(content);
    println!("提取的链接: {:?}", links);
    assert_eq!(links.len(), 2);
    assert_eq!(links[0], "测试链接");
    assert_eq!(links[1], "另一个链接");

    println!("✅ 测试通过：Wiki 链接提取正确");
}

/// 对标 Obsidian：[[标题|别名]] 应解析为"标题"
#[test]
fn test_auto_link_wiki_alias() {
    let tmp = tempfile::tempdir().expect("tmp");
    let vault = tmp.path().join("vault");
    let kb = KnowledgeBase::new(&vault);
    kb.init().expect("init");

    let obj1 = kb
        .create_note("MCP协议", "concept", "技术概念")
        .expect("create obj1");
    let obj2 = kb
        .create_note("RAG检索", "concept", "技术概念")
        .expect("create obj2");

    // 使用别名语法 [[MCP协议|MCP]]
    let content = "# RAG检索\n\n使用了 [[MCP协议|MCP]]。\n";
    kb.save_object_content(&obj2.id, "RAG检索", content)
        .expect("save");

    let conn = kb.conn().expect("conn");
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM relations WHERE source_id = ?1 AND target_id = ?2",
            rusqlite::params![&obj2.id, &obj1.id],
            |row| row.get(0),
        )
        .unwrap_or(0);
    assert!(count > 0, "别名语法 [[标题|别名]] 应建立关系");

    println!("✅ 测试通过：别名语法解析正确");
}

/// 对标 Obsidian：[[标题#章节]] 应解析为"标题"
#[test]
fn test_auto_link_wiki_heading() {
    let tmp = tempfile::tempdir().expect("tmp");
    let vault = tmp.path().join("vault");
    let kb = KnowledgeBase::new(&vault);
    kb.init().expect("init");

    let obj1 = kb
        .create_note("向量数据库", "concept", "技术概念")
        .expect("create obj1");
    let obj2 = kb
        .create_note("RAG检索", "concept", "技术概念")
        .expect("create obj2");

    let content = "# RAG检索\n\n参考 [[向量数据库#存储引擎]]。\n";
    kb.save_object_content(&obj2.id, "RAG检索", content)
        .expect("save");

    let conn = kb.conn().expect("conn");
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM relations WHERE source_id = ?1 AND target_id = ?2",
            rusqlite::params![&obj2.id, &obj1.id],
            |row| row.get(0),
        )
        .unwrap_or(0);
    assert!(count > 0, "章节语法 [[标题#章节]] 应建立关系");

    println!("✅ 测试通过：章节语法解析正确");
}

/// 对标 Obsidian：大小写不敏感
#[test]
fn test_auto_link_case_insensitive() {
    let tmp = tempfile::tempdir().expect("tmp");
    let vault = tmp.path().join("vault");
    let kb = KnowledgeBase::new(&vault);
    kb.init().expect("init");

    let obj1 = kb
        .create_note("Rust", "concept", "技术概念")
        .expect("create obj1");
    let obj2 = kb
        .create_note("笔记", "concept", "技术概念")
        .expect("create obj2");

    let content = "# 笔记\n\n学习 [[rust]] 编程。\n";
    kb.save_object_content(&obj2.id, "笔记", content)
        .expect("save");

    let conn = kb.conn().expect("conn");
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM relations WHERE source_id = ?1 AND target_id = ?2",
            rusqlite::params![&obj2.id, &obj1.id],
            |row| row.get(0),
        )
        .unwrap_or(0);
    assert!(count > 0, "[[rust]] 应匹配标题 Rust（大小写不敏感）");

    println!("✅ 测试通过：大小写不敏感匹配");
}

/// 幂等性：多次调用 save 不应产生重复关系
#[test]
fn test_auto_link_idempotent() {
    let tmp = tempfile::tempdir().expect("tmp");
    let vault = tmp.path().join("vault");
    let kb = KnowledgeBase::new(&vault);
    kb.init().expect("init");

    // 用不同分组避免 same_group 干扰
    let obj1 = kb
        .create_note("MCP协议", "concept", "技术概念A")
        .expect("create obj1");
    let obj2 = kb
        .create_note("RAG检索", "concept", "技术概念B")
        .expect("create obj2");

    let content = "# RAG检索\n\n参考 [[MCP协议]]。\n";
    kb.save_object_content(&obj2.id, "RAG检索", content)
        .expect("save1");

    // 再次保存同样内容
    kb.save_object_content(&obj2.id, "RAG检索", content)
        .expect("save2");

    let conn = kb.conn().expect("conn");
    // 只看 related_to 类型，应只有 1 条
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM relations WHERE source_id = ?1 AND target_id = ?2 AND relation_type = 'related_to'",
            rusqlite::params![&obj2.id, &obj1.id],
            |row| row.get(0),
        )
        .unwrap_or(0);
    assert_eq!(count, 1, "多次保存不应产生重复 related_to 关系");

    println!("✅ 测试通过：幂等性");
}

/// 短标题（< 3 字符）不应触发纯文本提及匹配，避免误判
#[test]
fn test_auto_link_skip_short_title() {
    let tmp = tempfile::tempdir().expect("tmp");
    let vault = tmp.path().join("vault");
    let kb = KnowledgeBase::new(&vault);
    kb.init().expect("init");

    // 用不同分组避免 same_group 干扰；obj1 标题短
    let obj1 = kb
        .create_note("AI", "concept", "技术概念A")
        .expect("create obj1");
    let obj2 = kb
        .create_note("人工智能笔记", "concept", "技术概念B")
        .expect("create obj2");

    // 纯文本提及「AI」，无 [[]]
    let content = "# 人工智能笔记\n\nAI 是未来。\n";
    kb.save_object_content(&obj2.id, "人工智能笔记", content)
        .expect("save");

    let conn = kb.conn().expect("conn");
    // 「AI」只有 2 字符，不应触发任何关联（related_to / same_group / title_contains 都跳过）
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM relations WHERE source_id = ?1 AND target_id = ?2",
            rusqlite::params![&obj2.id, &obj1.id],
            |row| row.get(0),
        )
        .unwrap_or(0);
    assert_eq!(count, 0, "短标题（< 3 字符）不应触发任何关联");

    println!("✅ 测试通过：短标题跳过");
}

/// 策略 3：同分组自动关联（same_group），排除默认分组
#[test]
fn test_auto_link_same_group() {
    let tmp = tempfile::tempdir().expect("tmp");
    let vault = tmp.path().join("vault");
    let kb = KnowledgeBase::new(&vault);
    kb.init().expect("init");

    // 两个对象在同一有意义的分组
    let obj1 = kb
        .create_note("笔记A", "concept", "技术概念")
        .expect("create obj1");
    let obj2 = kb
        .create_note("笔记B", "concept", "技术概念")
        .expect("create obj2");
    // 第三个在收件箱（应跳过）
    let obj3 = kb
        .create_note("笔记C", "concept", "收件箱")
        .expect("create obj3");

    // 触发 obj1 的 auto_link
    kb.auto_link_relations(&obj1.id).expect("auto_link");

    let conn = kb.conn().expect("conn");
    // obj1 -> obj2 应有 same_group 关系
    let count_ab: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM relations WHERE source_id = ?1 AND target_id = ?2 AND relation_type = 'same_group'",
            rusqlite::params![&obj1.id, &obj2.id],
            |row| row.get(0),
        )
        .unwrap_or(0);
    assert_eq!(count_ab, 1, "同分组对象应建立 same_group 关系");

    // obj1 -> obj3 在收件箱，不应建立 same_group
    let count_ac: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM relations WHERE source_id = ?1 AND target_id = ?2 AND relation_type = 'same_group'",
            rusqlite::params![&obj1.id, &obj3.id],
            |row| row.get(0),
        )
        .unwrap_or(0);
    assert_eq!(count_ac, 0, "收件箱分组不应建立 same_group 关系");

    println!("✅ 测试通过：同分组自动关联");
}

/// 策略 4：共享标签关联（same_tag）
#[test]
fn test_auto_link_same_tag() {
    let tmp = tempfile::tempdir().expect("tmp");
    let vault = tmp.path().join("vault");
    let kb = KnowledgeBase::new(&vault);
    kb.init().expect("init");

    let obj1 = kb
        .create_note("笔记A", "concept", "技术概念")
        .expect("create obj1");
    let obj2 = kb
        .create_note("笔记B", "concept", "其他分组")
        .expect("create obj2");

    // 给两个对象打相同标签
    {
        let conn = kb.conn().expect("conn");
        conn.execute(
            "INSERT INTO tags (object_id, tag) VALUES (?1, ?2)",
            rusqlite::params![&obj1.id, "RAG"],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO tags (object_id, tag) VALUES (?1, ?2)",
            rusqlite::params![&obj1.id, "向量"],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO tags (object_id, tag) VALUES (?1, ?2)",
            rusqlite::params![&obj2.id, "RAG"],
        )
        .unwrap();
    }

    kb.auto_link_relations(&obj1.id).expect("auto_link");

    let conn = kb.conn().expect("conn");
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM relations WHERE source_id = ?1 AND target_id = ?2 AND relation_type = 'same_tag'",
            rusqlite::params![&obj1.id, &obj2.id],
            |row| row.get(0),
        )
        .unwrap_or(0);
    assert_eq!(count, 1, "共享标签应建立 same_tag 关系");

    println!("✅ 测试通过：共享标签关联");
}

/// 策略 5：标题包含关联（title_contains）
#[test]
fn test_auto_link_title_contains() {
    let tmp = tempfile::tempdir().expect("tmp");
    let vault = tmp.path().join("vault");
    let kb = KnowledgeBase::new(&vault);
    kb.init().expect("init");

    // 标题互为子串
    let obj1 = kb
        .create_note("RAG", "concept", "技术概念")
        .expect("create obj1");
    let obj2 = kb
        .create_note("RAG 检索技术", "concept", "技术概念")
        .expect("create obj2");

    kb.auto_link_relations(&obj1.id).expect("auto_link obj1");

    let conn = kb.conn().expect("conn");
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM relations WHERE source_id = ?1 AND target_id = ?2 AND relation_type = 'title_contains'",
            rusqlite::params![&obj1.id, &obj2.id],
            |row| row.get(0),
        )
        .unwrap_or(0);
    assert_eq!(count, 1, "标题包含应建立 title_contains 关系");

    println!("✅ 测试通过：标题包含关联");
}
