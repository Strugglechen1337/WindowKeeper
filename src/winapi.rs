//! Thin Win32 layer: monitors, window enumeration and placement.

use crate::filter::{should_track, WindowFacts};
use crate::geometry::{
    monitor_for_rect, resolve, MonitorDesc, MonitorRef, Placement, Rect, ShowState,
};
use crate::layout::Entry;
use crate::matching::Identity;
use std::collections::HashMap;
use std::ffi::c_void;
use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::{CloseHandle, BOOL, HWND, LPARAM, RECT};
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayDevicesW, EnumDisplayMonitors, GetMonitorInfoW, DISPLAY_DEVICEW, HDC, HMONITOR,
    MONITORINFO, MONITORINFOEXW,
};
use windows::Win32::System::Threading::{
    GetCurrentProcessId, OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
    PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::HiDpi::{
    GetDpiForMonitor, SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
    MDT_EFFECTIVE_DPI,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetClassNameW, GetWindow, GetWindowLongW, GetWindowPlacement, GetWindowRect,
    GetWindowTextW, GetWindowThreadProcessId, IsHungAppWindow, IsIconic, IsWindowVisible, IsZoomed,
    SetWindowPlacement, SetWindowPos, ShowWindow, GWL_EXSTYLE, GW_OWNER, SET_WINDOW_POS_FLAGS,
    SWP_NOACTIVATE, SWP_NOOWNERZORDER, SWP_NOZORDER, SW_SHOWMAXIMIZED, SW_SHOWMINNOACTIVE,
    SW_SHOWNOACTIVATE, WINDOWPLACEMENT, WINDOWPLACEMENT_FLAGS, WPF_RESTORETOMAXIMIZED,
    WS_EX_APPWINDOW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
};

/// A tracked live window together with its current saved-form entry.
pub struct Live {
    pub hwnd: isize,
    pub entry: Entry,
}

pub fn hwnd_from(id: isize) -> HWND {
    HWND(id as *mut c_void)
}

/// Enable per-monitor DPI awareness v2 (all coordinates are physical pixels).
pub fn set_dpi_awareness() {
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
}

fn from_wide(buf: &[u16]) -> String {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}

fn to_rect(r: &RECT) -> Rect {
    Rect::new(r.left, r.top, r.right - r.left, r.bottom - r.top)
}

fn describe_monitor(hmon: HMONITOR) -> Option<MonitorDesc> {
    let mut mi = MONITORINFOEXW::default();
    mi.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
    unsafe {
        if !GetMonitorInfoW(hmon, &mut mi as *mut MONITORINFOEXW as *mut MONITORINFO).as_bool() {
            return None;
        }
    }
    let gdi_name = from_wide(&mi.szDevice);
    let mut dpi_x = 96u32;
    let mut dpi_y = 96u32;
    unsafe {
        let _ = GetDpiForMonitor(hmon, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y);
    }
    // Stable device interface path of the attached display.
    let mut dd = DISPLAY_DEVICEW {
        cb: std::mem::size_of::<DISPLAY_DEVICEW>() as u32,
        ..Default::default()
    };
    let device_id = unsafe {
        if EnumDisplayDevicesW(PCWSTR(mi.szDevice.as_ptr()), 0, &mut dd, 1).as_bool() {
            from_wide(&dd.DeviceID)
        } else {
            String::new()
        }
    };
    // Adapter description.
    let mut adapter = String::new();
    for i in 0..16 {
        let mut ad = DISPLAY_DEVICEW {
            cb: std::mem::size_of::<DISPLAY_DEVICEW>() as u32,
            ..Default::default()
        };
        unsafe {
            if !EnumDisplayDevicesW(PCWSTR::null(), i, &mut ad, 0).as_bool() {
                break;
            }
        }
        if from_wide(&ad.DeviceName).eq_ignore_ascii_case(&gdi_name) {
            adapter = from_wide(&ad.DeviceString);
            break;
        }
    }
    Some(MonitorDesc {
        device_id,
        gdi_name,
        adapter,
        rect: to_rect(&mi.monitorInfo.rcMonitor),
        work: to_rect(&mi.monitorInfo.rcWork),
        dpi: dpi_x.max(1),
        primary: mi.monitorInfo.dwFlags & 1 != 0,
    })
}

/// All currently attached monitors.
pub fn monitors() -> Vec<MonitorDesc> {
    unsafe extern "system" fn cb(hmon: HMONITOR, _: HDC, _: *mut RECT, lp: LPARAM) -> BOOL {
        let v = &mut *(lp.0 as *mut Vec<MonitorDesc>);
        if let Some(d) = describe_monitor(hmon) {
            v.push(d);
        }
        BOOL(1)
    }
    let mut v: Vec<MonitorDesc> = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(None, None, Some(cb), LPARAM(&mut v as *mut _ as isize));
    }
    v
}

