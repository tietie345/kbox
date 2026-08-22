use kb_core::KnowledgeBase;
use std::io::Write;

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
