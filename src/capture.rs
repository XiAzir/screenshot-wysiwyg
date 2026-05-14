use anyhow::{anyhow, Context, Result};
use windows::core::*;
use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Direct3D::*;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::*;

mod tone_map;

/// 显示器信息
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct MonitorInfo {
    pub index: usize,
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub x: i32,
    pub y: i32,
    pub is_primary: bool,
    adapter_index: usize,
    output_index: usize,
}

/// 截图结果
pub struct CapturedImage {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
    pub is_hdr_source: bool,
}

/// 截取虚拟桌面中的指定区域。
pub fn capture_region(rect: RECT) -> Result<CapturedImage> {
    let left = rect.left.min(rect.right);
    let top = rect.top.min(rect.bottom);
    let right = rect.left.max(rect.right);
    let bottom = rect.top.max(rect.bottom);
    let width = (right - left).max(0) as u32;
    let height = (bottom - top).max(0) as u32;

    if width == 0 || height == 0 {
        return Err(anyhow!("截图区域为空"));
    }

    let monitors = list_monitors().context("扫描显示器失败")?;
    let mut canvas = vec![0u8; width as usize * height as usize * 4];
    let mut touched = false;
    let mut hdr = false;

    for monitor in &monitors {
        let monitor_rect = RECT {
            left: monitor.x,
            top: monitor.y,
            right: monitor.x + monitor.width as i32,
            bottom: monitor.y + monitor.height as i32,
        };

        let Some(intersection) = intersect_rect(
            RECT {
                left,
                top,
                right,
                bottom,
            },
            monitor_rect,
        ) else {
            continue;
        };

        let image =
            capture_monitor(monitor).context(format!("截取显示器「{}」失败", monitor.name))?;
        hdr |= image.is_hdr_source;

        let copy_w = (intersection.right - intersection.left) as usize;
        let copy_h = (intersection.bottom - intersection.top) as usize;
        let src_x = (intersection.left - monitor.x) as usize;
        let src_y = (intersection.top - monitor.y) as usize;
        let dst_x = (intersection.left - left) as usize;
        let dst_y = (intersection.top - top) as usize;

        for row in 0..copy_h {
            let src_start = ((src_y + row) * image.width as usize + src_x) * 4;
            let dst_start = ((dst_y + row) * width as usize + dst_x) * 4;
            let bytes = copy_w * 4;
            canvas[dst_start..dst_start + bytes]
                .copy_from_slice(&image.data[src_start..src_start + bytes]);
        }

        touched = true;
    }

    if !touched {
        return Err(anyhow!("截图区域不在任何显示器内"));
    }

    Ok(CapturedImage {
        width,
        height,
        data: canvas,
        is_hdr_source: hdr,
    })
}

// ─── 常量 ───

/// D3D11_CPU_ACCESS_READ = 0x20000
const CPU_ACCESS_READ: u32 = 0x20000;

/// D3D11_CREATE_DEVICE_BGRA_SUPPORT 的实际 u32 值
const CREATE_DEVICE_BGRA_SUPPORT: u32 = 0x20;

