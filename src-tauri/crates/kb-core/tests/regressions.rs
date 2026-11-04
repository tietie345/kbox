use kb_core::{db, KnowledgeBase};
use std::io::Write;

#[test]
fn startup_maintenance_creates_one_complete_backup_per_day() {
    let tmp = tempfile::tempdir().expect("tmp");
    let kb = KnowledgeBase::new(&tmp.path().join("vault"));
    kb.init_storage().expect("storage ready");
    assert!(!kb.vault_path.join(".kb/backups").exists());
    let note = kb.create_note("备份验证", "concept", "测试").expect("note");
    let backup = kb
        .backup_database()
        .expect("backup")
        .expect("first daily backup");
    let conn = db::open(&backup).expect("open backup");
    assert!(db::get_object(&conn, &note.id).unwrap().is_some());
    let integrity: String = conn
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .unwrap();
    assert_eq!(integrity, "ok");
    assert!(kb.backup_database().expect("second backup").is_none());
    assert!(!backup.with_extension("db.tmp").exists());
}

#[test]
fn search_treats_user_input_as_plain_text() {
    let tmp = tempfile::tempdir().expect("tmp");
    let kb = KnowledgeBase::new(&tmp.path().join("vault"));
    kb.init().expect("init");
    let obj = kb
        .create_note("搜索边界", "concept", "测试")
        .expect("create");
    kb.save_object_content(&obj.id, "搜索边界", "包含 Zebra-20260813 和星号 *")
        .expect("save");

    assert_eq!(kb.search("Zebra-20260813", 10).expect("hyphen").len(), 1);
    assert!(kb.search("*", 10).is_ok(), "星号不应触发 FTS 语法错误");
    assert!(
        kb.search("Alpha OR Beta", 10).is_ok(),
        "OR 应按普通文本处理"
    );
}

#[test]
fn search_falls_back_to_chinese_substring_matching() {
    let tmp = tempfile::tempdir().expect("tmp");
    let kb = KnowledgeBase::new(&tmp.path().join("vault"));
    kb.init().expect("init");
    let obj = kb
        .create_note("中文检索", "concept", "测试")
        .expect("create");
    kb.save_object_content(&obj.id, "中文检索", "这是一个中文技术文档搜索命门")
        .expect("save");

    let results = kb.search("技术文档", 10).expect("search");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].id, obj.id);
    let conn = kb.conn().expect("conn");
    let indexed: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM objects_fts_cjk WHERE objects_fts_cjk MATCH ?1",
            ["\"技术文档\""],
            |row| row.get(0),
        )
        .expect("trigram query");
    assert_eq!(indexed, 1, "中文查询应命中 trigram 索引");
}

#[test]
fn undo_restores_content_tags_and_metadata() {
    let tmp = tempfile::tempdir().expect("tmp");
    let kb = KnowledgeBase::new(&tmp.path().join("vault"));
    kb.init().expect("init");
    let obj = kb
        .create_note("撤销测试", "concept", "测试")
        .expect("create");

    kb.save_object_content(&obj.id, "撤销测试", "第一版正文")
        .expect("first save");
    kb.save_object_content(&obj.id, "撤销测试", "第二版正文")
        .expect("second save");
    kb.undo_last_operation().expect("undo content");
    assert_eq!(
        kb.read_object_content(&obj.id).expect("content"),
        "第一版正文"
    );

    kb.add_object_tags(&obj.id, &["重要".to_string(), "KBOX".to_string()])
        .expect("tags");
    kb.undo_last_operation().expect("undo tags");
    let conn = kb.conn().expect("conn");
    assert!(db::get_tags(&conn, &obj.id).expect("get tags").is_empty());
    drop(conn);

    kb.update_object_metadata(&obj.id, &serde_json::json!({ "owner": "研发" }))
        .expect("metadata");
    kb.undo_last_operation().expect("undo metadata");
    let (detail, _, _) = kb.get_object_detail(&obj.id).expect("detail");
    assert_eq!(detail.metadata, "{}");
}

