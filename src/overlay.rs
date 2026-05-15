use crate::capture::DesktopSnapshot;
use anyhow::{anyhow, Context, Result};
use std::ffi::c_void;
use std::mem::size_of;
use windows::core::w;
use windows::Win32::Foundation::{
    BOOL, COLORREF, HANDLE, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM,
};
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_EXTENDED_FRAME_BOUNDS};
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC, ReleaseDC, SelectObject,
    AC_SRC_ALPHA, AC_SRC_OVER, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, BLENDFUNCTION, DIB_RGB_COLORS,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    RegisterHotKey, ReleaseCapture, SetActiveWindow, SetCapture, SetFocus, UnregisterHotKey,
    HOT_KEY_MODIFIERS, MOD_NOREPEAT, VK_ESCAPE, VK_RETURN,
};
use windows::Win32::UI::WindowsAndMessaging::{
    BringWindowToTop, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW,
    EnumWindows, GetClassNameW, GetCursorPos, GetWindowLongPtrW, GetWindowLongW, GetWindowRect,
    IsIconic, IsWindowVisible, LoadCursorW, RegisterClassW, SetForegroundWindow, SetWindowLongPtrW,
    ShowWindow, TranslateMessage, UpdateLayeredWindow, CREATESTRUCTW, CS_HREDRAW, CS_VREDRAW,
    GWLP_USERDATA, GWL_EXSTYLE, HTCLIENT, IDC_ARROW, MSG, SW_HIDE, SW_SHOW, ULW_ALPHA, WM_CLOSE,
    WM_HOTKEY, WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_NCCREATE, WM_NCDESTROY,
    WM_NCHITTEST, WNDCLASSW, WS_EX_LAYERED, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

const CLASS_NAME: windows::core::PCWSTR = w!("KumokiriOverlay");
const HOTKEY_ENTER_ID: i32 = 701;
const HOTKEY_ESCAPE_ID: i32 = 702;
const HANDLE_SIZE: i32 = 8;
const HIT_PAD: i32 = 8;
const MIN_SIZE: i32 = 6;
const DIM_NUMERATOR: u16 = 166;
const DIM_DENOMINATOR: u16 = 255;
const BORDER_R: u8 = 255;
const BORDER_G: u8 = 215;
const BORDER_B: u8 = 0;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Picking,
    Adjusting,
}

#[derive(Clone, Copy)]
enum Drag {
    None,
    PendingPick,
    Drawing,
    Moving,
    Resizing(Edges),
}

#[derive(Clone, Copy)]
struct Edges {
    left: bool,
    top: bool,
    right: bool,
    bottom: bool,
}

struct OverlayState {
    hwnd: HWND,
    bounds: RECT,
    monitor_rects: Vec<RECT>,
    window_rects: Vec<RECT>,
    frozen_bgra: Vec<u8>,
    frozen_width: usize,
    frozen_height: usize,
    selection: RECT,
    mode: Mode,
    drag: Drag,
    start: POINT,
    original: RECT,
    done: bool,
    result: Option<RECT>,
}

