// MCP Server 入口：基于 stdio 的 JSON-RPC 2.0 over stdin/stdout
// 不依赖任何外部 MCP 库，纯手写 JSON-RPC。
//
// 本文件只负责 JSON-RPC 协议转换：工具定义 / 参数解析 / 执行 / 出错格式化
// 全部委托给共享 Tool Registry（kb-core/src/agent_tools.rs）。

use std::io::{self, BufRead, Write};
use std::path::PathBuf;

use kb_core::agent_tools::{execute_tool, tools_list_json, TaskScope, ToolCall};
use kb_core::KnowledgeBase;
use serde_json::{json, Value};

fn get_vault_path() -> PathBuf {
    if let Some(path) = std::env::var_os("KB_VAULT_PATH") {
        if !path.is_empty() {
            return PathBuf::from(path);
        }
    }
    if cfg!(debug_assertions) {
        let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        // kb-mcp 在 src-tauri/crates/kb-mcp/，需要回退 3 层到项目根
        let project_root = manifest
            .parent() // crates/
            .and_then(|p| p.parent()) // src-tauri/
            .and_then(|p| p.parent()) // 项目根
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

// 工具定义：委托共享 Registry
fn tools_list() -> Value {
    tools_list_json()
}

// 处理工具调用：委托共享 Registry（MCP 视为全局作用域）
fn handle_tool_call(name: &str, args: &Value) -> Value {
    let kb = get_kb();
    let call = ToolCall {
        name: name.to_string(),
        arguments: args.clone(),
    };
    let result = execute_tool(&kb, &call, &TaskScope::KnowledgeBase);
    let content = json!([{ "type": "text", "text": result.content }]);
    if result.is_error {
        json!({ "content": content, "isError": true })
    } else {
        json!({ "content": content })
    }
}

// 处理 JSON-RPC 请求
fn handle_request(method: &str, params: &Value, id: &Value) -> Value {
    match method {
        "initialize" => json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "protocolVersion": "2024-11-05",
                "serverInfo": {
                    "name": "kb-mcp",
                    "version": "0.1.0"
                },
                "capabilities": {
                    "tools": {}
                }
            }
        }),
        "notifications/initialized" => {
            // 通知，正常情况下不会走到这里（main 中已 continue），保险起见返回空结果
            json!({ "jsonrpc": "2.0", "id": id, "result": {} })
        }
        "tools/list" => json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": tools_list()
        }),
        "tools/call" => {
            let name = params["name"].as_str().unwrap_or("");
            let default_args = json!({});
            let arguments = params.get("arguments").unwrap_or(&default_args);
            let result = handle_tool_call(name, arguments);
            json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": result
            })
        }
        _ => json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": {
                "code": -32601,
                "message": format!("Method not found: {}", method)
            }
        }),
    }
}

fn main() {
    // 初始化知识库（失败不阻止 server 启动）
    let vault_path = get_vault_path();
    let kb = KnowledgeBase::new(&vault_path);
    let _ = kb.init();

    let stdin = io::stdin();
    let mut stdout = io::stdout();

    eprintln!("kb-mcp server started (vault: {})", vault_path.display());

    // 逐行读取 JSON-RPC 请求
    for line in stdin.lock().lines() {
        match line {
            Ok(line) => {
                if line.trim().is_empty() {
                    continue;
                }
                match serde_json::from_str::<Value>(&line) {
                    Ok(req) => {
                        let method = req["method"].as_str().unwrap_or("");
                        let default_params = json!({});
                        let default_id = json!(null);
                        let params = req.get("params").unwrap_or(&default_params);
                        let id = req.get("id").unwrap_or(&default_id);

                        // 通知（无 id）不需要响应
                        if id.is_null() && method.starts_with("notifications/") {
                            continue;
                        }

                        let response = handle_request(method, params, id);
                        let response_str = serde_json::to_string(&response).unwrap_or_default();
                        writeln!(stdout, "{}", response_str).unwrap();
                        stdout.flush().unwrap();
                    }
                    Err(e) => {
                        eprintln!("Parse error: {}", e);
                    }
                }
            }
            Err(e) => {
                eprintln!("Read error: {}", e);
                break;
            }
        }
    }
}