/// 列出所有显示器
pub fn list_monitors() -> Result<Vec<MonitorInfo>> {
    let factory: IDXGIFactory1 =
        unsafe { CreateDXGIFactory1() }.context("创建 DXGI Factory 失败，请确认 DirectX 已安装")?;

    let mut monitors = Vec::new();
    let mut monitor_index = 0usize;

    for adapter_idx in 0.. {
        let adapter: IDXGIAdapter1 = match unsafe { factory.EnumAdapters1(adapter_idx) } {
            Ok(a) => a,
            Err(e) => {
                if e.code() == DXGI_ERROR_NOT_FOUND {
                    break;
                }
                continue;
            }
        };

        for output_idx in 0.. {
            let output: IDXGIOutput = match unsafe { adapter.EnumOutputs(output_idx) } {
                Ok(o) => o,
                Err(e) => {
                    if e.code() == DXGI_ERROR_NOT_FOUND {
                        break;
                    }
                    continue;
                }
            };

            let desc = match unsafe { output.GetDesc() } {
                Ok(d) => d,
                Err(_) => continue,
            };

            if !desc.AttachedToDesktop.as_bool() {
                continue;
            }

            let width =
                (desc.DesktopCoordinates.right - desc.DesktopCoordinates.left).max(0) as u32;
            let height =
                (desc.DesktopCoordinates.bottom - desc.DesktopCoordinates.top).max(0) as u32;

            let name = String::from_utf16_lossy(&desc.DeviceName)
                .trim_end_matches('\0')
                .to_string();

            let is_primary = desc.DesktopCoordinates.left == 0 && desc.DesktopCoordinates.top == 0;

            monitors.push(MonitorInfo {
                index: monitor_index,
                name: if name.is_empty() {
                    format!("显示器 {}", monitor_index)
                } else {
                    name
                },
                width,
                height,
                x: desc.DesktopCoordinates.left,
                y: desc.DesktopCoordinates.top,
                is_primary,
                adapter_index: adapter_idx as usize,
                output_index: output_idx as usize,
            });
            monitor_index += 1;
        }
    }

    if monitors.is_empty() {
        return Err(anyhow!("未找到任何显示器"));
    }

    Ok(monitors)
}

fn intersect_rect(a: RECT, b: RECT) -> Option<RECT> {
    let left = a.left.max(b.left);
    let top = a.top.max(b.top);
    let right = a.right.min(b.right);
    let bottom = a.bottom.min(b.bottom);

    if right <= left || bottom <= top {
        None
    } else {
        Some(RECT {
            left,
            top,
            right,
            bottom,
        })
    }
}

/// 截取指定显示器
pub fn capture_monitor(monitor: &MonitorInfo) -> Result<CapturedImage> {
    let factory: IDXGIFactory1 =
        unsafe { CreateDXGIFactory1() }.context("创建 DXGI Factory 失败")?;

    let adapter: IDXGIAdapter1 = unsafe { factory.EnumAdapters1(monitor.adapter_index as u32) }
        .context("无法枚举显卡适配器")?;

    let output: IDXGIOutput = unsafe { adapter.EnumOutputs(monitor.output_index as u32) }
        .context("无法枚举显示器输出")?;

    let output1: IDXGIOutput1 = output
        .cast()
        .context("该显示器不支持桌面复制（需要 IDXGIOutput1）")?;

    let adapter_base: IDXGIAdapter = adapter.cast()?;
    let (device, context) = create_d3d11_device(&adapter_base)?;

    let duplication: IDXGIOutputDuplication = unsafe { output1.DuplicateOutput(&device) }
        .context("无法复制桌面输出。可能有其他程序（如 OBS、录屏软件）正在占用桌面复制通道。")?;

    // GetDesc 返回 DXGI_OUTDUPL_DESC 结构体（不是 Result）
    let dup_desc = unsafe { duplication.GetDesc() };
    let format = dup_desc.ModeDesc.Format;

    // 获取最新帧
    let (texture, frame_info) = acquire_fresh_frame(&duplication)?;

    // GetDesc 需要传入指针参数
    let mut texture_desc = D3D11_TEXTURE2D_DESC::default();
    unsafe { texture.GetDesc(&mut texture_desc) };
    let width = texture_desc.Width;
    let height = texture_desc.Height;

    // 创建 staging 纹理 — 使用原始 u32 值，不使用类型化的 flag struct
    let staging_desc = D3D11_TEXTURE2D_DESC {
        Width: width,
        Height: height,
        MipLevels: 1,
        ArraySize: 1,
        Format: texture_desc.Format,
        SampleDesc: texture_desc.SampleDesc,
        Usage: D3D11_USAGE_STAGING,
        BindFlags: 0,
        CPUAccessFlags: CPU_ACCESS_READ,
        MiscFlags: 0,
    };

    let mut staging: Option<ID3D11Texture2D> = None;
    unsafe {
        device
            .CreateTexture2D(&staging_desc, None, Some(&mut staging))
            .context("创建 staging 纹理失败")?;
    }
    let staging = staging.context("staging 纹理为空")?;

    unsafe {
        context.CopyResource(&staging, &texture);
    }

    // 映射 staging 纹理
    let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
    let hr = unsafe { context.Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped)) };
    hr.context("映射纹理失败")?;

    let row_pitch = mapped.RowPitch as usize;
    let (mut data, is_hdr) = unsafe {
        match format {
            DXGI_FORMAT_B8G8R8A8_UNORM => {
                (read_bgra8(mapped.pData, row_pitch, width, height), false)
            }
            DXGI_FORMAT_R16G16B16A16_FLOAT => {
                let hdr_data = read_rgba16f(mapped.pData, row_pitch, width, height);
                (
                    tone_map::tone_map_hdr_to_sdr(&hdr_data, width, height),
                    true,
                )
            }
            DXGI_FORMAT_R10G10B10A2_UNORM => {
                (read_rgb10a2(mapped.pData, row_pitch, width, height), false)
            }
            _ => {
                return Err(anyhow!(
                    "不支持的像素格式: {:?}。请尝试关闭 HDR 后重试。",
                    format
                ));
            }
        }
    };

    unsafe {
        context.Unmap(&staging, 0);
    }

    // 绘制光标
    draw_cursor_from_frame(&mut data, &duplication, &frame_info, width, height).ok();

    // 释放帧
    unsafe {
        duplication.ReleaseFrame().ok();
    }

    Ok(CapturedImage {
        width,
        height,
        data,
        is_hdr_source: is_hdr,
    })
}

