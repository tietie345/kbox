use kb_core::KnowledgeBase;
use std::fs;

#[test]
fn test_md_core_loop() {
    let tmp = tempfile::tempdir().expect("tmp");
    let vault = tmp.path().join("vault");
    let kb = KnowledgeBase::new(&vault);

    // 初始化
    kb.init().expect("init");

    // 创建笔记
    let obj = kb
        .create_note("测试笔记", "concept", "技术概念")
        .expect("create_note");
    assert_eq!(obj.title, "测试笔记");
    assert_eq!(obj.group_name, "技术概念");

    // 读取内容
    let content = kb.read_object_content(&obj.id).expect("read");
    assert!(content.contains("# 测试笔记"));

    // 保存修改后的内容
    let new_content = "# 测试笔记\n\n新增内容 RAG 检索测试";
    kb.save_object_content(&obj.id, "测试笔记", new_content)
        .expect("save");

    // 重新读取
    let content2 = kb.read_object_content(&obj.id).expect("read2");
    assert_eq!(content2, new_content);

    // 标题、hash、大小和 index.md 必须同步更新
    kb.save_object_content(&obj.id, "重命名后的笔记", new_content)
        .expect("rename");
    let (renamed, _, _) = kb.get_object_detail(&obj.id).expect("renamed detail");
    assert_eq!(renamed.title, "重命名后的笔记");
    assert!(renamed
        .file_hash
        .as_deref()
        .unwrap_or("")
        .starts_with("sha256:"));
    assert_eq!(renamed.file_size, new_content.len() as i64);
    let index_path = vault.join("objects").join(&obj.id).join("index.md");
    let index = fs::read_to_string(&index_path).expect("read index");
    assert!(index.contains("title: \"重命名后的笔记\""));

    // 更新分组不能清空 FTS 正文，同时必须同步 index.md
    kb.update_object_group(&obj.id, "新分组")
        .expect("move group");
    assert_eq!(kb.search("RAG", 10).expect("search after move").len(), 1);
    let index = fs::read_to_string(&index_path).expect("read moved index");
    assert!(index.contains("group: \"新分组\""));

    // 文件树
    let tree = kb.get_file_tree().expect("file_tree");
    assert_eq!(tree.len(), 1);
    assert_eq!(tree[0].name, "新分组");
    assert_eq!(tree[0].objects.len(), 1);

    // 搜索（FTS5）
    let results = kb.search("RAG", 10).expect("search");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].id, obj.id);

    // 对象详情
    let (obj2, tags, rels) = kb.get_object_detail(&obj.id).expect("detail");
    assert_eq!(obj2.id, obj.id);
    assert!(tags.is_empty());
    assert!(rels.is_empty());

    // 统计
    let stats = kb.get_stats().expect("stats");
    assert_eq!(stats.total_objects, 1);

    // 软删除
    {
        kb.delete_object(&obj.id).expect("delete");
    }
    let stats2 = kb.get_stats().expect("stats2");
    assert_eq!(stats2.total_objects, 0);

    // 文件仍然存在（软删除不删文件）
    let obj_dir = vault.join("objects").join(&obj.id);
    assert!(obj_dir.exists());

    // 删除/恢复状态同步到 index.md，并且有真实操作日志
    let index = fs::read_to_string(&index_path).expect("deleted index");
    assert!(index.contains("status: \"deleted\""));
    kb.restore_object(&obj.id).expect("restore");
    let index = fs::read_to_string(&index_path).expect("restored index");
    assert!(index.contains("status: \"draft\""));
    let conn = kb.conn().expect("conn");
    let op_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM operation_log", [], |row| row.get(0))
        .expect("operation count");
    assert!(op_count >= 6);

    // 模拟索引数据库丢失：重建应从 index.md + 主资产恢复最新数据和 FTS。
    drop(conn);
    for suffix in ["", "-wal", "-shm"] {
        let path = vault.join(".kb").join(format!("index.db{}", suffix));
        if path.exists() {
            fs::remove_file(path).expect("remove database");
        }
    }
    kb.init().expect("reinitialize empty database");
    assert_eq!(kb.rebuild().expect("rebuild"), 1);
    let (rebuilt, _, _) = kb.get_object_detail(&obj.id).expect("rebuilt detail");
    assert_eq!(rebuilt.title, "重命名后的笔记");
    assert_eq!(rebuilt.group_name, "新分组");
    assert_eq!(kb.search("RAG", 10).expect("rebuilt search").len(), 1);

    // 清理
    let _ = fs::remove_dir_all(&vault);
}

// 防止没有 tempfile 时编译失败的最小依赖占位
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