pub fn select_region(snapshot: &DesktopSnapshot) -> Result<Option<RECT>> {
    let bounds = snapshot.bounds;
    let monitor_rects = if snapshot.monitor_rects.is_empty() {
        vec![bounds]
    } else {
        snapshot.monitor_rects.clone()
    };
    let window_rects = snapshot_window_rects();
    let initial_selection = cursor_screen_point()
        .and_then(|pt| {
            window_rect_at_point(pt, &window_rects)
                .or_else(|| monitor_rect_at_point(pt, &monitor_rects))
        })
        .unwrap_or(bounds);

    let mut state = Box::new(OverlayState {
        hwnd: HWND::default(),
        bounds,
        monitor_rects,
        window_rects,
        frozen_bgra: rgba_to_bgra(
            &snapshot.image.data,
            snapshot.image.width,
            snapshot.image.height,
        )?,
        frozen_width: snapshot.image.width as usize,
        frozen_height: snapshot.image.height as usize,
        selection: initial_selection,
        mode: Mode::Picking,
        drag: Drag::None,
        start: POINT::default(),
        original: bounds,
        done: false,
        result: None,
    });
    let state_ptr: *mut OverlayState = &mut *state;

    let hmodule = unsafe { GetModuleHandleW(None) }.context("获取模块句柄失败")?;
    let hinstance = hmodule.into();
    let cursor = unsafe { LoadCursorW(None, IDC_ARROW) }.unwrap_or_default();
    let wc = WNDCLASSW {
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: Some(overlay_wndproc),
        hInstance: hinstance,
        hCursor: cursor,
        lpszClassName: CLASS_NAME,
        ..Default::default()
    };
    unsafe {
        RegisterClassW(&wc);
    }

    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
            CLASS_NAME,
            w!("截图选区"),
            WS_POPUP,
            bounds.left,
            bounds.top,
            bounds.right - bounds.left,
            bounds.bottom - bounds.top,
            None,
            None,
            hinstance,
            Some(state_ptr as *const _),
        )
    }
    .context("创建截图选区窗口失败")?;

    state.hwnd = hwnd;
    render_layered_overlay(&state).context("绘制冻结画面失败")?;
    unsafe {
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = BringWindowToTop(hwnd);
        let _ = SetForegroundWindow(hwnd);
        let _ = SetActiveWindow(hwnd);
        let _ = SetFocus(hwnd);
        let _ = RegisterHotKey(
            hwnd,
            HOTKEY_ENTER_ID,
            HOT_KEY_MODIFIERS(MOD_NOREPEAT.0),
            VK_RETURN.0 as u32,
        );
        let _ = RegisterHotKey(
            hwnd,
            HOTKEY_ESCAPE_ID,
            HOT_KEY_MODIFIERS(MOD_NOREPEAT.0),
            VK_ESCAPE.0 as u32,
        );
    }

    let mut msg = MSG::default();
    loop {
        if state.done {
            break;
        }
        let ret = unsafe {
            windows::Win32::UI::WindowsAndMessaging::GetMessageW(&mut msg, HWND::default(), 0, 0)
        };
        if ret.0 <= 0 {
            state.done = true;
            break;
        }
        if msg.message == WM_HOTKEY {
            let vk = ((msg.lParam.0 >> 16) & 0xffff) as u32;
            if vk == VK_RETURN.0 as u32 {
                complete_selection(hwnd, &mut state);
            } else if vk == VK_ESCAPE.0 as u32 {
                cancel_selection(hwnd, &mut state);
            }
            continue;
        }
        unsafe {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }

    unsafe {
        let _ = UnregisterHotKey(hwnd, HOTKEY_ENTER_ID);
        let _ = UnregisterHotKey(hwnd, HOTKEY_ESCAPE_ID);
    }

    Ok(state.result)
}

