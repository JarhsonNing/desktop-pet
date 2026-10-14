//! Isolated demo fixtures for screenshots; never reads or writes the OS clipboard.
use desktop_pet::content::{Payload, Policy, Source, Store};
use std::{fs, path::PathBuf, sync::atomic::AtomicBool};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let directory = PathBuf::from(
        std::env::args()
            .nth(1)
            .ok_or("Pass an isolated demo directory")?,
    );
    fs::create_dir_all(&directory)?;
    if directory.join("content/index.sqlite").exists() {
        return Err("Demo directory already contains a content database".into());
    }
    fs::write(
        directory.join("config.json"),
        r#"{"name":"团团","roaming":false,"clipboard_enabled":false}"#,
    )?;
    let sources = directory.join("demo-files");
    fs::create_dir_all(sources.join("项目资料"))?;
    fs::write(
        sources.join("项目资料/需求说明.txt"),
        "文件已保存为独立副本，原文件移动后仍可取回。",
    )?;
    fs::write(sources.join("演示笔记.txt"), "这是演示文件。")?;
    let mut store = Store::open(&directory, Policy::default())?;
    let mut add = |id, source, payload| {
        store.import(
            id,
            source,
            vec![payload],
            &AtomicBool::new(false),
            &mut |_, _| {},
        )
    };
    add(
        "clip-a",
        Source::Clipboard,
        Payload::text(
            "曦玥的工作笔记\n支持中文、生僻字和多行文本。\n复制过的内容，可以重新复制，也可以收纳到中转站。",
        ),
    )?;
    add(
        "clip-b",
        Source::Clipboard,
        Payload::text("https://example.com/notes"),
    )?;
    add(
        "clip-c",
        Source::Clipboard,
        Payload::Files(vec![sources.join("演示笔记.txt")]),
    )?;
    add(
        "transfer-a",
        Source::Transfer,
        Payload::text("暂存的文字\n从这里拖到文本编辑器，或者另存为 UTF-8 文本文件。"),
    )?;
    add(
        "transfer-b",
        Source::Transfer,
        Payload::Files(vec![sources.join("项目资料"), sources.join("演示笔记.txt")]),
    )?;
    let frame = PathBuf::from("skins/orange-hood-girl-smooth/images/idle-000.png");
    let png = if frame.exists() {
        fs::read(frame)?
    } else {
        let mut data = std::io::Cursor::new(vec![]);
        image::RgbaImage::from_fn(256, 160, |x, y| {
            image::Rgba([90 + (x / 3) as u8, 100 + (y / 2) as u8, 220, 255])
        })
        .write_to(&mut data, image::ImageFormat::Png)?;
        data.into_inner()
    };
    add("transfer-z", Source::Transfer, Payload::Image(png))?;
    println!("Demo fixtures created at {}", directory.display());
    Ok(())
}
