use super::{Cursor, DesktopSnapshot};
use objc2::MainThreadMarker;
use objc2_app_kit::{NSEvent, NSScreen};
use std::{
    io,
    process::{Command, Output, Stdio},
    thread,
    time::{Duration, Instant},
};

pub fn cursor_position(_scale: f32, reference_height: f32) -> Option<[f32; 2]> {
    let point = NSEvent::mouseLocation();
    Some([point.x as f32, reference_height - point.y as f32])
}

pub fn cursor(_scale: f32) -> Option<Cursor> {
    let marker = MainThreadMarker::new()?;
    let screens = NSScreen::screens(marker);
    let primary = screens.firstObject()?;
    let top = primary.frame().origin.y + primary.frame().size.height;
    let point = NSEvent::mouseLocation();
    for screen in screens.iter() {
        let bounds = screen.frame();
        if point.x >= bounds.origin.x
            && point.x <= bounds.origin.x + bounds.size.width
            && point.y >= bounds.origin.y
            && point.y <= bounds.origin.y + bounds.size.height
        {
            let work = screen.visibleFrame();
            return Some(Cursor {
                position: [point.x as f32, (top - point.y) as f32],
                work_area: [
                    work.origin.x as f32,
                    (top - work.origin.y - work.size.height) as f32,
                    work.size.width as f32,
                    work.size.height as f32,
                ],
                reference_height: top as f32,
            });
        }
    }
    None
}

pub fn work_areas(_scale: f32) -> Vec<[f32; 4]> {
    let Some(marker) = MainThreadMarker::new() else {
        return Vec::new();
    };
    let screens = NSScreen::screens(marker);
    let Some(primary) = screens.firstObject() else {
        return Vec::new();
    };
    let top = primary.frame().origin.y + primary.frame().size.height;
    screens
        .iter()
        .map(|screen| {
            let work = screen.visibleFrame();
            [
                work.origin.x as f32,
                (top - work.origin.y - work.size.height) as f32,
                work.size.width as f32,
                work.size.height as f32,
            ]
        })
        .collect()
}

fn run(program: &str, arguments: &[&str]) -> io::Result<Output> {
    let mut child = Command::new(program)
        .args(arguments)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if child.try_wait()?.is_some() {
            return child.wait_with_output();
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!("{program} timed out"),
            ));
        }
        thread::sleep(Duration::from_millis(25));
    }
}

fn preference() -> io::Result<Option<bool>> {
    let output = run(
        "/usr/bin/defaults",
        &["read", "com.apple.finder", "CreateDesktop"],
    )?;
    if output.status.success() {
        match String::from_utf8_lossy(&output.stdout).trim() {
            "1" | "true" | "YES" => Ok(Some(true)),
            "0" | "false" | "NO" => Ok(Some(false)),
            _ => Err(io::Error::other(
                "Unexpected Finder CreateDesktop value; leaving it untouched",
            )),
        }
    } else if String::from_utf8_lossy(&output.stderr).contains("does not exist") {
        Ok(None)
    } else {
        Err(io::Error::other(
            String::from_utf8_lossy(&output.stderr).into_owned(),
        ))
    }
}

fn restart_finder() -> io::Result<()> {
    let output = run("/usr/bin/killall", &["Finder"])?;
    if output.status.success()
        || String::from_utf8_lossy(&output.stderr).contains("No matching processes")
    {
        Ok(())
    } else {
        Err(io::Error::other(
            String::from_utf8_lossy(&output.stderr).into_owned(),
        ))
    }
}

pub fn snapshot() -> io::Result<DesktopSnapshot> {
    let previous = preference()?;
    if previous == Some(false) {
        return Err(io::Error::other(
            "Desktop icons were already hidden; preserving your setting",
        ));
    }
    Ok(DesktopSnapshot::Mac { previous })
}

pub fn hide_icons() -> io::Result<()> {
    let output = run(
        "/usr/bin/defaults",
        &[
            "write",
            "com.apple.finder",
            "CreateDesktop",
            "-bool",
            "false",
        ],
    )?;
    if !output.status.success() {
        return Err(io::Error::other("Cannot update Finder desktop visibility"));
    }
    restart_finder()
}

pub fn restore_icons(saved: &DesktopSnapshot) -> io::Result<()> {
    let DesktopSnapshot::Mac { previous } = saved else {
        return Err(io::Error::other("Invalid macOS snapshot"));
    };
    if preference()? != Some(false) {
        return Ok(());
    }
    let output = match previous {
        Some(value) => run(
            "/usr/bin/defaults",
            &[
                "write",
                "com.apple.finder",
                "CreateDesktop",
                "-bool",
                if *value { "true" } else { "false" },
            ],
        )?,
        None => run(
            "/usr/bin/defaults",
            &["delete", "com.apple.finder", "CreateDesktop"],
        )?,
    };
    if !output.status.success() {
        return Err(io::Error::other("Cannot restore Finder desktop visibility"));
    }
    restart_finder()
}

pub fn parent_alive(pid: u32) -> bool {
    unsafe {
        libc::kill(pid as libc::pid_t, 0) == 0
            || io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }
}