// ─── D3D11 设备创建 ───

fn create_d3d11_device(adapter: &IDXGIAdapter) -> Result<(ID3D11Device, ID3D11DeviceContext)> {
    let mut device: Option<ID3D11Device> = None;
    let mut context: Option<ID3D11DeviceContext> = None;

    let flags = D3D11_CREATE_DEVICE_FLAG(CREATE_DEVICE_BGRA_SUPPORT);

    unsafe {
        D3D11CreateDevice(
            Some(adapter),
            D3D_DRIVER_TYPE_UNKNOWN,
            None,
            flags,
            None,
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            Some(&mut context),
        )
    }
    .context("创建 D3D11 设备失败，请确认显卡驱动是否正常")?;

    Ok((
        device.context("D3D11 设备为空")?,
        context.context("D3D11 设备上下文为空")?,
    ))
}

// ─── 帧获取 ───

fn acquire_fresh_frame(
    duplication: &IDXGIOutputDuplication,
) -> Result<(ID3D11Texture2D, DXGI_OUTDUPL_FRAME_INFO)> {
    let mut frame_info = DXGI_OUTDUPL_FRAME_INFO::default();
    let mut resource: Option<IDXGIResource> = None;

    // 步骤1：丢弃 DuplicateOutput 时的初始缓冲帧（可能是空白帧）
    let init_result = unsafe { duplication.AcquireNextFrame(0, &mut frame_info, &mut resource) };
    if init_result.is_ok() {
        unsafe { duplication.ReleaseFrame().ok() };
    }

    // 步骤2：等待真正的桌面帧更新
    let mut frame_info = DXGI_OUTDUPL_FRAME_INFO::default();
    let mut resource: Option<IDXGIResource> = None;

    let result = unsafe { duplication.AcquireNextFrame(1000, &mut frame_info, &mut resource) };

    match result {
        Ok(()) => {}
        Err(e) => {
            let code = e.code();
            if code == DXGI_ERROR_WAIT_TIMEOUT {
                return Err(anyhow!(
                    "获取桌面帧超时。请稍微移动鼠标或活动一下桌面后重试。"
                ));
            }
            if code == DXGI_ERROR_ACCESS_LOST {
                return Err(anyhow!(
                    "桌面复制通道丢失（可能切换了分辨率或显示器配置），请重试。"
                ));
            }
            return Err(anyhow!("获取桌面帧失败: {}", e));
        }
    }

    // 步骤3：丢弃积压帧，拿到最新画面
    loop {
        let mut next_info = DXGI_OUTDUPL_FRAME_INFO::default();
        let mut next_resource: Option<IDXGIResource> = None;

        let next_result =
            unsafe { duplication.AcquireNextFrame(0, &mut next_info, &mut next_resource) };

        if next_result.is_ok() {
            unsafe { duplication.ReleaseFrame().ok() };
            frame_info = next_info;
            resource = next_resource;
        } else {
            break;
        }
    }

    let texture: ID3D11Texture2D = resource
        .context("帧资源为空")?
        .cast()
        .context("无法将帧资源转换为纹理")?;

    Ok((texture, frame_info))
}

