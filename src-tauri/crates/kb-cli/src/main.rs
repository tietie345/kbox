// CLI 入口：知识库命令行工具

use std::path::PathBuf;

use clap::{Parser, Subcommand};
use kb_core::KnowledgeBase;

fn get_vault_path() -> PathBuf {
    if let Some(path) = std::env::var_os("KB_VAULT_PATH") {
        if !path.is_empty() {
            return PathBuf::from(path);
        }
    }
    if cfg!(debug_assertions) {
        let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        // kb-cli 在 src-tauri/crates/kb-cli/，需要回退 3 层到项目根
        let project_root = manifest
            .parent()
            .and_then(|p| p.parent())
            .and_then(|p| p.parent())
            .unwrap_or_else(|| std::path::Path::new("."));
        project_root.join("dev-vault")
    } else {
        dirs::home_dir().unwrap_or_default().join("KnowledgeBase")
    }
}

fn get_kb() -> KnowledgeBase {
    let vault_path = get_vault_path();
    KnowledgeBase::new(&vault_path)
}

#[derive(Parser)]
#[command(name = "kb", about = "知识库 CLI 工具")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// 初始化知识库
    Init,
    /// 显示状态
    Status,
    /// 入库文件
    Ingest {
        /// 文件路径
        #[arg(short, long)]
        file: String,
        /// 分组
        #[arg(short, long)]
        group: Option<String>,
        /// 手动指定类型（md/pdf/code/image/attachment），不指定则按扩展名自动识别
        #[arg(short, long)]
        r#type: Option<String>,
    },
    /// 创建笔记
    Create {
        #[arg(short, long)]
        title: String,
        #[arg(short, long, default_value = "document")]
        obj_type: String,
        #[arg(short, long, default_value = "收件箱")]
        group: String,
    },
    /// 搜索
    Search {
        query: String,
        #[arg(short, long, default_value_t = 20)]
        limit: i64,
    },
    /// 列出对象
    List {
        #[arg(short, long)]
        group: Option<String>,
    },
    /// 查看对象
    Get { id: String },
    /// 删除对象
    Delete { id: String },
    /// 重建索引
    Rebuild,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let kb = get_kb();

    match cli.command {
        Commands::Init => {
            kb.init()?;
            println!("✓ 知识库已初始化: {}", kb.vault_path.display());
        }
        Commands::Status => {
            let stats = kb.get_stats()?;
            println!("知识库路径: {}", kb.vault_path.display());
            println!("对象总数: {}", stats.total_objects);
            if !stats.by_group.is_empty() {
                println!("\n按分组:");
                for (group, count) in &stats.by_group {
                    println!("  {}: {}", group, count);
                }
            }
        }
        Commands::Ingest {
            file,
            group,
            r#type,
        } => {
            let path = std::path::Path::new(&file);
            let result = match r#type.as_deref() {
                Some("md") => kb.ingest_markdown(path, group.as_deref()),
                // 其他类型统一走 ingest_file（按扩展名识别），手动指定 type 时也可通过扩展名匹配
                _ => kb.ingest_file(path, group.as_deref()),
            };
            match result {
                Ok(obj) => println!(
                    "✓ 已入库: {} (id: {}, 格式: {})",
                    obj.title, obj.id, obj.format
                ),
                Err(e) => {
                    println!("✗ 入库失败: {}", e);
                    std::process::exit(1);
                }
            }
        }
        Commands::Create {
            title,
            obj_type,
            group,
        } => {
            let obj = kb.create_note(&title, &obj_type, &group)?;
            println!("✓ 已创建: {} (id: {})", obj.title, obj.id);
        }
        Commands::Search { query, limit } => {
            let results = kb.search(&query, limit)?;
            if results.is_empty() {
                println!("未找到匹配结果");
            } else {
                for r in results {
                    println!(
                        "- [{}] {} (id: {}, score: {})",
                        r.r#type, r.title, r.id, r.score
                    );
                    if !r.snippet.is_empty() {
                        println!("  {}", r.snippet);
                    }
                }
            }
        }
        Commands::List { group } => {
            let groups = kb.get_file_tree()?;
            let filtered: Vec<_> = if let Some(g) = group {
                groups.into_iter().filter(|grp| grp.name == g).collect()
            } else {
                groups
            };
            if filtered.is_empty() {
                println!("无对象");
            } else {
                for g in filtered {
                    println!("📁 {} ({}):", g.name, g.objects.len());
                    for o in g.objects {
                        println!("  - [{}] {} (id: {})", o.r#type, o.title, o.id);
                    }
                }
            }
        }
        Commands::Get { id } => {
            let (obj, tags, _relations) = kb.get_object_detail(&id)?;
            let content = kb.read_object_content(&id).unwrap_or_default();
            println!("ID: {}", obj.id);
            println!("标题: {}", obj.title);
            println!("类型: {}", obj.r#type);
            println!("格式: {}", obj.format);
            println!("分组: {}", obj.group_name);
            println!("标签: {}", tags.join(", "));
            println!("状态: {}", obj.status);
            println!("创建者: {}", obj.created_by);
            println!("\n---\n\n{}", content);
        }
        Commands::Delete { id } => {
            kb.delete_object(&id)?;
            println!("✓ 已删除: {}", id);
        }
        Commands::Rebuild => {
            let count = kb.rebuild()?;
            println!("✓ 已从文件系统重建 {} 个对象", count);
        }
    }

    Ok(())
}
