/// HDR → SDR 色调映射
///
/// 将 scRGB 线性浮点数据（R16G16B16A16_FLOAT）映射到 sRGB 8-bit。
/// 使用改进的 Reinhard 算子 + sRGB 传递函数。

/// 将 HDR f32 像素数据色调映射为 RGBA8
pub fn tone_map_hdr_to_sdr(hdr_data: &[f32], width: u32, height: u32) -> Vec<u8> {
    let pixel_count = (width * height) as usize;
    let mut sdr = Vec::with_capacity(pixel_count * 4);

    // 找到场景最大亮度用于归一化
    let max_lum = find_max_luminance(hdr_data, pixel_count);

    // 白点：亮度高于此值的像素映射为纯白
    let l_white = max_lum.max(1.0);

    for i in 0..pixel_count {
        let r = hdr_data[i * 4];
        let g = hdr_data[i * 4 + 1];
        let b = hdr_data[i * 4 + 2];

        // Reinhard 扩展算子: L_final = L * (1 + L/L_white^2) / (1 + L)
        let r_mapped = reinhard_channel(r, l_white);
        let g_mapped = reinhard_channel(g, l_white);
        let b_mapped = reinhard_channel(b, l_white);

        // sRGB 编码
        sdr.push(srgb_encode(r_mapped));
        sdr.push(srgb_encode(g_mapped));
        sdr.push(srgb_encode(b_mapped));
        sdr.push(255);
    }

    sdr
}

/// 计算画面最大亮度值（BT.709 权重）
fn find_max_luminance(data: &[f32], pixel_count: usize) -> f32 {
    let mut max_lum = 0.0f32;
    for i in 0..pixel_count {
        let r = data[i * 4];
        let g = data[i * 4 + 1];
        let b = data[i * 4 + 2];
        let lum = 0.2126 * r + 0.7152 * g + 0.0722 * b;
        if lum > max_lum {
            max_lum = lum;
        }
    }
    max_lum
}

/// Reinhard 扩展单通道映射
fn reinhard_channel(c: f32, l_white: f32) -> f32 {
    if c <= 0.0 {
        return 0.0;
    }
    // L_final = L * (1 + L/L_white^2) / (1 + L)
    let numerator = c * (1.0 + c / (l_white * l_white));
    let denominator = 1.0 + c;
    numerator / denominator
}

/// 线性 RGB → sRGB 编码
fn srgb_encode(linear: f32) -> u8 {
    let c = linear.clamp(0.0, 1.0);
    let encoded = if c <= 0.0031308 {
        12.92 * c
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    };
    (encoded * 255.0 + 0.5) as u8
}