#[test]
fn content_save_rolls_back_database_and_search_when_audit_write_fails() {
    let tmp = tempfile::tempdir().expect("tmp");
    let kb = KnowledgeBase::new(&tmp.path().join("vault"));
    kb.init().expect("init");
    let object = kb
        .create_note("事务保存", "document", "测试")
        .expect("create");
    kb.save_object_content(&object.id, "旧标题", "旧正文")
        .expect("initial save");
    let conn = kb.conn().expect("conn");
    conn.execute_batch(
        "CREATE TRIGGER reject_audit BEFORE INSERT ON operation_log
         BEGIN SELECT RAISE(ABORT, 'audit blocked'); END;",
    )
    .expect("trigger");
    drop(conn);

    assert!(kb
        .save_object_content(&object.id, "新标题", "新正文")
        .is_err());
    let (stored, _, _) = kb.get_object_detail(&object.id).expect("detail");
    assert_eq!(stored.title, "旧标题");
    assert_eq!(kb.read_object_content(&object.id).expect("file"), "旧正文");
    assert!(kb.search("新正文", 10).expect("search").is_empty());
    assert_eq!(kb.search("旧正文", 10).expect("search").len(), 1);
}

#[test]
fn external_content_changes_sync_and_create_a_restorable_version() {
    let tmp = tempfile::tempdir().expect("tmp");
    let kb = KnowledgeBase::new(&tmp.path().join("vault"));
    kb.init().expect("init");
    let object = kb
        .create_note("外部同步", "document", "测试")
        .expect("create");
    kb.save_object_content(&object.id, &object.title, "库存正文")
        .expect("initial save");
    let (detail, _, _) = kb.get_object_detail(&object.id).expect("detail");
    let path = kb
        .vault_path
        .join("objects")
        .join(&object.id)
        .join(detail.primary_file.expect("primary file"));
    std::fs::write(path, "外部编辑后的正文").expect("external write");

    let report = kb.sync_external_changes().expect("sync");
    assert_eq!(report.changed_object_ids, std::slice::from_ref(&object.id));
    assert!(!report.rebuilt);
    assert_eq!(
        kb.read_object_content(&object.id).expect("content"),
        "外部编辑后的正文"
    );
    assert_eq!(kb.search("外部编辑", 10).expect("search").len(), 1);

    let old = kb
        .get_object_versions(&object.id, 20)
        .expect("versions")
        .into_iter()
        .find(|version| version.content == "库存正文")
        .expect("old version");
    kb.restore_object_version(&object.id, old.operation_id)
        .expect("restore");
    assert_eq!(
        kb.read_object_content(&object.id).expect("restored"),
        "库存正文"
    );
}

#[test]
fn rebuild_does_not_pollute_user_undo_history() {
    let tmp = tempfile::tempdir().expect("tmp");
    let kb = KnowledgeBase::new(&tmp.path().join("vault"));
    kb.init().expect("init");
    let object = kb
        .create_note("重建日志", "document", "测试")
        .expect("create");
    kb.save_object_content(&object.id, &object.title, "正文")
        .expect("save");
    let before = {
        let conn = kb.conn().expect("conn");
        db::max_operation_id(&conn).expect("max before")
    };

    kb.rebuild().expect("rebuild");

    let conn = kb.conn().expect("conn");
    assert_eq!(db::max_operation_id(&conn).expect("max after"), before);
}

#[cfg(unix)]
#[test]
fn image_ocr_updates_searchable_text_with_an_external_engine() {
    use std::os::unix::fs::PermissionsExt;

    let tmp = tempfile::tempdir().expect("tmp");
    let image = tmp.path().join("scan.png");
    std::fs::write(&image, b"not-a-real-image").expect("image");
    let engine = tmp.path().join("fake-tesseract.sh");
    std::fs::write(&engine, "#!/bin/sh\nprintf '车规以太网识别结果\\n'\n").expect("engine");
    let mut permissions = std::fs::metadata(&engine).expect("metadata").permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&engine, permissions).expect("permissions");

    let kb = KnowledgeBase::new(&tmp.path().join("vault"));
    kb.init().expect("init");
    let object = kb.ingest_file(&image, Some("扫描件")).expect("ingest");
    std::env::set_var("KBOX_TESSERACT_BIN", &engine);
    let text = kb.ocr_object(&object.id).expect("ocr");
    std::env::remove_var("KBOX_TESSERACT_BIN");

    assert_eq!(text, "车规以太网识别结果");
    assert_eq!(kb.search("车规以太网", 10).expect("search").len(), 1);
    assert!(kb
        .read_object_content(&object.id)
        .expect("content")
        .contains("## OCR 文本"));
}

