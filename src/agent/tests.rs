use super::*;
use std::io;

#[test]
fn local_skills_install_toggle_and_uninstall_keep_source_untouched() {
    let mut worker = worker();
    let source = worker.paths.root.join("source/test-skill");
    std::fs::create_dir_all(source.join("scripts")).unwrap();
    std::fs::write(source.join("SKILL.md"), "---\nname: test-skill\ndescription: >\n  A multiline description\n  for local skills.\n---\n# Local skill\n").unwrap();
    std::fs::write(source.join("scripts/example.txt"), "asset").unwrap();
    std::fs::create_dir_all(source.join("subskills/child")).unwrap();
    let child = "---\nname: child\ndescription: Bundled subskill\n---\n# Child";
    std::fs::write(source.join("subskills/child/SKILL.md"), child).unwrap();
    worker
        .handle("agent_skills_install", &json!({"path":source}))
        .unwrap();
    assert!(worker.runtime.is_none());
    let installed = worker.handle("agent_skills_list", &json!({})).unwrap();
    assert_eq!(installed[0]["id"], "test-skill");
    assert_eq!(installed[0]["enabled"], true);
    assert_eq!(installed.as_array().unwrap().len(), 1);
    assert_eq!(skills::enabled(&worker.paths).unwrap().len(), 1);
    assert_eq!(
        std::fs::read_to_string(
            worker
                .paths
                .skills()
                .join("test-skill/subskills/child/SKILL.md")
        )
        .unwrap(),
        child
    );
    assert_eq!(
        std::fs::read_to_string(worker.paths.skills().join("test-skill/scripts/example.txt"))
            .unwrap(),
        "asset"
    );
    assert!(
        worker
            .handle("agent_skills_install", &json!({"path":source}))
            .is_err()
    );
    worker
        .handle(
            "agent_skills_toggle",
            &json!({"id":"test-skill","enabled":false}),
        )
        .unwrap();
    assert!(skills::enabled(&worker.paths).unwrap().is_empty());
    assert!(worker.paths.skills().join("test-skill/SKILL.md").exists());
    assert!(
        !skills::list(&Paths {
            root: worker.paths.root.clone()
        })
        .unwrap()[0]
            .enabled
    );
    worker
        .handle(
            "agent_skills_toggle",
            &json!({"id":"test-skill","enabled":true}),
        )
        .unwrap();
    worker.busy = true;
    assert!(
        worker
            .handle("agent_skills_uninstall", &json!({"id":"test-skill"}))
            .is_err()
    );
    worker.busy = false;
    worker
        .handle("agent_skills_uninstall", &json!({"id":"test-skill"}))
        .unwrap();
    assert!(skills::list(&worker.paths).unwrap().is_empty());
    assert!(source.join("SKILL.md").exists());
    assert!(
        worker
            .handle("agent_skills_uninstall", &json!({"id":"../source"}))
            .is_err()
    );
    std::fs::remove_dir_all(&worker.paths.root).unwrap();
}