// ─── 像素格式转换 ───

unsafe fn read_bgra8(
    pdata: *mut std::ffi::c_void,
    row_pitch: usize,
    width: u32,
    height: u32,
) -> Vec<u8> {
    let mut data = Vec::with_capacity((width * height * 4) as usize);
    let src = pdata as *const u8;

    for y in 0..height as usize {
        let row_start = y * row_pitch;
        for x in 0..width as usize {
            let p = row_start + x * 4;
            data.push(*src.add(p + 2)); // R
            data.push(*src.add(p + 1)); // G
            data.push(*src.add(p)); // B
            data.push(*src.add(p + 3)); // A
        }
    }

    data
}

unsafe fn read_rgba16f(
    pdata: *mut std::ffi::c_void,
    row_pitch: usize,
    width: u32,
    height: u32,
) -> Vec<f32> {
    let mut data = Vec::with_capacity((width * height * 4) as usize);
    let src = pdata as *const u16;
    let row_pitch_u16 = row_pitch / 2;

    for y in 0..height as usize {
        let row_start = y * row_pitch_u16;
        for x in 0..width as usize {
            let p = row_start + x * 4;
            data.push(half_to_f32(*src.add(p)));
            data.push(half_to_f32(*src.add(p + 1)));
            data.push(half_to_f32(*src.add(p + 2)));
            data.push(half_to_f32(*src.add(p + 3)));
        }
    }

    data
}

unsafe fn read_rgb10a2(
    pdata: *mut std::ffi::c_void,
    row_pitch: usize,
    width: u32,
    height: u32,
) -> Vec<u8> {
    let mut data = Vec::with_capacity((width * height * 4) as usize);
    let src = pdata as *const u32;
    let row_pitch_u32 = row_pitch / 4;

    for y in 0..height as usize {
        let row_start = y * row_pitch_u32;
        for x in 0..width as usize {
            let pixel = *src.add(row_start + x);
            let r = ((pixel & 0x3FF) as f32) / 1023.0;
            let g = (((pixel >> 10) & 0x3FF) as f32) / 1023.0;
            let b = (((pixel >> 20) & 0x3FF) as f32) / 1023.0;
            let a = (((pixel >> 30) & 0x3) as f32) / 3.0;

            data.push(srgb_encode(r));
            data.push(srgb_encode(g));
            data.push(srgb_encode(b));
            data.push((a * 255.0) as u8);
        }
    }

    data
}

// ─── 半精度浮点转换 ───

fn half_to_f32(h: u16) -> f32 {
    let h = h as u32;
    let sign = (h >> 15) & 1;
    let exp = (h >> 10) & 0x1F;
    let mantissa = h & 0x3FF;

    if exp == 0 {
        if mantissa == 0 {
            f32::from_bits(sign << 31)
        } else {
            let m = mantissa as f32 / 1024.0;
            let bits = (sign << 31) | (((127 - 14) as u32) << 23) | ((m * 8388608.0) as u32);
            f32::from_bits(bits)
        }
    } else if exp == 31 {
        if mantissa == 0 {
            f32::from_bits((sign << 31) | 0x7F80_0000)
        } else {
            f32::NAN
        }
    } else {
        let unbiased_exp = exp as i32 - 15;
        let new_exp = (unbiased_exp + 127) as u32;
        f32::from_bits((sign << 31) | (new_exp << 23) | (mantissa << 13))
    }
}

fn srgb_encode(linear: f32) -> u8 {
    let c = linear.clamp(0.0, 1.0);
    let encoded = if c <= 0.0031308 {
        12.92 * c
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    };
    (encoded * 255.0 + 0.5) as u8
}

// ─── 光标绘制 ───