#[test]
fn undo_move_restores_group_and_primary_collection() {
    let tmp = tempfile::tempdir().expect("tmp");
    let kb = KnowledgeBase::new(&tmp.path().join("vault"));
    kb.init().expect("init");
    let obj = kb
        .create_note("目录撤销", "concept", "原目录")
        .expect("create");
    let target = kb
        .ensure_collection_path("目标目录/子目录", "", "human")
        .expect("target collection");

    kb.move_object_to_collection(&obj.id, &target)
        .expect("move");
    kb.undo_last_operation().expect("undo move");

    let (restored, _, _) = kb.get_object_detail(&obj.id).expect("detail");
    assert_eq!(restored.group_name, "原目录");
    let conn = kb.conn().expect("conn");
    let collection_id: String = conn
        .query_row(
            "SELECT collection_id FROM object_collections WHERE object_id = ?1 AND is_primary = 1",
            [&obj.id],
            |row| row.get(0),
        )
        .expect("primary collection");
    assert_eq!(
        db::collection_path(&conn, &collection_id).expect("collection path"),
        "原目录"
    );
}

#[test]
fn rename_duplicate_group_and_agent_source_are_safe() {
    let tmp = tempfile::tempdir().expect("tmp");
    let kb = KnowledgeBase::new(&tmp.path().join("vault"));
    kb.init().expect("init");
    let obj = kb
        .create_note_as("原始标题", "concept", "旧分组", "agent")
        .expect("create agent note");
    kb.save_object_content(&obj.id, "原始标题", "不可丢失的正文")
        .expect("save");

    kb.rename_object(&obj.id, "新标题").expect("rename");
    assert_eq!(
        kb.read_object_content(&obj.id).expect("content"),
        "不可丢失的正文"
    );
    let (renamed, _, _) = kb.get_object_detail(&obj.id).expect("detail");
    assert_eq!(renamed.created_by, "agent");
    assert_eq!(renamed.title, "新标题");

    let copy = kb.duplicate_object(&obj.id).expect("duplicate");
    assert_eq!(copy.created_by, "agent");
    assert_eq!(
        kb.read_object_content(&copy.id).expect("copy content"),
        "不可丢失的正文"
    );

    assert_eq!(
        kb.rename_group("旧分组", "新分组").expect("rename group"),
        2
    );
    for object in kb.get_file_tree().expect("tree")[0].objects.iter() {
        assert_eq!(object.created_by, "agent");
    }
}

#[test]
fn hierarchical_collection_rename_and_delete_update_descendants() {
    let tmp = tempfile::tempdir().expect("tmp");
    let kb = KnowledgeBase::new(&tmp.path().join("vault"));
    kb.init().expect("init");
    let object = kb
        .create_note("层级对象", "concept", "收件箱")
        .expect("create");
    let child = kb
        .ensure_collection_path("Parent/Child", "", "human")
        .expect("child");
    kb.move_object_to_collection(&object.id, &child)
        .expect("move");
    assert_eq!(kb.rename_group("Parent", "Renamed").expect("rename"), 1);
    assert_eq!(
        kb.get_object_detail(&object.id)
            .expect("detail")
            .0
            .group_name,
        "Renamed/Child"
    );
    assert_eq!(kb.delete_group("Renamed").expect("delete"), 1);
    assert_eq!(
        kb.get_object_detail(&object.id)
            .expect("detail")
            .0
            .group_name,
        ""
    );
    assert!(kb.get_collection_tree().expect("tree").is_empty());
}

#[test]
fn empty_folder_is_visible_and_persistent() {
    let tmp = tempfile::tempdir().expect("tmp");
    let kb = KnowledgeBase::new(&tmp.path().join("vault"));
    kb.init().expect("init");

    assert!(kb.create_group("资料").expect("create folder"));
    assert!(!kb.create_group("资料").expect("duplicate folder"));
    let tree = kb.get_file_tree().expect("tree");
    assert_eq!(tree.len(), 1);
    assert_eq!(tree[0].name, "资料");
    assert!(tree[0].objects.is_empty());

    kb.delete_group("资料").expect("delete folder");
    assert!(kb.get_file_tree().expect("tree after delete").is_empty());
}