#[test]
fn zip_skills_validate_paths_and_metadata_before_installing() {
    use std::io::Write;
    let worker = worker();
    worker.paths.ensure().unwrap();
    let archive = worker.paths.root.join("skill.zip");
    let zip = |entries: &[(&str, &str)]| {
        let mut writer = zip::ZipWriter::new(std::fs::File::create(&archive).unwrap());
        for (name, value) in entries {
            writer
                .start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            writer.write_all(value.as_bytes()).unwrap();
        }
        writer.finish().unwrap();
    };
    for prefix in ["", "nested/demo/"] {
        let parent = format!("{prefix}SKILL.md");
        let child = format!("{prefix}subskills/child/SKILL.md");
        let resource = format!("{prefix}subskills/child/references/test.txt");
        let child_text = "---\nname: child\ndescription: Bundled subskill\n---\n# Child";
        zip(&[
            (
                &parent,
                "---\nname: zip-demo\ndescription: Test ZIP\n---\nSee subskills/child/SKILL.md",
            ),
            (&child, child_text),
            (&resource, "resource"),
        ]);
        assert_eq!(
            skills::install(&worker.paths, &archive).unwrap(),
            "zip-demo"
        );
        assert_eq!(skills::list(&worker.paths).unwrap().len(), 1);
        assert_eq!(skills::enabled(&worker.paths).unwrap().len(), 1);
        assert_eq!(
            std::fs::read_to_string(
                worker
                    .paths
                    .skills()
                    .join("zip-demo/subskills/child/SKILL.md")
            )
            .unwrap(),
            child_text
        );
        assert_eq!(
            std::fs::read_to_string(
                worker
                    .paths
                    .skills()
                    .join("zip-demo/subskills/child/references/test.txt")
            )
            .unwrap(),
            "resource"
        );
        skills::uninstall(&worker.paths, "zip-demo").unwrap();
        assert!(skills::list(&worker.paths).unwrap().is_empty());
    }
    zip(&[
        ("demo/SKILL.md", "---\nname: first\ndescription: Test\n---"),
        (
            "demo/subskills/child/SKILL.md",
            "---\nname: child\ndescription: Test\n---",
        ),
        (
            "demo-other/SKILL.md",
            "---\nname: second\ndescription: Test\n---",
        ),
    ]);
    assert!(
        skills::install(&worker.paths, &archive)
            .unwrap_err()
            .contains("多个并列")
    );
    zip(&[("README.md", "No Skill entry point")]);
    assert!(
        skills::install(&worker.paths, &archive)
            .unwrap_err()
            .contains("未找到 SKILL.md")
    );
    zip(&[
        ("../escape.txt", "bad"),
        ("SKILL.md", "---\nname: unsafe\ndescription: test\n---"),
    ]);
    assert!(skills::install(&worker.paths, &archive).is_err());
    zip(&[("SKILL.md", "---\nname: invalid\n---")]);
    assert!(skills::install(&worker.paths, &archive).is_err());
    zip(&[
        ("SKILL.md", "---\nname: duplicate\ndescription: test\n---"),
        ("Readme.txt", "one"),
        ("readme.txt", "two"),
    ]);
    assert!(skills::install(&worker.paths, &archive).is_err());
    assert!(skills::list(&worker.paths).unwrap().is_empty());
    assert!(
        !worker
            .paths
            .root
            .parent()
            .unwrap()
            .join("escape.txt")
            .exists()
    );
    std::fs::remove_dir_all(&worker.paths.root).unwrap();
}

#[test]
fn default_model_configuration_is_lazy_and_skills_changes_are_deferred() {
    let mut worker = worker();
    worker
        .handle(
            "agent_model_default",
            &json!({"provider":"test","modelId":"first"}),
        )
        .unwrap();
    assert!(worker.runtime.is_none());
    assert_eq!(
        worker.paths.read("settings.json").unwrap()["defaultModel"],
        "first"
    );
    assert!(
        worker
            .handle("agent_model_default", &json!({"provider":"","modelId":""}))
            .is_err()
    );
    worker.paths.ensure().unwrap();
    std::fs::create_dir_all(worker.paths.skills().join("demo")).unwrap();
    std::fs::write(
        worker.paths.skills().join("demo/SKILL.md"),
        "---\nname: demo\ndescription: Test\n---",
    )
    .unwrap();
    worker.runtime = Some(fixture_runtime());
    worker.busy = true;
    let pid = worker.runtime.as_ref().unwrap().child.id();
    worker
        .handle("agent_skills_toggle", &json!({"id":"demo","enabled":false}))
        .unwrap();
    assert!(worker.dirty);
    assert_eq!(worker.runtime.as_ref().unwrap().child.id(), pid);
    assert!(skills::enabled(&worker.paths).unwrap().is_empty());
}