unsafe extern "system" fn overlay_wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == WM_NCCREATE {
        let create = lparam.0 as *const CREATESTRUCTW;
        let state = (*create).lpCreateParams as *mut OverlayState;
        (*state).hwnd = hwnd;
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, state as isize);
        return LRESULT(1);
    }

    let state_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut OverlayState;
    if state_ptr.is_null() {
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }
    let state = &mut *state_ptr;

    match msg {
        WM_NCHITTEST => LRESULT(HTCLIENT as isize),
        WM_MOUSEMOVE => {
            let pt =
                cursor_screen_point().unwrap_or_else(|| point_from_lparam(lparam, state.bounds));
            handle_mouse_move(state, pt);
            let _ = render_layered_overlay(state);
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            let pt =
                cursor_screen_point().unwrap_or_else(|| point_from_lparam(lparam, state.bounds));
            state.start = pt;
            state.original = state.selection;
            state.drag = begin_drag(state, pt);
            SetCapture(hwnd);
            let _ = SetForegroundWindow(hwnd);
            let _ = SetActiveWindow(hwnd);
            let _ = SetFocus(hwnd);
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let pt =
                cursor_screen_point().unwrap_or_else(|| point_from_lparam(lparam, state.bounds));
            finish_drag(state, pt);
            let _ = ReleaseCapture();
            let _ = render_layered_overlay(state);
            LRESULT(0)
        }
        WM_KEYDOWN => {
            match wparam.0 as u32 {
                0x0D => complete_selection(hwnd, state),
                0x1B => cancel_selection(hwnd, state),
                _ => {}
            }
            LRESULT(0)
        }
        WM_HOTKEY => {
            match wparam.0 as i32 {
                HOTKEY_ENTER_ID => complete_selection(hwnd, state),
                HOTKEY_ESCAPE_ID => cancel_selection(hwnd, state),
                _ => {}
            }
            LRESULT(0)
        }
        WM_CLOSE => {
            cancel_selection(hwnd, state);
            LRESULT(0)
        }
        WM_NCDESTROY => {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

fn complete_selection(hwnd: HWND, state: &mut OverlayState) {
    let rect = normalize_rect(state.selection);
    if rect.right - rect.left >= MIN_SIZE && rect.bottom - rect.top >= MIN_SIZE {
        state.result = Some(rect);
        state.done = true;
        unsafe {
            let _ = ShowWindow(hwnd, SW_HIDE);
            let _ = DestroyWindow(hwnd);
        }
    }
}

fn cancel_selection(hwnd: HWND, state: &mut OverlayState) {
    state.result = None;
    state.done = true;
    unsafe {
        let _ = ShowWindow(hwnd, SW_HIDE);
        let _ = DestroyWindow(hwnd);
    }
}

fn handle_mouse_move(state: &mut OverlayState, pt: POINT) {
    match state.drag {
        Drag::None => {
            if state.mode == Mode::Picking {
                state.selection =
                    window_rect_at_point(pt, &state.window_rects).unwrap_or_else(|| {
                        monitor_rect_at_point(pt, &state.monitor_rects).unwrap_or(state.bounds)
                    });
                state.selection = clamp_rect(state.selection, state.bounds);
            }
        }
        Drag::PendingPick => {
            if distance_exceeds_threshold(state.start, pt) {
                state.drag = Drag::Drawing;
                state.selection = clamp_rect(rect_from_points(state.start, pt), state.bounds);
            }
        }
        Drag::Drawing => {
            state.selection = clamp_rect(rect_from_points(state.start, pt), state.bounds);
        }
        Drag::Moving => {
            state.selection = move_rect_clamped(
                state.original,
                pt.x - state.start.x,
                pt.y - state.start.y,
                state.bounds,
            );
        }
        Drag::Resizing(edges) => {
            state.selection = resize_rect_clamped(state.original, edges, pt, state.bounds);
        }
    }
}

fn begin_drag(state: &OverlayState, pt: POINT) -> Drag {
    if state.mode == Mode::Picking {
        return Drag::PendingPick;
    }

    if let Some(edges) = hit_test_resize(state.selection, pt) {
        return Drag::Resizing(edges);
    }
    if point_in_rect(pt, normalize_rect(state.selection)) {
        return Drag::Moving;
    }
    Drag::Drawing
}

fn finish_drag(state: &mut OverlayState, pt: POINT) {
    match state.drag {
        Drag::PendingPick => {
            state.mode = Mode::Adjusting;
        }
        Drag::Drawing => {
            state.selection = clamp_rect(rect_from_points(state.start, pt), state.bounds);
            state.mode = Mode::Adjusting;
        }
        Drag::Moving | Drag::Resizing(_) => {
            state.selection = clamp_rect(normalize_rect(state.selection), state.bounds);
            state.mode = Mode::Adjusting;
        }
        Drag::None => {}
    }
    state.drag = Drag::None;
}

fn render_layered_overlay(state: &OverlayState) -> Result<()> {
    if state.frozen_width == 0 || state.frozen_height == 0 {
        return Err(anyhow!("冻结画面尺寸无效"));
    }

    let mut pixels = state.frozen_bgra.clone();
    let mut local = normalize_rect(state.selection);
    local.left -= state.bounds.left;
    local.right -= state.bounds.left;
    local.top -= state.bounds.top;
    local.bottom -= state.bounds.top;
    local = clamp_rect(
        local,
        RECT {
            left: 0,
            top: 0,
            right: state.frozen_width as i32,
            bottom: state.frozen_height as i32,
        },
    );

    for rect in dim_rects(
        RECT {
            left: 0,
            top: 0,
            right: state.frozen_width as i32,
            bottom: state.frozen_height as i32,
        },
        local,
    ) {
        dim_rect_bgra(&mut pixels, state.frozen_width, state.frozen_height, rect);
    }

    draw_selection_frame(&mut pixels, state.frozen_width, state.frozen_height, local);

    unsafe {
        update_layered_bitmap(
            state.hwnd,
            state.bounds,
            state.frozen_width as i32,
            state.frozen_height as i32,
            &mut pixels,
        )
    }
}

unsafe fn update_layered_bitmap(
    hwnd: HWND,
    bounds: RECT,
    width: i32,
    height: i32,
    pixels: &mut [u8],
) -> Result<()> {
    let screen_dc = GetDC(HWND::default());
    if screen_dc.0.is_null() {
        return Err(anyhow!("获取屏幕 DC 失败"));
    }

    let mem_dc = CreateCompatibleDC(screen_dc);
    if mem_dc.0.is_null() {
        let _ = ReleaseDC(HWND::default(), screen_dc);
        return Err(anyhow!("创建内存 DC 失败"));
    }

    let mut bmi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width,
            biHeight: -height,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            biSizeImage: pixels.len() as u32,
            biXPelsPerMeter: 0,
            biYPelsPerMeter: 0,
            biClrUsed: 0,
            biClrImportant: 0,
        },
        ..Default::default()
    };
    let mut bits: *mut c_void = std::ptr::null_mut();
    let bitmap = match CreateDIBSection(
        screen_dc,
        &mut bmi,
        DIB_RGB_COLORS,
        &mut bits,
        HANDLE::default(),
        0,
    ) {
        Ok(bitmap) => bitmap,
        Err(err) => {
            let _ = DeleteDC(mem_dc);
            let _ = ReleaseDC(HWND::default(), screen_dc);
            return Err(err).context("创建覆盖层位图失败");
        }
    };

    std::ptr::copy_nonoverlapping(pixels.as_ptr(), bits as *mut u8, pixels.len());
    let old = SelectObject(mem_dc, bitmap);

    let dst = POINT {
        x: bounds.left,
        y: bounds.top,
    };
    let size = SIZE {
        cx: width,
        cy: height,
    };
    let src = POINT { x: 0, y: 0 };
    let blend = BLENDFUNCTION {
        BlendOp: AC_SRC_OVER as u8,
        BlendFlags: 0,
        SourceConstantAlpha: 255,
        AlphaFormat: AC_SRC_ALPHA as u8,
    };

    let result = UpdateLayeredWindow(
        hwnd,
        screen_dc,
        Some(&dst),
        Some(&size),
        mem_dc,
        Some(&src),
        COLORREF(0),
        Some(&blend),
        ULW_ALPHA,
    )
    .context("更新覆盖层窗口失败");

    let _ = SelectObject(mem_dc, old);
    let _ = DeleteObject(bitmap);
    let _ = DeleteDC(mem_dc);
    let _ = ReleaseDC(HWND::default(), screen_dc);
    result
}