#[test]
fn hierarchical_collections_migrate_and_move_objects() {
    let tmp = tempfile::tempdir().expect("tmp");
    let kb = KnowledgeBase::new(&tmp.path().join("vault"));
    kb.init().expect("init");
    let object = kb
        .create_note("目录测试", "concept", "旧分类")
        .expect("create note");

    let migrated = kb.get_collection_tree().expect("migrated tree");
    assert_eq!(migrated.len(), 1);
    assert_eq!(migrated[0].name, "旧分类");
    assert_eq!(migrated[0].objects[0].id, object.id);

    let collection_id = kb
        .ensure_collection_path("工作/KBOX/技术", "技术实现资料", "agent")
        .expect("ensure path");
    let duplicate_id = kb
        .ensure_collection_path("工作/KBOX/技术", "技术实现资料", "agent")
        .expect("ensure same path");
    assert_eq!(collection_id, duplicate_id);
    kb.move_object_to_collection(&object.id, &collection_id)
        .expect("move object");

    let tree = kb.get_collection_tree().expect("collection tree");
    let work = tree.iter().find(|node| node.name == "工作").expect("work");
    let kbox = work
        .children
        .iter()
        .find(|node| node.name == "KBOX")
        .expect("kbox");
    let tech = kbox
        .children
        .iter()
        .find(|node| node.name == "技术")
        .expect("tech");
    assert_eq!(tech.objects.len(), 1);
    assert_eq!(tech.objects[0].id, object.id);
    fn count_object(nodes: &[kb_core::CollectionNode], object_id: &str) -> usize {
        nodes
            .iter()
            .map(|node| {
                node.objects
                    .iter()
                    .filter(|item| item.id == object_id)
                    .count()
                    + count_object(&node.children, object_id)
            })
            .sum()
    }
    assert_eq!(count_object(&tree, &object.id), 1);
    let (moved, _, _) = kb.get_object_detail(&object.id).expect("detail");
    assert_eq!(moved.group_name, "工作/KBOX/技术");
}

#[test]
fn manual_related_relation_survives_relation_rebuild() {
    let tmp = tempfile::tempdir().expect("tmp");
    let kb = KnowledgeBase::new(&tmp.path().join("vault"));
    kb.init().expect("init");
    let first = kb
        .create_note("手动关系甲", "concept", "测试")
        .expect("first");
    let second = kb
        .create_note("手动关系乙", "concept", "测试")
        .expect("second");
    let conn = kb.conn().expect("conn");
    db::link_objects_manual(&conn, &first.id, &second.id, "related_to").expect("link");
    drop(conn);
    assert_eq!(kb.get_object_detail(&first.id).expect("detail").2.len(), 1);
    kb.rebuild_all_relations().expect("rebuild");
    assert!(kb
        .get_object_detail(&first.id)
        .expect("detail")
        .2
        .iter()
        .any(|relation| relation.target_id == second.id));
}

#[test]
fn manual_relation_is_recorded_and_undoable() {
    let tmp = tempfile::tempdir().expect("tmp");
    let kb = KnowledgeBase::new(&tmp.path().join("vault"));
    kb.init().expect("init");
    let first = kb
        .create_note("可撤销关系甲", "concept", "测试")
        .expect("first");
    let second = kb
        .create_note("可撤销关系乙", "concept", "测试")
        .expect("second");
    let conn = kb.conn().expect("conn");
    db::link_objects_manual(&conn, &first.id, &second.id, "related_to").expect("link");
    drop(conn);
    assert_eq!(kb.get_object_detail(&first.id).expect("detail").2.len(), 1);
    kb.undo_last_operation().expect("undo relation");
    assert!(kb
        .get_object_detail(&first.id)
        .expect("detail")
        .2
        .is_empty());
}

