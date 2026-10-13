use serde::{Deserialize, Serialize};
use std::io;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum DesktopSnapshot {
    Mac { previous: Option<bool> },
    Windows { previous: bool },
}

#[derive(Debug, Clone, Copy)]
pub struct Cursor {
    pub position: [f32; 2],
    pub work_area: [f32; 4],
    pub reference_height: f32,
}

#[cfg(target_os = "macos")]
pub use macos::{
    cursor, cursor_position, hide_icons, parent_alive, restore_icons, snapshot, work_areas,
};
#[cfg(target_os = "windows")]
pub use windows::{
    cursor, cursor_position, hide_icons, parent_alive, restore_icons, snapshot, work_areas,
};

pub fn supported() -> bool {
    cfg!(any(target_os = "windows", target_os = "macos"))
}

pub fn validate_snapshot(snapshot: &DesktopSnapshot) -> io::Result<()> {
    if matches!(snapshot, DesktopSnapshot::Mac { .. }) && cfg!(target_os = "macos")
        || matches!(snapshot, DesktopSnapshot::Windows { .. }) && cfg!(target_os = "windows")
    {
        Ok(())
    } else {
        Err(io::Error::other(
            "Desktop lease belongs to a different platform",
        ))
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub fn cursor(_scale: f32) -> Option<Cursor> {
    None
}
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub fn cursor_position(_scale: f32, _reference_height: f32) -> Option<[f32; 2]> {
    None
}
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub fn work_areas(_scale: f32) -> Vec<[f32; 4]> {
    Vec::new()
}
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub fn snapshot() -> io::Result<DesktopSnapshot> {
    Err(io::Error::other("Unsupported platform"))
}
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub fn hide_icons() -> io::Result<()> {
    Err(io::Error::other("Unsupported platform"))
}
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub fn restore_icons(_snapshot: &DesktopSnapshot) -> io::Result<()> {
    Err(io::Error::other("Unsupported platform"))
}
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub fn parent_alive(_pid: u32) -> bool {
    false
}