fn draw_cursor_from_frame(
    data: &mut [u8],
    duplication: &IDXGIOutputDuplication,
    frame_info: &DXGI_OUTDUPL_FRAME_INFO,
    img_width: u32,
    img_height: u32,
) -> Result<()> {
    let shape_buf_size = frame_info.PointerShapeBufferSize;

    if shape_buf_size == 0 {
        return Ok(());
    }

    if !frame_info.PointerPosition.Visible.as_bool() {
        return Ok(());
    }

    let mut shape_info = DXGI_OUTDUPL_POINTER_SHAPE_INFO::default();
    let mut actual_size: u32 = 0;
    let mut shape_buffer: Vec<u8> = vec![0u8; shape_buf_size as usize];

    unsafe {
        duplication.GetFramePointerShape(
            shape_buf_size,
            shape_buffer.as_mut_ptr() as *mut std::ffi::c_void,
            &mut actual_size,
            &mut shape_info,
        )
    }
    .context("获取光标形状失败")?;

    if shape_info.Width == 0 || shape_info.Height == 0 {
        return Ok(());
    }

    let cursor_x = frame_info.PointerPosition.Position.x - shape_info.HotSpot.x;
    let cursor_y = frame_info.PointerPosition.Position.y - shape_info.HotSpot.y;
    let shape_type = shape_info.Type;

    match shape_type {
        2 => draw_color_cursor(
            data,
            img_width,
            img_height,
            &shape_buffer,
            shape_info.Width,
            shape_info.Height,
            cursor_x,
            cursor_y,
            shape_info.Pitch,
        ),
        4 => draw_masked_color_cursor(
            data,
            img_width,
            img_height,
            &shape_buffer,
            shape_info.Width,
            shape_info.Height,
            cursor_x,
            cursor_y,
            shape_info.Pitch,
        ),
        1 => draw_mono_cursor(
            data,
            img_width,
            img_height,
            &shape_buffer,
            shape_info.Width,
            shape_info.Height,
            cursor_x,
            cursor_y,
            shape_info.Pitch,
        ),
        _ => {}
    }

    Ok(())
}

fn draw_color_cursor(
    data: &mut [u8],
    img_w: u32,
    img_h: u32,
    cursor_buf: &[u8],
    cursor_w: u32,
    cursor_h: u32,
    pos_x: i32,
    pos_y: i32,
    pitch: u32,
) {
    let cursor_w_actual = cursor_w.min(pitch / 4);

    for cy in 0..cursor_h as i32 {
        for cx in 0..cursor_w_actual as i32 {
            let px = pos_x + cx;
            let py = pos_y + cy;

            if px < 0 || py < 0 || px >= img_w as i32 || py >= img_h as i32 {
                continue;
            }

            let img_idx = ((py as u32 * img_w + px as u32) * 4) as usize;
            let cursor_idx = (cy as u32 * pitch + cx as u32 * 4) as usize;

            if cursor_idx + 3 >= cursor_buf.len() || img_idx + 3 >= data.len() {
                continue;
            }

            let cb = cursor_buf[cursor_idx];
            let cg = cursor_buf[cursor_idx + 1];
            let cr = cursor_buf[cursor_idx + 2];
            let ca = cursor_buf[cursor_idx + 3];

            if ca == 0 {
                continue;
            }
            if ca == 255 {
                data[img_idx] = cr;
                data[img_idx + 1] = cg;
                data[img_idx + 2] = cb;
                data[img_idx + 3] = 255;
            } else {
                let alpha = ca as f32 / 255.0;
                let inv = 1.0 - alpha;
                data[img_idx] = (data[img_idx] as f32 * inv + cr as f32 * alpha) as u8;
                data[img_idx + 1] = (data[img_idx + 1] as f32 * inv + cg as f32 * alpha) as u8;
                data[img_idx + 2] = (data[img_idx + 2] as f32 * inv + cb as f32 * alpha) as u8;
            }
        }
    }
}