#[test]
fn ai_task_history_records_actions_and_recovers_interrupted_runs() {
    let tmp = tempfile::tempdir().expect("tmp");
    let kb = KnowledgeBase::new(&tmp.path().join("vault"));
    kb.init().expect("init");

    kb.create_ai_task(
        "aitask_completed",
        "knowledge_base",
        "整理项目",
        "openai-compatible",
        "test-model",
    )
    .expect("create completed task");
    kb.add_ai_task_action(
        "aitask_completed",
        "move_object",
        Some("obj_demo"),
        Some("col_demo"),
        "移动测试对象",
    )
    .expect("add action");
    kb.finish_ai_task("aitask_completed", "completed", "已完成")
        .expect("finish task");
    kb.create_ai_task(
        "aitask_running",
        "collection:col_demo",
        "整理目录",
        "openai-compatible",
        "test-model",
    )
    .expect("create running task");

    assert_eq!(kb.recover_running_ai_tasks().expect("recover"), 1);
    let tasks = kb.list_ai_tasks(10).expect("list tasks");
    assert_eq!(tasks.len(), 2);
    assert_eq!(
        tasks
            .iter()
            .find(|task| task.id == "aitask_completed")
            .expect("completed")
            .status,
        "completed"
    );
    let interrupted = tasks
        .iter()
        .find(|task| task.id == "aitask_running")
        .expect("interrupted");
    assert_eq!(interrupted.status, "interrupted");
    assert!(interrupted.finished_at.is_some());

    let actions = kb
        .list_ai_task_actions("aitask_completed")
        .expect("list actions");
    assert_eq!(actions.len(), 1);
    assert_eq!(actions[0].description, "移动测试对象");
}

#[test]
fn docx_import_extracts_text_for_search() {
    let tmp = tempfile::tempdir().expect("tmp");
    let file = tmp.path().join("需求说明.docx");
    write_zip(
        &file,
        &[(
            "word/document.xml",
            r#"<?xml version="1.0" encoding="UTF-8"?>
            <w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
              <w:body>
                <w:p><w:r><w:t>KBOX 应该支持 Word 全文抽取</w:t></w:r></w:p>
              </w:body>
            </w:document>"#,
        )],
    );

    let kb = KnowledgeBase::new(&tmp.path().join("vault"));
    kb.init().expect("init");
    let obj = kb.ingest_file(&file, Some("资料")).expect("ingest docx");

    assert_eq!(obj.title, "需求说明.docx");
    assert_eq!(obj.format, "attachment");
    assert_eq!(kb.search("全文抽取", 10).expect("search").len(), 1);
}

#[test]
fn xlsx_import_extracts_shared_strings_for_search() {
    let tmp = tempfile::tempdir().expect("tmp");
    let file = tmp.path().join("服务矩阵.xlsx");
    write_zip(
        &file,
        &[
            (
                "xl/sharedStrings.xml",
                r#"<?xml version="1.0" encoding="UTF-8"?>
                <sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
                  <si><t>车规以太网</t></si>
                  <si><t>服务矩阵设计</t></si>
                </sst>"#,
            ),
            (
                "xl/workbook.xml",
                r#"<?xml version="1.0" encoding="UTF-8"?>
                <workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"
                  xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
                  <sheets><sheet name="矩阵" sheetId="1" r:id="rId1"/></sheets>
                </workbook>"#,
            ),
            (
                "xl/_rels/workbook.xml.rels",
                r#"<?xml version="1.0" encoding="UTF-8"?>
                <Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
                  <Relationship Id="rId1" Type="worksheet" Target="worksheets/sheet1.xml"/>
                </Relationships>"#,
            ),
            (
                "xl/worksheets/sheet1.xml",
                r#"<?xml version="1.0" encoding="UTF-8"?>
                <worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
                  <sheetData>
                    <row r="1"><c r="A1" t="s"><v>0</v></c><c r="B1" t="s"><v>1</v></c></row>
                  </sheetData>
                </worksheet>"#,
            ),
        ],
    );

    let kb = KnowledgeBase::new(&tmp.path().join("vault"));
    kb.init().expect("init");
    let obj = kb.ingest_file(&file, Some("资料")).expect("ingest xlsx");

    assert_eq!(obj.title, "服务矩阵.xlsx");
    assert_eq!(obj.format, "attachment");
    assert_eq!(kb.search("服务矩阵设计", 10).expect("search").len(), 1);
}

fn write_zip(path: &std::path::Path, files: &[(&str, &str)]) {
    let file = std::fs::File::create(path).expect("zip file");
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::FileOptions::default();
    for (name, content) in files {
        zip.start_file(*name, options).expect("start file");
        zip.write_all(content.as_bytes()).expect("write file");
    }
    zip.finish().expect("finish zip");
}

mod tempfile {
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    pub struct TempDir(PathBuf);
    pub fn tempdir() -> std::io::Result<TempDir> {
        let path = std::env::temp_dir().join(format!(
            "kb-regression-{}-{}",
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
