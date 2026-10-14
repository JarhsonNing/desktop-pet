//! Create isolated UI fixtures without reading/writing the system clipboard.
use desktop_pet::content::{Payload, Policy, Source, Store};
use std::{fs, path::PathBuf, sync::atomic::AtomicBool};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let directory = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .ok_or("Usage: seed_panel_demo NEW_DIRECTORY")?,
    );
    if directory.exists() {
        return Err("Use a new directory; existing data will not be overwritten".into());
    }
    fs::create_dir_all(&directory)?;
    fs::write(
        directory.join("config.json"),
        r#"{"name":"团团","clipboard_enabled":false,"roaming":false,"allow_hide_icons":false,"allow_cursor_chase":false}"#,
    )?;
    let originals = directory.join("originals");
    fs::create_dir_all(originals.join("资料"))?;
    fs::write(
        originals.join("资料/说明.txt"),
        "独立中转副本保留目录层级。",
    )?;
    fs::write(originals.join("明天的计划.txt"), "把今天的小事好好收起来。")?;
    let image = image::RgbaImage::from_fn(480, 300, |x, y| {
        if (x as i32 - 240).pow(2) + (y as i32 - 150).pow(2) < 95 * 95 {
            image::Rgba([195, 174, 219, 255])
        } else {
            image::Rgba([248, 236, 223, 255])
        }
    });
    let mut png = std::io::Cursor::new(Vec::new());
    image.write_to(&mut png, image::ImageFormat::Png)?;
    let mut store = Store::open(&directory, Policy::default())?;
    let fixtures = [
        (
            "demo-note",
            Source::Clipboard,
            vec![Payload::text(
                "工作笔记\n今天的待办事项：\n整理灵感与文件\n生僻字：曦玥珩璟團團𠮷",
            )],
        ),
        (
            "demo-link",
            Source::Clipboard,
            vec![Payload::text("https://github.com/JarhsonNing/desktop-pet")],
        ),
        (
            "demo-clip-image",
            Source::Clipboard,
            vec![Payload::Image(png.get_ref().clone())],
        ),
        (
            "demo-text",
            Source::Transfer,
            vec![Payload::text(
                "周末的小计划\n读一本书，拍一张照片，陪团团玩球。",
            )],
        ),
        (
            "demo-image",
            Source::Transfer,
            vec![Payload::Image(png.into_inner())],
        ),
        (
            "demo-files",
            Source::Transfer,
            vec![Payload::Files(vec![
                originals.join("资料"),
                originals.join("明天的计划.txt"),
            ])],
        ),
    ];
    for (id, source, payloads) in fixtures {
        store.import(
            id,
            source,
            payloads,
            &AtomicBool::new(false),
            &mut |_, _| {},
        )?;
    }
    println!("{}", directory.display());
    Ok(())
}
