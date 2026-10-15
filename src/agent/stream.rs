//! Reconstruct Pi 1.2's delta-only wire messages for panel reconnect snapshots.
use serde_json::{Value, json};

pub fn update(partial: &mut Value, record: &Value) {
    if let Some(message) = record.get("message") {
        *partial = message.clone();
        return;
    }
    let event = &record["assistantMessageEvent"];
    if let Some(message) = event.get("message").or_else(|| event.get("error")) {
        if message.is_object() {
            *partial = message.clone();
        }
        return;
    }
    if partial.is_null() {
        return;
    }
    if let Some(usage) = record.get("usage") {
        partial["usage"] = usage.clone();
    }
    let Some(index) = event["contentIndex"]
        .as_u64()
        .filter(|i| *i < 10000)
        .map(|i| i as usize)
    else {
        return;
    };
    let Some(content) = partial["content"].as_array_mut() else {
        return;
    };
    while content.len() <= index {
        content.push(Value::Null);
    }
    let block = &mut content[index];
    let kind = event["type"].as_str().unwrap_or("");
    match kind {
        "text_start" => *block = json!({"type":"text","text":""}),
        "thinking_start" => *block = json!({"type":"thinking","thinking":""}),
        "toolcall_start" => {
            *block = json!({"type":"toolCall","id":event["id"],"name":event["toolName"],"arguments":{},"_mochiArguments":""})
        }
        "text_delta" | "thinking_delta" | "toolcall_delta" => {
            let field = match kind {
                "text_delta" => "text",
                "thinking_delta" => "thinking",
                _ => "_mochiArguments",
            };
            let mut text = block[field].as_str().unwrap_or("").to_owned();
            text.push_str(event["delta"].as_str().unwrap_or(""));
            if kind == "toolcall_delta"
                && let Ok(arguments) = serde_json::from_str::<Value>(&text)
            {
                block["arguments"] = arguments;
            }
            if block.is_object() {
                block[field] = json!(text);
            }
        }
        "text_end" => block["text"] = event["content"].clone(),
        "thinking_end" => block["thinking"] = event["content"].clone(),
        "toolcall_end" => *block = event["toolCall"].clone(),
        _ => {}
    }
}