fn draw_masked_color_cursor(
    data: &mut [u8],
    img_w: u32,
    img_h: u32,
    cursor_buf: &[u8],
    cursor_w: u32,
    cursor_h: u32,
    pos_x: i32,
    pos_y: i32,
    pitch: u32,
) {
    let color_size = (pitch * cursor_h) as usize;
    let mask_pitch = ((cursor_w + 31) / 32) * 4;

    for cy in 0..cursor_h as i32 {
        for cx in 0..cursor_w as i32 {
            let px = pos_x + cx;
            let py = pos_y + cy;

            if px < 0 || py < 0 || px >= img_w as i32 || py >= img_h as i32 {
                continue;
            }

            let img_idx = ((py as u32 * img_w + px as u32) * 4) as usize;
            if img_idx + 3 >= data.len() {
                continue;
            }

            let mask_byte_idx = color_size + (cy as u32 * mask_pitch + (cx as u32 / 8)) as usize;
            let mask_bit = 0x80u8 >> (cx as u32 % 8);

            let and_on = if mask_byte_idx < cursor_buf.len() {
                cursor_buf[mask_byte_idx] & mask_bit != 0
            } else {
                true
            };

            let cursor_idx = (cy as u32 * pitch + cx as u32 * 4) as usize;
            if cursor_idx + 3 >= color_size {
                continue;
            }

            let cb = cursor_buf[cursor_idx];
            let cg = cursor_buf[cursor_idx + 1];
            let cr = cursor_buf[cursor_idx + 2];
            let ca = cursor_buf[cursor_idx + 3];

            if and_on && ca == 0 {
                continue;
            }
            if ca == 0 {
                continue;
            }
            if ca == 255 {
                data[img_idx] = cr;
                data[img_idx + 1] = cg;
                data[img_idx + 2] = cb;
                data[img_idx + 3] = 255;
            } else {
                let alpha = ca as f32 / 255.0;
                let inv = 1.0 - alpha;
                data[img_idx] = (data[img_idx] as f32 * inv + cr as f32 * alpha) as u8;
                data[img_idx + 1] = (data[img_idx + 1] as f32 * inv + cg as f32 * alpha) as u8;
                data[img_idx + 2] = (data[img_idx + 2] as f32 * inv + cb as f32 * alpha) as u8;
            }
        }
    }
}

fn draw_mono_cursor(
    data: &mut [u8],
    img_w: u32,
    img_h: u32,
    cursor_buf: &[u8],
    cursor_w: u32,
    cursor_h: u32,
    pos_x: i32,
    pos_y: i32,
    _pitch: u32,
) {
    let row_bytes = (cursor_w + 7) / 8;
    let half_size = (row_bytes * cursor_h) as usize;

    for cy in 0..cursor_h as i32 {
        for cx in 0..cursor_w as i32 {
            let px = pos_x + cx;
            let py = pos_y + cy;

            if px < 0 || py < 0 || px >= img_w as i32 || py >= img_h as i32 {
                continue;
            }

            let img_idx = ((py as u32 * img_w + px as u32) * 4) as usize;
            if img_idx + 3 >= data.len() {
                continue;
            }

            let byte_idx = (cy as u32 * row_bytes + cx as u32 / 8) as usize;
            let bit = 0x80u8 >> (cx as u8 % 8);

            let and_on = if byte_idx < half_size {
                cursor_buf[byte_idx] & bit != 0
            } else {
                false
            };

            let xor_on = if byte_idx + half_size < cursor_buf.len() {
                cursor_buf[byte_idx + half_size] & bit != 0
            } else {
                false
            };

            match (and_on, xor_on) {
                (false, false) => {
                    data[img_idx] = 0;
                    data[img_idx + 1] = 0;
                    data[img_idx + 2] = 0;
                    data[img_idx + 3] = 255;
                }
                (false, true) => {
                    data[img_idx] = 255;
                    data[img_idx + 1] = 255;
                    data[img_idx + 2] = 255;
                    data[img_idx + 3] = 255;
                }
                (true, false) => {
                    data[img_idx] = 255 - data[img_idx];
                    data[img_idx + 1] = 255 - data[img_idx + 1];
                    data[img_idx + 2] = 255 - data[img_idx + 2];
                }
                _ => {}
            }
        }
    }
}