fn worker() -> Worker {
    let directory = std::env::temp_dir().join(format!(
        "mochi-agent-tests-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let (sender, _) = mpsc::channel();
    Worker::new(Paths::new(&directory), sender)
}

#[test]
fn stream_end_is_not_settled_and_interactions_survive_panel_close() {
    let mut worker = worker();
    worker.record(json!({"type":"agent_start"}));
    worker.record(json!({"type":"message_update","message":{"role":"assistant","content":[]}}));
    worker.record(json!({"type":"agent_end"}));
    assert!(worker.busy);
    worker.record(
        json!({"type":"extension_ui_request","id":"dialog","method":"input","title":"名字"}),
    );
    worker.panel = false;
    assert_eq!(worker.interactions.len(), 1);
    assert!(!worker.partial.is_null());
    worker.record(json!({"type":"agent_settled"}));
    assert!(!worker.busy);
    assert!(worker.partial.is_null());
    assert_eq!(worker.interactions.len(), 1);
}

#[test]
fn reconstruct_delta_only_text_thinking_and_tool_calls_for_reconnect() {
    let mut worker = worker();
    worker.record(
        json!({"type":"message_start","message":{"role":"assistant","content":[],"timestamp":1}}),
    );
    for event in [
        json!({"type":"text_start","contentIndex":0}),
        json!({"type":"text_delta","contentIndex":0,"delta":"中"}),
        json!({"type":"text_delta","contentIndex":0,"delta":"文"}),
        json!({"type":"text_end","contentIndex":0,"content":"中文完成"}),
        json!({"type":"thinking_start","contentIndex":1}),
        json!({"type":"thinking_delta","contentIndex":1,"delta":"思考"}),
        json!({"type":"toolcall_start","contentIndex":2,"id":"tool","toolName":"read"}),
        json!({"type":"toolcall_delta","contentIndex":2,"delta":"{\"path\":"}),
        json!({"type":"toolcall_delta","contentIndex":2,"delta":"\"file\"}"}),
    ] {
        worker.record(json!({"type":"message_update","assistantMessageEvent":event}));
    }
    assert_eq!(worker.partial["content"][0]["text"], "中文完成");
    assert_eq!(worker.partial["content"][1]["thinking"], "思考");
    assert_eq!(worker.partial["content"][2]["arguments"]["path"], "file");
    worker.record(json!({"type":"message_update","assistantMessageEvent":{"type":"toolcall_end","contentIndex":2,"toolCall":{"type":"toolCall","id":"tool","name":"read","arguments":{"path":"final"}}}}));
    assert_eq!(worker.partial["content"][2]["arguments"]["path"], "final");
    assert!(
        worker.partial["content"][2]
            .get("_mochiArguments")
            .is_none()
    );
    worker.record(json!({"type":"message_end","message":{"role":"assistant","content":[]}}));
    assert!(worker.partial.is_null());
}

#[test]
fn ordinary_status_never_starts_pi_or_exposes_keys() {
    let mut worker = worker();
    worker.handle("agent_storage", &json!({"store":"provider-keys","operation":"set","key":"test","value":"secret-test-token"})).unwrap();
    assert!(worker.runtime.is_none());
    assert_eq!(
        worker
            .handle(
                "agent_storage",
                &json!({"store":"provider-keys","operation":"get","key":"test"})
            )
            .unwrap(),
        "configured"
    );
    assert!(!worker.status().to_string().contains("secret-test-token"));
    assert_eq!(
        worker.redact("failure secret-test-token"),
        "failure [已隐藏]"
    );
    assert_eq!(
        worker.paths.read("auth.json").unwrap()["test"]["key"],
        "secret-test-token"
    );
}

#[test]
fn relative_data_directory_is_resolved_before_pi_changes_working_directory() {
    let paths = Paths::new(Path::new(".artifacts/agent-relative-fixture"));
    assert!(paths.root.is_absolute());
    assert!(paths.skills().is_absolute());
    assert!(paths.config().is_absolute());
}

#[test]
fn key_and_model_changes_are_saved_without_interrupting_an_active_run() {
    let mut worker = worker();
    worker.runtime = Some(fixture_runtime());
    worker.busy = true;
    let pid = worker.runtime.as_ref().unwrap().child.id();
    worker.handle("agent_storage", &json!({"store":"provider-keys","operation":"set","key":"test","value":"deferred-secret"})).unwrap();
    assert!(worker.dirty);
    let result = worker
        .handle(
            "agent_rpc",
            &json!({"type":"set_model","provider":"test","modelId":"next"}),
        )
        .unwrap();
    assert_eq!(result["deferred"], true);
    assert_eq!(worker.pending_model.as_ref().unwrap()["modelId"], "next");
    assert_eq!(worker.runtime.as_ref().unwrap().child.id(), pid);
    assert_eq!(
        worker.paths.read("settings.json").unwrap()["defaultModel"],
        "next"
    );
}

#[test]
fn restrict_rpc_sessions_and_mcp_configuration() {
    let mut worker = worker();
    assert!(
        worker
            .handle("agent_rpc", &json!({"type":"bash","command":"bad"}))
            .is_err()
    );
    worker.busy = true;
    assert!(
        worker
            .handle("agent_rpc", &json!({"type":"new_session"}))
            .is_err()
    );
    assert!(
        worker
            .handle(
                "agent_rpc",
                &json!({"type":"switch_session","sessionPath":"outside.jsonl"})
            )
            .is_err()
    );
    assert!(validate_mcp(&json!({"mcpServers":{"bad name":{"command":"mock"}}})).is_err());
    assert!(
        validate_mcp(&json!({"mcpServers":{"ok":{"type":"sse","url":"https://example.com"}}}))
            .is_err()
    );
    assert!(validate_mcp(&json!({"mcpServers":{"ok":{"command":"mock","args":[1]}}})).is_err());
    assert!(validate_mcp(&json!({"mcpServers":{"ok":{"command":"mock","args":[]}}})).is_ok());
}

#[test]
fn runtime_rejects_missing_wrong_version_and_corrupt_resources() {
    let worker = worker();
    let root = worker.paths.root.join("runtime");
    assert!(process::verify(&root).is_err());
    fs_create(&root);
    let executable = if cfg!(windows) { "pi.exe" } else { "pi" };
    let mut files = json!({});
    for name in [executable, "package.json", "LICENSE"] {
        std::fs::write(root.join(name), "valid").unwrap();
        use sha2::Digest;
        files[name] = json!(format!("{:x}", sha2::Sha256::digest(b"valid")));
    }
    crate::config::atomic_json(
        &root.join("mochi-runtime.json"),
        &json!({"version":"1.2.0","files":files}),
    )
    .unwrap();
    assert!(process::verify(&root).is_ok());
    std::fs::write(root.join("LICENSE"), "changed").unwrap();
    assert!(process::verify(&root).is_err());
    crate::config::atomic_json(
        &root.join("mochi-runtime.json"),
        &json!({"version":"1.2.1","files":files}),
    )
    .unwrap();
    assert!(process::verify(&root).unwrap_err().contains("版本"));
}

fn fs_create(path: &Path) {
    std::fs::create_dir_all(path).unwrap();
}

#[test]
fn jsonl_is_the_session_source_and_outside_paths_are_rejected() {
    let worker = worker();
    worker.paths.ensure().unwrap();
    let session = worker.paths.sessions().join("one.jsonl");
    std::fs::write(&session, "{\"type\":\"session\",\"id\":\"one\"}\n{\"type\":\"message\",\"message\":{\"role\":\"user\",\"content\":[{\"type\":\"text\",\"text\":\"测试会话\"}]}}\n{partial").unwrap();
    assert_eq!(
        worker.paths.sessions_list().unwrap()[0]["title"],
        "测试会话"
    );
    worker.paths.remember(session.to_str().unwrap()).unwrap();
    assert_eq!(
        worker.paths.current(),
        Some(session.canonicalize().unwrap())
    );
    let outside = worker.paths.root.join("outside.jsonl");
    std::fs::write(&outside, "{}").unwrap();
    assert!(
        worker
            .paths
            .session_path(outside.to_str().unwrap())
            .is_err()
    );
}

#[test]
fn deleting_sessions_is_confined_and_current_session_cannot_be_restored() {
    let mut worker = worker();
    worker.paths.ensure().unwrap();
    let current = worker.paths.sessions().join("current.jsonl");
    let archived = worker.paths.sessions().join("archived.jsonl");
    let outside = worker.paths.root.join("outside.jsonl");
    for file in [&current, &archived, &outside] {
        std::fs::write(file, "{\"type\":\"session\",\"id\":\"test\"}\n").unwrap();
    }
    worker.paths.remember(current.to_str().unwrap()).unwrap();
    worker.state = json!({"sessionId":"test", "sessionFile":current});
    let args = json!({"sessionPath":archived});
    worker.busy = true;
    assert!(worker.handle("agent_session_delete", &args).is_err());
    worker.busy = false;
    worker.starting = true;
    assert!(worker.handle("agent_session_delete", &args).is_err());
    worker.starting = false;
    worker.interactions.insert("input".into(), json!({}));
    assert!(worker.handle("agent_session_delete", &args).is_err());
    worker.interactions.clear();
    assert!(archived.exists());
    assert!(
        worker
            .handle("agent_session_delete", &json!({"sessionPath":outside}))
            .is_err()
    );
    assert!(outside.exists());
    worker.handle("agent_session_delete", &args).unwrap();
    assert!(!archived.exists());
    assert!(current.exists());
    assert_eq!(
        worker.paths.current(),
        Some(current.canonicalize().unwrap())
    );
    assert_eq!(
        worker
            .paths
            .sessions_list()
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert!(worker.runtime.is_none());

    worker.runtime = Some(fixture_runtime());
    let generation = worker.generation;
    let snapshot = worker
        .handle("agent_session_delete", &json!({"sessionPath":current}))
        .unwrap();
    assert!(worker.runtime.is_none());
    assert!(!current.exists());
    assert!(!worker.paths.root.join("current.json").exists());
    assert!(worker.paths.current().is_none());
    assert_eq!(snapshot["messages"], json!([]));
    assert_eq!(snapshot["state"], json!({}));
    assert_eq!(worker.generation, generation + 1);
    assert!(
        worker
            .paths
            .sessions_list()
            .unwrap()
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(
        worker
            .handle("agent_session_delete", &json!({"sessionPath":current}))
            .is_err()
    );
    let (sender, _) = mpsc::channel();
    let mut reopened = Worker::new(worker.paths.clone(), sender);
    assert_eq!(
        reopened.handle("agent_restore", &json!({})).unwrap()["messages"],
        json!([])
    );
    assert!(reopened.runtime.is_none());
    std::fs::remove_dir_all(&worker.paths.root).unwrap();
}

fn fixture_runtime() -> Runtime {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", "agent::tests::rpc_fixture", "--nocapture"])
        .env("MOCHI_RPC_FIXTURE", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    process::hide(&mut command);
    let mut child = command.spawn().unwrap();
    let tree = process::Tree::attach(&child).unwrap();
    let input = child.stdin.take();
    let output = child.stdout.take().unwrap();
    let (sender, records) = mpsc::channel();
    thread::spawn(move || {
        for line in BufReader::new(output).lines().map_while(Result::ok) {
            if let Ok(value) = serde_json::from_str(&line)
                && sender.send(value).is_err()
            {
                break;
            }
        }
    });
    Runtime {
        child,
        input,
        records,
        tree: Some(tree),
    }
}

#[test]
fn rpc_fixture() {
    if std::env::var("MOCHI_RPC_FIXTURE").as_deref() != Ok("1") {
        return;
    }
    for line in io::stdin().lock().lines().map_while(Result::ok) {
        let value: Value = serde_json::from_str(&line).unwrap();
        let id = &value["id"];
        let kind = value["type"].as_str().unwrap();
        if kind == "crash" {
            std::process::exit(7);
        }
        if kind == "prompt" && value["message"] != "handled" {
            println!("{}", json!({"type":"agent_start"}));
            println!(
                "{}",
                json!({"type":"message_update","message":{"role":"assistant","content":[{"type":"text","text":"流式"}]}})
            );
            println!(
                "{}",
                json!({"type":"tool_execution_start","toolCallId":"tool"})
            );
            println!(
                "{}",
                json!({"type":"tool_execution_end","toolCallId":"tool","isError":value["message"] == "failure"})
            );
            println!("{}", json!({"type":"agent_end"}));
            println!("{}", json!({"type":"agent_settled"}));
        }
        println!(
            "{}",
            json!({"id":"unrelated","type":"response","success":true,"data":"out-of-order"})
        );
        println!(
            "{}",
            json!({"id":id,"type":"response","command":kind,"success":true,"data":{"disposition":if value["message"] == "handled" {"handled"} else {"started"}}})
        );
        io::stdout().flush().unwrap();
    }
}

#[test]
fn correlate_interleaved_events_handle_immediate_completion_and_never_replay_crash() {
    let mut worker = worker();
    worker.runtime = Some(fixture_runtime());
    worker
        .handle("agent_rpc", &json!({"type":"prompt","message":"hello"}))
        .unwrap();
    assert!(!worker.busy);
    assert!(!worker.starting);
    assert!(worker.responses.contains_key("unrelated"));
    worker
        .handle("agent_rpc", &json!({"type":"prompt","message":"handled"}))
        .unwrap();
    assert!(!worker.starting);
    worker
        .handle("agent_rpc", &json!({"type":"prompt","message":"failure"}))
        .unwrap();
    assert!(!worker.busy);
    worker.rpc(json!({"type":"clear_queue"})).unwrap();
    worker.rpc(json!({"type":"abort"})).unwrap();
    worker.record(json!({"type":"extension_ui_request","id":"waiting-dialog","method":"input"}));
    worker.handle("agent_rpc", &json!({"type":"stop"})).unwrap();
    assert!(worker.interactions.is_empty());
    assert!(worker.rpc(json!({"type":"crash"})).is_err());
    assert!(
        worker
            .runtime
            .as_mut()
            .unwrap()
            .child
            .wait()
            .unwrap()
            .code()
            == Some(7)
    );
}