fn rgba_to_bgra(data: &[u8], width: u32, height: u32) -> Result<Vec<u8>> {
    let expected = width as usize * height as usize * 4;
    if data.len() != expected {
        return Err(anyhow!("冻结画面的像素数据大小不正确"));
    }

    let mut converted = Vec::with_capacity(expected);
    for pixel in data.chunks_exact(4) {
        converted.push(pixel[2]);
        converted.push(pixel[1]);
        converted.push(pixel[0]);
        converted.push(255);
    }
    Ok(converted)
}

fn dim_rect_bgra(pixels: &mut [u8], width: usize, height: usize, rect: RECT) {
    let rect = clamp_rect(
        rect,
        RECT {
            left: 0,
            top: 0,
            right: width as i32,
            bottom: height as i32,
        },
    );
    if rect.right <= rect.left || rect.bottom <= rect.top {
        return;
    }

    for y in rect.top as usize..rect.bottom as usize {
        for x in rect.left as usize..rect.right as usize {
            let idx = (y * width + x) * 4;
            pixels[idx] = scale_dim(pixels[idx]);
            pixels[idx + 1] = scale_dim(pixels[idx + 1]);
            pixels[idx + 2] = scale_dim(pixels[idx + 2]);
            pixels[idx + 3] = 255;
        }
    }
}

fn scale_dim(channel: u8) -> u8 {
    ((channel as u16 * DIM_NUMERATOR + DIM_DENOMINATOR / 2) / DIM_DENOMINATOR) as u8
}

fn fill_rect_bgra(pixels: &mut [u8], width: usize, height: usize, rect: RECT, r: u8, g: u8, b: u8) {
    let rect = clamp_rect(
        rect,
        RECT {
            left: 0,
            top: 0,
            right: width as i32,
            bottom: height as i32,
        },
    );
    if rect.right <= rect.left || rect.bottom <= rect.top {
        return;
    }

    for y in rect.top as usize..rect.bottom as usize {
        for x in rect.left as usize..rect.right as usize {
            let idx = (y * width + x) * 4;
            pixels[idx] = b;
            pixels[idx + 1] = g;
            pixels[idx + 2] = r;
            pixels[idx + 3] = 255;
        }
    }
}

