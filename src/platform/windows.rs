use super::{Cursor, DesktopSnapshot};
use std::{io, ptr};
use windows_sys::Win32::{
    Foundation::{CloseHandle, HWND, LPARAM, POINT, RECT},
    Graphics::Gdi::{
        EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITOR_DEFAULTTONEAREST, MONITORINFO,
        MonitorFromPoint,
    },
    System::Threading::{GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION},
    UI::WindowsAndMessaging::{
        EnumWindows, FindWindowExW, GetCursorPos, IsWindowVisible, SW_HIDE, SW_SHOWNA, ShowWindow,
    },
};

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

unsafe extern "system" fn find_desktop(window: HWND, parameter: LPARAM) -> i32 {
    let shell_class = wide("SHELLDLL_DefView");
    let list_class = wide("SysListView32");
    let shell =
        unsafe { FindWindowExW(window, ptr::null_mut(), shell_class.as_ptr(), ptr::null()) };
    if !shell.is_null() {
        let icons =
            unsafe { FindWindowExW(shell, ptr::null_mut(), list_class.as_ptr(), ptr::null()) };
        if !icons.is_null() {
            unsafe {
                *(parameter as *mut HWND) = icons;
            }
            return 0;
        }
    }
    1
}

fn desktop() -> io::Result<HWND> {
    let mut result: HWND = ptr::null_mut();
    unsafe {
        EnumWindows(Some(find_desktop), &mut result as *mut HWND as LPARAM);
    }
    if result.is_null() {
        Err(io::Error::other("Explorer desktop icon window not found"))
    } else {
        Ok(result)
    }
}

pub fn cursor_position(scale: f32, _reference_height: f32) -> Option<[f32; 2]> {
    let mut point = POINT { x: 0, y: 0 };
    if unsafe { GetCursorPos(&mut point) } == 0 {
        return None;
    }
    Some([
        point.x as f32 / scale.max(0.1),
        point.y as f32 / scale.max(0.1),
    ])
}

pub fn cursor(scale: f32) -> Option<Cursor> {
    let mut position = POINT { x: 0, y: 0 };
    unsafe {
        if GetCursorPos(&mut position) == 0 {
            return None;
        }
        let monitor = MonitorFromPoint(position, MONITOR_DEFAULTTONEAREST);
        let mut info: MONITORINFO = std::mem::zeroed();
        info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        if GetMonitorInfoW(monitor, &mut info) == 0 {
            return None;
        }
        let scale = scale.max(0.1);
        Some(Cursor {
            position: [position.x as f32 / scale, position.y as f32 / scale],
            work_area: [
                info.rcWork.left as f32 / scale,
                info.rcWork.top as f32 / scale,
                (info.rcWork.right - info.rcWork.left) as f32 / scale,
                (info.rcWork.bottom - info.rcWork.top) as f32 / scale,
            ],
            reference_height: 0.0,
        })
    }
}

struct MonitorVisitor {
    scale: f32,
    areas: Vec<[f32; 4]>,
}

unsafe extern "system" fn collect_monitor(
    monitor: HMONITOR,
    _context: HDC,
    _rect: *mut RECT,
    parameter: LPARAM,
) -> i32 {
    let visitor = unsafe { &mut *(parameter as *mut MonitorVisitor) };
    let mut info: MONITORINFO = unsafe { std::mem::zeroed() };
    info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
    if unsafe { GetMonitorInfoW(monitor, &mut info) } != 0 {
        visitor.areas.push([
            info.rcWork.left as f32 / visitor.scale,
            info.rcWork.top as f32 / visitor.scale,
            (info.rcWork.right - info.rcWork.left) as f32 / visitor.scale,
            (info.rcWork.bottom - info.rcWork.top) as f32 / visitor.scale,
        ]);
    }
    1
}

pub fn work_areas(scale: f32) -> Vec<[f32; 4]> {
    let mut visitor = MonitorVisitor {
        scale: scale.max(0.1),
        areas: Vec::new(),
    };
    unsafe {
        EnumDisplayMonitors(
            ptr::null_mut(),
            ptr::null(),
            Some(collect_monitor),
            &mut visitor as *mut MonitorVisitor as LPARAM,
        );
    }
    visitor.areas
}

pub fn snapshot() -> io::Result<DesktopSnapshot> {
    let previous = unsafe { IsWindowVisible(desktop()?) != 0 };
    if !previous {
        return Err(io::Error::other(
            "Desktop icons were already hidden; preserving your setting",
        ));
    }
    Ok(DesktopSnapshot::Windows { previous })
}

pub fn hide_icons() -> io::Result<()> {
    let window = desktop()?;
    unsafe {
        ShowWindow(window, SW_HIDE);
    }
    if unsafe { IsWindowVisible(window) } != 0 {
        return Err(io::Error::other("Explorer refused to hide desktop icons"));
    }
    Ok(())
}

pub fn restore_icons(saved: &DesktopSnapshot) -> io::Result<()> {
    let DesktopSnapshot::Windows { previous } = saved else {
        return Err(io::Error::other("Invalid Windows snapshot"));
    };
    let window = desktop()?;
    if *previous && unsafe { IsWindowVisible(window) } == 0 {
        unsafe {
            ShowWindow(window, SW_SHOWNA);
        }
    }
    Ok(())
}

pub fn parent_alive(pid: u32) -> bool {
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if process.is_null() {
            return false;
        }
        let mut code = 0;
        let result = GetExitCodeProcess(process, &mut code);
        CloseHandle(process);
        result != 0 && code == 259
    }
}