/// Offset between workspace coordinates (used by WINDOWPLACEMENT) and screen
/// coordinates: the primary monitor's taskbar offset.
fn workspace_offset(mons: &[MonitorDesc]) -> (i32, i32) {
    mons.iter()
        .find(|m| m.primary)
        .map(|m| (m.work.x - m.rect.x, m.work.y - m.rect.y))
        .unwrap_or((0, 0))
}

fn process_path(pid: u32, cache: &mut HashMap<u32, Option<String>>) -> Option<String> {
    cache
        .entry(pid)
        .or_insert_with(|| unsafe {
            let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
            let mut buf = vec![0u16; 1024];
            let mut len = buf.len() as u32;
            let r = QueryFullProcessImageNameW(
                h,
                PROCESS_NAME_WIN32,
                PWSTR(buf.as_mut_ptr()),
                &mut len,
            );
            let _ = CloseHandle(h);
            r.ok()?;
            Some(String::from_utf16_lossy(&buf[..len as usize]))
        })
        .clone()
}

fn placement_of(hwnd: HWND) -> Option<WINDOWPLACEMENT> {
    let mut wp = WINDOWPLACEMENT {
        length: std::mem::size_of::<WINDOWPLACEMENT>() as u32,
        ..Default::default()
    };
    unsafe { GetWindowPlacement(hwnd, &mut wp).ok()? };
    Some(wp)
}

/// Build the saved-form entry of a window if it is a tracked application window.
fn describe_window(
    hwnd: HWND,
    mons: &[MonitorDesc],
    own_pid: u32,
    cache: &mut HashMap<u32, Option<String>>,
) -> Option<Entry> {
    unsafe {
        if !IsWindowVisible(hwnd).as_bool() {
            return None;
        }
        let ex = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
        let mut cloaked = 0u32;
        let _ = DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED,
            &mut cloaked as *mut u32 as *mut c_void,
            std::mem::size_of::<u32>() as u32,
        );
        let hung = IsHungAppWindow(hwnd).as_bool();
        let mut facts = WindowFacts {
            visible: true,
            tool_window: ex & WS_EX_TOOLWINDOW.0 != 0,
            app_window_flag: ex & WS_EX_APPWINDOW.0 != 0,
            no_activate: ex & WS_EX_NOACTIVATE.0 != 0,
            cloaked: cloaked != 0,
            hung,
            has_owner: GetWindow(hwnd, GW_OWNER)
                .map(|h| !h.0.is_null())
                .unwrap_or(false),
            ..Default::default()
        };
        if facts.cloaked || facts.hung {
            return None;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        facts.own_process = pid == own_pid;
        let mut cls = [0u16; 256];
        let n = GetClassNameW(hwnd, &mut cls);
        facts.class_name = String::from_utf16_lossy(&cls[..n.max(0) as usize]);
        let mut title = [0u16; 512];
        let n = GetWindowTextW(hwnd, &mut title);
        facts.title = String::from_utf16_lossy(&title[..n.max(0) as usize]);
        let mut wr = RECT::default();
        GetWindowRect(hwnd, &mut wr).ok()?;
        let wp = placement_of(hwnd)?;
        let minimized = wp.showCmd == 2;
        let (off_x, off_y) = workspace_offset(mons);
        let normal_ws = to_rect(&wp.rcNormalPosition);
        facts.width = if minimized {
            normal_ws.w
        } else {
            wr.right - wr.left
        };
        facts.height = if minimized {
            normal_ws.h
        } else {
            wr.bottom - wr.top
        };
        if !should_track(&facts) {
            return None;
        }
        let exe = process_path(pid, cache)?;

        let maximized = wp.showCmd == 3;
        let state = if minimized {
            ShowState::Minimized
        } else if maximized {
            ShowState::Maximized
        } else {
            ShowState::Normal
        };
        // Normal windows: exact screen rect. Others: workspace rect -> screen.
        let rect = if state == ShowState::Normal {
            to_rect(&wr)
        } else {
            Rect::new(
                normal_ws.x + off_x,
                normal_ws.y + off_y,
                normal_ws.w,
                normal_ws.h,
            )
        };
        let probe = if state == ShowState::Maximized {
            to_rect(&wr)
        } else {
            rect
        };
        let mon = monitor_for_rect(&probe, mons)?;
        Some(Entry {
            identity: Identity {
                exe_path: exe,
                class_name: facts.class_name,
                title: facts.title,
            },
            placement: Placement {
                state,
                restore_maximized: minimized && wp.flags.0 & WPF_RESTORETOMAXIMIZED.0 != 0,
                rect,
                monitor: MonitorRef::from_desc(mon),
            },
        })
    }
}

