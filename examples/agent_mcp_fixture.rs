//! Native, dependency-free MCP fixture for the no-Node Windows packaging checks.
use serde_json::{Value, json};
use std::io::{BufRead, Write};

fn main() {
    if let Some(path) = std::env::var_os("MOCHI_MCP_PID_FILE") {
        use std::fs::OpenOptions;
        if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
            let _ = writeln!(file, "{}", std::process::id());
        }
    }
    if std::env::args().any(|arg| arg == "--descendant") {
        std::thread::sleep(std::time::Duration::from_secs(600));
        return;
    }
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command.arg("--descendant");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    // Intentionally outlive the stdio server unless Mochi's process tree is reclaimed.
    let mut descendant = command.spawn().unwrap();
    for line in std::io::stdin().lock().lines().map_while(Result::ok) {
        let Ok(request) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let Some(id) = request.get("id") else {
            continue;
        };
        let result = match request["method"].as_str().unwrap_or("") {
            "initialize" => {
                json!({"protocolVersion":"2025-03-26","capabilities":{"tools":{}},"serverInfo":{"name":"mochi-fixture","version":"1.0.0"}})
            }
            "tools/list" => {
                json!({"tools":[{"name":"echo","description":"Read the Mochi MCP fixture","inputSchema":{"type":"object","properties":{},"additionalProperties":false}}]})
            }
            "tools/call" => json!({"content":[{"type":"text","text":"Mochi MCP fixture 中文"}]}),
            _ => json!({}),
        };
        println!("{}", json!({"jsonrpc":"2.0","id":id,"result":result}));
        let _ = std::io::stdout().flush();
    }
    let _ = descendant.kill();
    let _ = descendant.wait();
}