fn draw_selection_frame(pixels: &mut [u8], width: usize, height: usize, rect: RECT) {
    let rect = normalize_rect(rect);
    let thickness = 3;
    fill_rect_bgra(
        pixels,
        width,
        height,
        RECT {
            left: rect.left,
            top: rect.top,
            right: rect.right,
            bottom: rect.top + thickness,
        },
        BORDER_R,
        BORDER_G,
        BORDER_B,
    );
    fill_rect_bgra(
        pixels,
        width,
        height,
        RECT {
            left: rect.left,
            top: rect.bottom - thickness,
            right: rect.right,
            bottom: rect.bottom,
        },
        BORDER_R,
        BORDER_G,
        BORDER_B,
    );
    fill_rect_bgra(
        pixels,
        width,
        height,
        RECT {
            left: rect.left,
            top: rect.top,
            right: rect.left + thickness,
            bottom: rect.bottom,
        },
        BORDER_R,
        BORDER_G,
        BORDER_B,
    );
    fill_rect_bgra(
        pixels,
        width,
        height,
        RECT {
            left: rect.right - thickness,
            top: rect.top,
            right: rect.right,
            bottom: rect.bottom,
        },
        BORDER_R,
        BORDER_G,
        BORDER_B,
    );

    for handle in handle_rects(rect) {
        fill_rect_bgra(pixels, width, height, handle, BORDER_R, BORDER_G, BORDER_B);
    }
}

fn dim_rects(client: RECT, selection: RECT) -> [RECT; 4] {
    let selection = normalize_rect(selection);
    [
        RECT {
            left: client.left,
            top: client.top,
            right: client.right,
            bottom: selection.top.max(client.top),
        },
        RECT {
            left: client.left,
            top: selection.bottom.min(client.bottom),
            right: client.right,
            bottom: client.bottom,
        },
        RECT {
            left: client.left,
            top: selection.top.max(client.top),
            right: selection.left.max(client.left),
            bottom: selection.bottom.min(client.bottom),
        },
        RECT {
            left: selection.right.min(client.right),
            top: selection.top.max(client.top),
            right: client.right,
            bottom: selection.bottom.min(client.bottom),
        },
    ]
}

fn monitor_rect_at_point(pt: POINT, monitor_rects: &[RECT]) -> Option<RECT> {
    monitor_rects
        .iter()
        .copied()
        .find(|rect| point_in_rect(pt, *rect))
}

fn window_rect_at_point(pt: POINT, window_rects: &[RECT]) -> Option<RECT> {
    window_rects
        .iter()
        .copied()
        .find(|rect| point_in_rect(pt, *rect))
}

fn snapshot_window_rects() -> Vec<RECT> {
    struct EnumData {
        rects: Vec<RECT>,
    }

    unsafe extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let data = &mut *(lparam.0 as *mut EnumData);

        if !IsWindowVisible(hwnd).as_bool() || IsIconic(hwnd).as_bool() {
            return BOOL(1);
        }

        let ex_style = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
        if ex_style & WS_EX_TOOLWINDOW.0 != 0 {
            return BOOL(1);
        }

        if is_shell_desktop_window(hwnd) {
            return BOOL(1);
        }

        let rect = normalize_rect(window_visible_rect(hwnd));
        if rect.right - rect.left < 16 || rect.bottom - rect.top < 16 {
            return BOOL(1);
        }

        data.rects.push(rect);
        BOOL(1)
    }

    let mut data = EnumData { rects: Vec::new() };
    unsafe {
        let _ = EnumWindows(Some(enum_proc), LPARAM(&mut data as *mut _ as isize));
    }
    data.rects
}

fn cursor_screen_point() -> Option<POINT> {
    let mut pt = POINT::default();
    unsafe { GetCursorPos(&mut pt).ok()? };
    Some(pt)
}

fn point_from_lparam(lparam: LPARAM, bounds: RECT) -> POINT {
    let x = (lparam.0 & 0xffff) as u16 as i16 as i32 + bounds.left;
    let y = ((lparam.0 >> 16) & 0xffff) as u16 as i16 as i32 + bounds.top;
    POINT { x, y }
}

unsafe fn is_shell_desktop_window(hwnd: HWND) -> bool {
    matches!(
        window_class_name(hwnd).as_str(),
        "Progman" | "WorkerW" | "Shell_TrayWnd" | "Shell_SecondaryTrayWnd"
    )
}

unsafe fn window_class_name(hwnd: HWND) -> String {
    let mut buf = [0u16; 256];
    let len = GetClassNameW(hwnd, &mut buf) as usize;
    String::from_utf16_lossy(&buf[..len])
}

