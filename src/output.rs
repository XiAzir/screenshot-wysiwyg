use anyhow::{anyhow, Context, Result};
use image::codecs::png::PngEncoder;
use image::{ColorType, ImageEncoder};
use std::fs;
use std::mem::size_of;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use windows::Win32::Foundation::{HANDLE, HWND};
use windows::Win32::Graphics::Gdi::{BITMAPINFOHEADER, BI_RGB};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
};
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use windows::Win32::System::Ole::CF_DIB;

pub fn save_png_file(data: &[u8], width: u32, height: u32, dir: &str) -> Result<PathBuf> {
    if width == 0 || height == 0 {
        return Err(anyhow!("截图区域为空"));
    }
    if data.len() != (width as usize * height as usize * 4) {
        return Err(anyhow!("截图像素数据大小不正确"));
    }

    let dir = Path::new(dir);
    fs::create_dir_all(dir).context("创建截图保存目录失败")?;
    let path = dir.join(format!("screenshot_{}.png", timestamp_millis()));
    let file = fs::File::create(&path).context("创建截图文件失败")?;

    PngEncoder::new(file)
        .write_image(data, width, height, ColorType::Rgba8)
        .context("写入 PNG 失败")?;

    Ok(path)
}

pub fn copy_to_clipboard(owner: HWND, data: &[u8], width: u32, height: u32) -> Result<()> {
    if width == 0 || height == 0 {
        return Err(anyhow!("截图区域为空"));
    }
    if data.len() != (width as usize * height as usize * 4) {
        return Err(anyhow!("截图像素数据大小不正确"));
    }

    let header_size = size_of::<BITMAPINFOHEADER>();
    let row_stride = ((width as usize * 3) + 3) & !3;
    let pixel_size = row_stride * height as usize;
    let total_size = header_size + pixel_size;

    let hglobal =
        unsafe { GlobalAlloc(GMEM_MOVEABLE, total_size) }.context("分配剪贴板内存失败")?;
    let ptr = unsafe { GlobalLock(hglobal) };
    if ptr.is_null() {
        return Err(anyhow!("锁定剪贴板内存失败"));
    }

    unsafe {
        let header = BITMAPINFOHEADER {
            biSize: header_size as u32,
            biWidth: width as i32,
            biHeight: height as i32,
            biPlanes: 1,
            biBitCount: 24,
            biCompression: BI_RGB.0,
            biSizeImage: pixel_size as u32,
            biXPelsPerMeter: 0,
            biYPelsPerMeter: 0,
            biClrUsed: 0,
            biClrImportant: 0,
        };

        std::ptr::copy_nonoverlapping(
            &header as *const BITMAPINFOHEADER as *const u8,
            ptr as *mut u8,
            header_size,
        );

        let pixels = (ptr as *mut u8).add(header_size);
        std::ptr::write_bytes(pixels, 0, pixel_size);
        for y in 0..height as usize {
            let src_y = y;
            let dst_y = height as usize - 1 - y;
            let dst_row = pixels.add(dst_y * row_stride);
            for x in 0..width as usize {
                let src = (src_y * width as usize + x) * 4;
                let dst = dst_row.add(x * 3);
                *dst.add(0) = data[src + 2];
                *dst.add(1) = data[src + 1];
                *dst.add(2) = data[src];
            }
        }

        let _ = GlobalUnlock(hglobal);
    }

    unsafe {
        OpenClipboard(owner).context("打开剪贴板失败")?;
        let clipboard_result = (|| -> Result<()> {
            EmptyClipboard().context("清空剪贴板失败")?;
            SetClipboardData(CF_DIB.0 as u32, HANDLE(hglobal.0)).context("写入剪贴板失败")?;
            Ok(())
        })();
        CloseClipboard().context("关闭剪贴板失败")?;
        clipboard_result
    }
}

fn timestamp_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}
