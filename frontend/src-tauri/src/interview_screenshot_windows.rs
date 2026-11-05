//! User-triggered full-monitor capture. No files, shell commands or background capture.
use super::{bounded_size, Display};
use std::{mem::size_of, ptr::{null, null_mut}};
use windows_sys::Win32::{Foundation::{BOOL, LPARAM, RECT}, Graphics::Gdi::*};

struct Monitor { id: u32, rect: RECT, primary: bool }

unsafe extern "system" fn collect(monitor: HMONITOR, _: HDC, _: *mut RECT, data: LPARAM) -> BOOL {
    let mut info: MONITORINFOEXW = std::mem::zeroed();
    info.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
    if GetMonitorInfoW(monitor, &mut info.monitorInfo) == 0 { return 1; }
    // The Windows display device name is stable across enumeration order changes.
    let id = info.szDevice.iter().take_while(|&&c| c != 0)
        .fold(2166136261_u32, |hash, c| (hash ^ *c as u32).wrapping_mul(16777619));
    (&mut *(data as *mut Vec<Monitor>)).push(Monitor {
        id, rect: info.monitorInfo.rcMonitor,
        primary: info.monitorInfo.dwFlags & 1 != 0,
    });
    1
}

fn monitors() -> Result<Vec<Monitor>, String> {
    let mut items: Vec<Monitor> = Vec::new();
    let ok = unsafe { EnumDisplayMonitors(null_mut(), null(), Some(collect), &mut items as *mut _ as LPARAM) };
    if ok == 0 || items.is_empty() { return Err("无法读取 Windows 显示器，请检查当前桌面会话".into()); }
    items.sort_by_key(|m| (!m.primary, m.id));
    Ok(items)
}

pub(super) fn displays() -> Result<Vec<Display>, String> {
    Ok(monitors()?.into_iter().map(|m| Display {
        id: m.id, width: (m.rect.right - m.rect.left).max(0) as u64,
        height: (m.rect.bottom - m.rect.top).max(0) as u64, primary: m.primary,
    }).collect())
}

pub(super) fn capture(id: Option<u32>) -> Result<Vec<u8>, String> {
    let monitors = monitors()?;
    let m = match id {
        Some(id) => monitors.iter().find(|m| m.id == id),
        None => monitors.iter().find(|m| m.primary),
    }.ok_or("所选屏幕已断开，请刷新屏幕列表后重试")?;
    let source_width = m.rect.right - m.rect.left;
    let source_height = m.rect.bottom - m.rect.top;
    let (width, height) = bounded_size(source_width.max(0) as usize, source_height.max(0) as usize)?;
    // GDI resources are always released on this worker, including every error path.
    unsafe {
        let screen = GetDC(null_mut());
        if screen.is_null() { return Err("无法访问当前 Windows 桌面".into()); }
        let memory = CreateCompatibleDC(screen);
        let bitmap = CreateCompatibleBitmap(screen, width as i32, height as i32);
        let result = (|| {
            if memory.is_null() || bitmap.is_null() { return Err("无法分配截图缓冲区".into()); }
            let old = SelectObject(memory, bitmap);
            if old.is_null() || old as isize == -1 { return Err("无法创建截图画布".into()); }
            SetStretchBltMode(memory, HALFTONE);
            let copied = StretchBlt(memory, 0, 0, width as i32, height as i32,
                screen, m.rect.left, m.rect.top, source_width, source_height, SRCCOPY | CAPTUREBLT);
            SelectObject(memory, old);
            if copied == 0 { return Err("Windows 截图失败，请检查屏幕连接或受保护桌面".into()); }
            let mut info: BITMAPINFO = std::mem::zeroed();
            info.bmiHeader = BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32, biWidth: width as i32,
                biHeight: -(height as i32), biPlanes: 1, biBitCount: 32,
                biCompression: BI_RGB, ..std::mem::zeroed()
            };
            let mut pixels = vec![0_u8; width * height * 4];
            if GetDIBits(screen, bitmap, 0, height as u32, pixels.as_mut_ptr().cast(), &mut info, DIB_RGB_COLORS) != height as i32 {
                return Err("无法读取截图像素".into());
            }
            let rgb: Vec<u8> = pixels.chunks_exact(4).flat_map(|p| [p[2], p[1], p[0]]).collect();
            let mut bytes = Vec::new();
            {
                let mut encoder = png::Encoder::new(&mut bytes, width as u32, height as u32);
                encoder.set_color(png::ColorType::Rgb);
                encoder.set_depth(png::BitDepth::Eight);
                let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
                writer.write_image_data(&rgb).map_err(|e| e.to_string())?;
            }
            if bytes.len() > interview_core::MAX_SCREENSHOT_BYTES { return Err("截图超过 10 MiB，请降低屏幕分辨率后重试".into()); }
            Ok(bytes)
        })();
        if !bitmap.is_null() { DeleteObject(bitmap); }
        if !memory.is_null() { DeleteDC(memory); }
        ReleaseDC(null_mut(), screen);
        result
    }
}