/// Enumerate all tracked top-level windows.
pub fn enumerate(mons: &[MonitorDesc]) -> Vec<Live> {
    struct Ctx<'a> {
        mons: &'a [MonitorDesc],
        pid: u32,
        cache: HashMap<u32, Option<String>>,
        out: Vec<Live>,
    }
    unsafe extern "system" fn cb(hwnd: HWND, lp: LPARAM) -> BOOL {
        let ctx = &mut *(lp.0 as *mut Ctx);
        if let Some(entry) = describe_window(hwnd, ctx.mons, ctx.pid, &mut ctx.cache) {
            ctx.out.push(Live {
                hwnd: hwnd.0 as isize,
                entry,
            });
        }
        BOOL(1)
    }
    let mut ctx = Ctx {
        mons,
        pid: unsafe { GetCurrentProcessId() },
        cache: HashMap::new(),
        out: Vec::new(),
    };
    unsafe {
        let _ = EnumWindows(Some(cb), LPARAM(&mut ctx as *mut Ctx as isize));
    }
    ctx.out
}

fn to_ws_rect(r: &Rect, off: (i32, i32)) -> RECT {
    RECT {
        left: r.x - off.0,
        top: r.y - off.1,
        right: r.right() - off.0,
        bottom: r.bottom() - off.1,
    }
}

fn set_placement(hwnd: HWND, rect: &Rect, off: (i32, i32), cmd: i32, flags: u32) -> bool {
    let wp = WINDOWPLACEMENT {
        length: std::mem::size_of::<WINDOWPLACEMENT>() as u32,
        flags: WINDOWPLACEMENT_FLAGS(flags),
        showCmd: cmd as u32,
        ptMinPosition: windows::Win32::Foundation::POINT { x: -1, y: -1 },
        ptMaxPosition: windows::Win32::Foundation::POINT { x: -1, y: -1 },
        rcNormalPosition: to_ws_rect(rect, off),
    };
    unsafe { SetWindowPlacement(hwnd, &wp).is_ok() }
}

/// Apply a saved placement to a window. Returns true if the window was
/// touched successfully (or already in place).
pub fn apply(id: isize, p: &Placement, mons: &[MonitorDesc]) -> bool {
    let Some(res) = resolve(p, mons) else {
        return false;
    };
    let hwnd = hwnd_from(id);
    let off = workspace_offset(mons);
    let rect = res.rect;
    unsafe {
        match p.state {
            ShowState::Normal => {
                if IsIconic(hwnd).as_bool() || IsZoomed(hwnd).as_bool() {
                    let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
                }
                // Twice: a DPI change on the first move may resize the window.
                for _ in 0..2 {
                    let mut cur = RECT::default();
                    if GetWindowRect(hwnd, &mut cur).is_ok() && to_rect(&cur) == rect {
                        return true;
                    }
                    let flags: SET_WINDOW_POS_FLAGS =
                        SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOOWNERZORDER;
                    if SetWindowPos(hwnd, None, rect.x, rect.y, rect.w, rect.h, flags).is_err() {
                        return false;
                    }
                }
                true
            }
            ShowState::Maximized => {
                if IsZoomed(hwnd).as_bool() {
                    let mut cur = RECT::default();
                    if GetWindowRect(hwnd, &mut cur).is_ok() {
                        let same = monitor_for_rect(&to_rect(&cur), mons)
                            .zip(monitor_for_rect(&rect, mons))
                            .is_some_and(|(a, b)| a.gdi_name == b.gdi_name);
                        if same {
                            return true;
                        }
                    }
                }
                set_placement(hwnd, &rect, off, SW_SHOWMAXIMIZED.0, 0)
            }
            ShowState::Minimized => {
                if IsIconic(hwnd).as_bool() {
                    return true;
                }
                let flags = if p.restore_maximized {
                    WPF_RESTORETOMAXIMIZED.0
                } else {
                    0
                };
                set_placement(hwnd, &rect, off, SW_SHOWMINNOACTIVE.0, flags)
            }
        }
    }
}
