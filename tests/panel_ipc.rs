use base64::Engine;
use desktop_pet::{
    content::Payload,
    panel_ipc::{Message, Request},
    skin::Skin,
};

#[test]
fn pipe_protocol_preserves_unicode_and_compact_binary_payloads() {
    let payloads = vec![
        Payload::text("曦玥\n團團𠮷"),
        Payload::Image(vec![0, 1, 2, 254, 255]),
    ];
    let request = Request {
        id: 42,
        command: "import".into(),
        args: serde_json::json!({"payloads":payloads}),
    };
    let encoded = serde_json::to_string(&request).unwrap();
    assert!(encoded.contains("AAEC/v8="));
    assert!(!encoded.contains("[0,1,2,254,255]"));
    let decoded: Request = serde_json::from_str(&encoded).unwrap();
    let decoded: Vec<Payload> = serde_json::from_value(decoded.args["payloads"].clone()).unwrap();
    assert!(matches!(&decoded[0],Payload::Text{plain,..}if plain=="曦玥\n團團𠮷"));
    assert!(matches!(&decoded[1],Payload::Image(bytes)if bytes==&[0,1,2,254,255]));
    let error = Message::error(42, "原文件已不存在");
    let decoded: Message = serde_json::from_str(&serde_json::to_string(&error).unwrap()).unwrap();
    assert_eq!(decoded.error.as_deref(), Some("原文件已不存在"));
    assert_eq!(decoded.id, Some(42));
}

#[test]
fn static_panel_thumbnail_uses_validated_skin_and_embeds_png_assets() {
    for skin in [
        Skin::builtin(),
        desktop_pet::skin::load(std::path::Path::new("skins/png")).unwrap(),
    ] {
        let uri = skin.preview_svg().unwrap();
        let data = base64::engine::general_purpose::STANDARD
            .decode(uri.strip_prefix("data:image/svg+xml;base64,").unwrap())
            .unwrap();
        let svg = String::from_utf8(data).unwrap();
        assert!(svg.starts_with("<svg xmlns="));
        assert!(svg.ends_with("</svg>"));
        assert!(!svg.contains("<script"));
        assert!(svg.contains("<ellipse") || svg.contains("data:image/png;base64,"));
    }
}
