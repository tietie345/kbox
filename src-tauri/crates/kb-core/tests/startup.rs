use kb_core::db;
use kb_core::KnowledgeBase;
use rusqlite::Connection;
use std::time::Instant;

fn temp_root() -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!(
        "kb-startup-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&path).unwrap();
    path
}

#[test]
fn current_schema_startup_does_not_write() {
    let conn = Connection::open_in_memory().unwrap();
    db::init_schema(&conn).unwrap();
    // SQLite rejects any accidental migration/trigger rebuild on the warm path.
    conn.execute_batch("PRAGMA query_only = ON").unwrap();
    db::init_schema(&conn).unwrap();
}

#[test]
fn legacy_upgrade_preserves_search_content_and_restores_missing_index_rows() {
    let conn = Connection::open_in_memory().unwrap();
    db::init_schema(&conn).unwrap();
    conn.execute_batch(
        "INSERT INTO objects(id, type, format, title, group_name, created_by, created_at, updated_at)
         VALUES ('legacy', 'concept', 'markdown', '历史笔记', '旧目录', 'human', 0, 0);
         UPDATE objects_fts SET content = '中文技术文档', tags = '测试' WHERE id = 'legacy';
         DELETE FROM objects_fts_cjk WHERE id = 'legacy';
         DELETE FROM schema_version;
         INSERT INTO schema_version VALUES (4, 0);",
    ).unwrap();
    db::init_schema(&conn).unwrap();
    db::init_schema(&conn).unwrap();
    conn.execute(
        "UPDATE objects SET title = '新标题' WHERE id = 'legacy'",
        [],
    )
    .unwrap();
    let (content, tags, title): (String, String, String) = conn
        .query_row(
            "SELECT content, tags, title FROM objects_fts_cjk WHERE id = 'legacy'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(
        (content.as_str(), tags.as_str(), title.as_str()),
        ("中文技术文档", "测试", "新标题")
    );
    let matches: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM objects_fts_cjk WHERE objects_fts_cjk MATCH '技术文档'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(matches, 1);
    let memberships: i64 = conn.query_row(
        "SELECT COUNT(*) FROM object_collections oc JOIN collections c ON c.id = oc.collection_id
         WHERE oc.object_id = 'legacy' AND c.name = '旧目录'", [], |row| row.get(0),
    ).unwrap();
    assert_eq!(memberships, 1);
}

#[test]
fn failed_migration_rolls_back_and_can_retry() {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("CREATE TABLE objects(id TEXT PRIMARY KEY)")
        .unwrap();
    assert!(db::init_schema(&conn).is_err());
    let version_tables: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE name = 'schema_version'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(version_tables, 0);
    conn.execute_batch("DROP TABLE objects").unwrap();
    db::init_schema(&conn).unwrap();
}

#[test]
fn corrupt_database_is_quarantined_and_rebuilt_from_object_files() {
    let tmp = temp_root();
    let vault = tmp.join("vault");
    let kb = KnowledgeBase::new(&vault);
    kb.init_storage().expect("initial storage");
    let object = kb
        .create_note("损坏恢复", "document", "恢复")
        .expect("create");
    kb.save_object_content(&object.id, &object.title, "可恢复正文")
        .expect("content");
    kb.ensure_collection_path("Parent/Empty", "空目录", "human")
        .expect("collection");
    std::fs::write(vault.join(".kb/index.db"), b"not sqlite").expect("corrupt db");

    let recovered = KnowledgeBase::new(&vault);
    recovered.init_storage().expect("recover storage");
    let detail = recovered
        .get_object_detail(&object.id)
        .expect("recovered object");
    assert_eq!(detail.0.title, "损坏恢复");
    assert_eq!(
        recovered.read_object_content(&object.id).expect("content"),
        "可恢复正文"
    );
    let tree = recovered.get_collection_tree().expect("collections");
    let parent = tree
        .iter()
        .find(|node| node.name == "Parent")
        .expect("parent");
    assert_eq!(parent.children[0].name, "Empty");
    assert!(std::fs::read_dir(vault.join(".kb"))
        .unwrap()
        .flatten()
        .any(|entry| entry
            .file_name()
            .to_string_lossy()
            .starts_with("index.corrupt-")));
    let _ = std::fs::remove_dir_all(tmp);
}

#[test]
#[ignore = "manual startup benchmark"]
fn benchmark_existing_schema_startup() {
    let conn = Connection::open_in_memory().unwrap();
    db::init_schema(&conn).unwrap();
    conn.execute_batch(
        "BEGIN;
         WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM n WHERE x < 5000)
         INSERT INTO objects(id, type, format, title, created_by, created_at, updated_at)
         SELECT 'obj_' || x, 'concept', 'markdown', '技术文档 ' || x, 'human', 0, 0 FROM n;
         COMMIT;",
    )
    .unwrap();
    let start = Instant::now();
    for _ in 0..3 {
        db::init_schema(&conn).unwrap();
    }
    println!(
        "existing schema, 5000 objects, mean of 3: {:?}",
        start.elapsed() / 3
    );
}
