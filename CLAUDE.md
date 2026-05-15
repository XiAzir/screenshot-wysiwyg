# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## 项目概述

Kumokiri 截图工具，基于 Windows Desktop Duplication API 直接从显卡输出缓冲区捕获画面，规避 QQ/游戏截图的质量损失。

## 构建与运行

```powershell
cargo build --release          # 编译 release（无调试符号）
cargo build                    # 编译 debug
cargo build --release -p kumokiri  # 仅编主包
```

编译产物: `target/release/Kumokiri.exe`

测试截图:
```powershell
.\Kumokiri.exe
```

## 架构

```
src/main.rs        CLI 入口 (clap)，分发单次截图 / 热键后台 / 拼接模式
src/capture.rs     DDA 核心：枚举显示器 → D3D11 设备 → DuplicateOutput → 获取帧 → 像素读取
src/capture/tone_map.rs  HDR→SDR：Reinhard 扩展算子 + sRGB 编码
src/hotkey.rs      热键后台：RegisterHotKey(Ctrl+Alt+B) + 消息循环，截图自动存盘
```

### 数据流

1. `CreateDXGIFactory1` → 枚举 `IDXGIAdapter1` → `IDXGIOutput`
2. `IDXGIOutput1::DuplicateOutput(device)` → `IDXGIOutputDuplication`
3. `AcquireNextFrame` → 丢弃初始空白帧 → 等待真实帧 → 丢弃积压帧取最新
4. `CopyResource` → staging 纹理 → `Map` → 读像素
5. 格式: `B8G8R8A8_UNORM`(SDR) / `R16G16B16A16_FLOAT`(HDR→色调映射) / `R10G10B10A2_UNORM`
6. 叠加光标 → 编码输出(PNG/JPEG/BMP)

### 关键细节

- **DDA 第一帧是空白的**: `DuplicateOutput` 后必须丢弃初始帧再 `AcquireNextFrame(timeout)` 等真实帧
- **`D3D11_TEXTURE2D_DESC` 字段全部是 `u32`**: 不能用 `D3D11_BIND_FLAG` 等类型化常量，需用裸值 `0` / `0x20000`
- **`CreateTexture2D` 返回 `()`**, 通过输出参数 `Option<*mut Option<ID3D11Texture2D>>` 拿纹理
- **`Map` 返回 `HRESULT`**, 通过输出参数 `Option<*mut D3D11_MAPPED_SUBRESOURCE>` 拿映射数据
- **主显示器判定**: `DesktopCoordinates.left == 0 && top == 0`

## 依赖

- `windows = "0.58"` — Win32/DXGI/D3D11/UI 绑定（主要依赖，体积大）
- `image = "0.24"` — PNG/JPEG/BMP 编码
- `clap = "4"` — CLI 参数解析
- `anyhow = "1"` — 错误处理