unsafe fn window_visible_rect(hwnd: HWND) -> RECT {
    let mut rect = RECT::default();
    if DwmGetWindowAttribute(
        hwnd,
        DWMWA_EXTENDED_FRAME_BOUNDS,
        &mut rect as *mut RECT as *mut c_void,
        size_of::<RECT>() as u32,
    )
    .is_ok()
    {
        return rect;
    }

    let _ = GetWindowRect(hwnd, &mut rect);
    rect
}

fn rect_from_points(a: POINT, b: POINT) -> RECT {
    RECT {
        left: a.x.min(b.x),
        top: a.y.min(b.y),
        right: a.x.max(b.x),
        bottom: a.y.max(b.y),
    }
}

fn normalize_rect(rect: RECT) -> RECT {
    RECT {
        left: rect.left.min(rect.right),
        top: rect.top.min(rect.bottom),
        right: rect.left.max(rect.right),
        bottom: rect.top.max(rect.bottom),
    }
}

fn clamp_rect(rect: RECT, bounds: RECT) -> RECT {
    let mut rect = normalize_rect(rect);
    rect.left = rect.left.clamp(bounds.left, bounds.right);
    rect.right = rect.right.clamp(bounds.left, bounds.right);
    rect.top = rect.top.clamp(bounds.top, bounds.bottom);
    rect.bottom = rect.bottom.clamp(bounds.top, bounds.bottom);
    normalize_rect(rect)
}

fn move_rect_clamped(rect: RECT, dx: i32, dy: i32, bounds: RECT) -> RECT {
    let rect = normalize_rect(rect);
    let width = rect.right - rect.left;
    let height = rect.bottom - rect.top;
    let mut left = rect.left + dx;
    let mut top = rect.top + dy;
    left = left.clamp(bounds.left, bounds.right - width);
    top = top.clamp(bounds.top, bounds.bottom - height);
    RECT {
        left,
        top,
        right: left + width,
        bottom: top + height,
    }
}

fn resize_rect_clamped(original: RECT, edges: Edges, pt: POINT, bounds: RECT) -> RECT {
    let mut rect = original;
    if edges.left {
        rect.left = pt.x.clamp(bounds.left, rect.right - MIN_SIZE);
    }
    if edges.right {
        rect.right = pt.x.clamp(rect.left + MIN_SIZE, bounds.right);
    }
    if edges.top {
        rect.top = pt.y.clamp(bounds.top, rect.bottom - MIN_SIZE);
    }
    if edges.bottom {
        rect.bottom = pt.y.clamp(rect.top + MIN_SIZE, bounds.bottom);
    }
    normalize_rect(rect)
}

fn hit_test_resize(rect: RECT, pt: POINT) -> Option<Edges> {
    let rect = normalize_rect(rect);
    let near_left = (pt.x - rect.left).abs() <= HIT_PAD;
    let near_right = (pt.x - rect.right).abs() <= HIT_PAD;
    let near_top = (pt.y - rect.top).abs() <= HIT_PAD;
    let near_bottom = (pt.y - rect.bottom).abs() <= HIT_PAD;
    let within_x = pt.x >= rect.left - HIT_PAD && pt.x <= rect.right + HIT_PAD;
    let within_y = pt.y >= rect.top - HIT_PAD && pt.y <= rect.bottom + HIT_PAD;

    let edges = Edges {
        left: near_left && within_y,
        top: near_top && within_x,
        right: near_right && within_y,
        bottom: near_bottom && within_x,
    };

    if edges.left || edges.top || edges.right || edges.bottom {
        Some(edges)
    } else {
        None
    }
}

fn point_in_rect(pt: POINT, rect: RECT) -> bool {
    pt.x >= rect.left && pt.x <= rect.right && pt.y >= rect.top && pt.y <= rect.bottom
}

fn distance_exceeds_threshold(a: POINT, b: POINT) -> bool {
    (a.x - b.x).abs() > 3 || (a.y - b.y).abs() > 3
}

fn handle_rects(rect: RECT) -> [RECT; 8] {
    let cx = (rect.left + rect.right) / 2;
    let cy = (rect.top + rect.bottom) / 2;
    [
        handle_at(rect.left, rect.top),
        handle_at(cx, rect.top),
        handle_at(rect.right, rect.top),
        handle_at(rect.right, cy),
        handle_at(rect.right, rect.bottom),
        handle_at(cx, rect.bottom),
        handle_at(rect.left, rect.bottom),
        handle_at(rect.left, cy),
    ]
}

fn handle_at(x: i32, y: i32) -> RECT {
    let half = HANDLE_SIZE / 2;
    RECT {
        left: x - half,
        top: y - half,
        right: x + half,
        bottom: y + half,
    }
}
