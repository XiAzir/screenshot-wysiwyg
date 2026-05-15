use crate::capture;
use crate::hotkey::{parse_hotkey, Hotkey};
use crate::output;
use crate::overlay;
use crate::settings::Settings;
use anyhow::{anyhow, Context, Result};
use std::mem::size_of;
use std::thread;
use std::time::Duration;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{
    ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, HINSTANCE, HWND, LPARAM, LRESULT, POINT, WPARAM,
};
use windows::Win32::Graphics::Gdi::{GetStockObject, COLOR_WINDOW, DEFAULT_GUI_FONT, HBRUSH};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyExW, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
    HKEY, HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE, KEY_WRITE, REG_OPTION_NON_VOLATILE,
    REG_SZ,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{RegisterHotKey, UnregisterHotKey};
use windows::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP, NIM_ADD, NIM_DELETE,
    NIN_SELECT, NOTIFYICONDATAW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, BringWindowToTop, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu,
    DestroyWindow, DispatchMessageW, GetCursorPos, GetDlgItem, GetDlgItemTextW, GetMessageW,
    GetWindowLongPtrW, IsWindowVisible, LoadCursorW, LoadIconW, LoadImageW, MessageBoxW,
    PostMessageW, PostQuitMessage, RegisterClassW, SendMessageW, SetForegroundWindow,
    SetWindowLongPtrW, SetWindowTextW, ShowWindow, TrackPopupMenu, TranslateMessage, BM_GETCHECK,
    BM_SETCHECK, BS_AUTOCHECKBOX, BS_DEFPUSHBUTTON, BS_PUSHBUTTON, CREATESTRUCTW, CS_HREDRAW,
    CS_VREDRAW, CW_USEDEFAULT, ES_AUTOHSCROLL, GWLP_USERDATA, HICON, HMENU, ICON_BIG, ICON_SMALL,
    IDC_ARROW, IDI_APPLICATION, IMAGE_ICON, LR_DEFAULTSIZE, MB_ICONERROR, MB_OK, MB_TOPMOST,
    MF_STRING, MSG, SIZE_MINIMIZED, SW_HIDE, SW_RESTORE, SW_SHOW, TPM_NONOTIFY, TPM_RETURNCMD,
    TPM_RIGHTBUTTON, WINDOW_EX_STYLE, WINDOW_STYLE, WM_APP, WM_CLOSE, WM_COMMAND, WM_CONTEXTMENU,
    WM_CREATE, WM_DESTROY, WM_HOTKEY, WM_LBUTTONDBLCLK, WM_NCCREATE, WM_NCDESTROY, WM_NULL,
    WM_RBUTTONUP, WM_SETFONT, WM_SETICON, WM_SIZE, WNDCLASSW, WS_BORDER, WS_CHILD, WS_EX_APPWINDOW,
    WS_OVERLAPPEDWINDOW, WS_TABSTOP, WS_VISIBLE,
};

const CLASS_NAME: PCWSTR = w!("KumokiriMainWindow");
const APP_NAME: PCWSTR = w!("Kumokiri");
const APP_ICON_ID: usize = 1;
const HOTKEY_ID: i32 = 101;
const TRAY_UID: u32 = 1;
const WM_TRAYICON: u32 = WM_APP + 1;
const AUTOSTART_NAME: &str = "Kumokiri";
const LEGACY_AUTOSTART_NAME: &str = "WysiwygScreenshot";

const IDC_SAVE_FILE: i32 = 1001;
const IDC_SAVE_CLIPBOARD: i32 = 1002;
const IDC_SAVE_DIR: i32 = 1003;
const IDC_HOTKEY: i32 = 1004;
const IDC_AUTOSTART: i32 = 1005;
const IDC_APPLY: i32 = 1006;
const IDC_CAPTURE: i32 = 1007;
const IDC_STATUS: i32 = 1008;

const ID_TRAY_EXIT: usize = 2001;

struct AppState {
    hwnd: HWND,
    settings: Settings,
    hotkey: Option<Hotkey>,
    tray_added: bool,
    allow_exit: bool,
    startup_hidden: bool,
}

pub fn run() -> Result<()> {
    let startup_hidden = std::env::args().any(|arg| arg == "--startup");
    let mut settings = Settings::load();
    settings.start_with_windows = autostart_enabled();

    let state = Box::new(AppState {
        hwnd: HWND::default(),
        settings,
        hotkey: None,
        tray_added: false,
        allow_exit: false,
        startup_hidden,
    });
    let state_ptr = Box::into_raw(state);

    let hmodule = unsafe { GetModuleHandleW(None) }.context("获取模块句柄失败")?;
    let hinstance: HINSTANCE = hmodule.into();
    let cursor = unsafe { LoadCursorW(None, IDC_ARROW) }.unwrap_or_default();
    let class_icon = unsafe { load_app_icon(hinstance, 0) };
    let wc = WNDCLASSW {
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: Some(main_wndproc),
        hInstance: hinstance,
        hIcon: class_icon,
        hCursor: cursor,
        hbrBackground: HBRUSH((COLOR_WINDOW.0 as usize + 1) as *mut _),
        lpszClassName: CLASS_NAME,
        ..Default::default()
    };
    unsafe {
        RegisterClassW(&wc);
    }

    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_APPWINDOW,
            CLASS_NAME,
            APP_NAME,
            WS_OVERLAPPEDWINDOW,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            560,
            330,
            None,
            None,
            hinstance,
            Some(state_ptr as *const _),
        )
    }
    .context("创建主窗口失败")?;

    unsafe {
        (*state_ptr).hwnd = hwnd;
        set_window_icons(hwnd, hinstance);
        if (*state_ptr).startup_hidden {
            (*state_ptr).hide_to_tray();
        } else {
            let _ = ShowWindow(hwnd, SW_SHOW);
        }
    }

    let mut msg = MSG::default();
    loop {
        let ret = unsafe { GetMessageW(&mut msg, HWND::default(), 0, 0) };
        if ret.0 <= 0 {
            break;
        }
        unsafe {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }

    Ok(())
}

pub fn show_error_message(message: &str) {
    let text = to_wide(message);
    unsafe {
        MessageBoxW(
            None,
            PCWSTR(text.as_ptr()),
            APP_NAME,
            MB_OK | MB_ICONERROR | MB_TOPMOST,
        );
    }
}

unsafe extern "system" fn main_wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == WM_NCCREATE {
        let create = lparam.0 as *const CREATESTRUCTW;
        let state = (*create).lpCreateParams as *mut AppState;
        (*state).hwnd = hwnd;
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, state as isize);
        return LRESULT(1);
    }

    let state_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut AppState;
    if state_ptr.is_null() {
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }
    let state = &mut *state_ptr;

    if msg == WM_TRAYICON {
        state.handle_tray_message(lparam);
        return LRESULT(0);
    }

    match msg {
        WM_CREATE => {
            if let Err(err) = state.create_controls() {
                state.show_error(&format!("{err:#}"));
            }
            LRESULT(0)
        }
        WM_COMMAND => {
            let id = (wparam.0 & 0xffff) as usize;
            match id {
                x if x == IDC_APPLY as usize => state.save_settings_from_controls(),
                x if x == IDC_CAPTURE as usize => state.start_capture(),
                ID_TRAY_EXIT => {
                    state.exit_app();
                }
                _ => {}
            }
            LRESULT(0)
        }
        WM_HOTKEY => {
            if wparam.0 as i32 == HOTKEY_ID {
                state.start_capture();
            }
            LRESULT(0)
        }
        WM_SIZE => {
            if wparam.0 as u32 == SIZE_MINIMIZED {
                state.hide_to_tray();
                return LRESULT(0);
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_CLOSE => {
            if state.allow_exit {
                let _ = DestroyWindow(hwnd);
            } else {
                state.hide_to_tray();
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            state.remove_tray_icon();
            let _ = UnregisterHotKey(hwnd, HOTKEY_ID);
            PostQuitMessage(0);
            LRESULT(0)
        }
        WM_NCDESTROY => {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            drop(Box::from_raw(state_ptr));
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

impl AppState {
    fn create_controls(&mut self) -> Result<()> {
        unsafe {
            create_label(self.hwnd, "截图保存", 24, 24, 120, 22)?;
            let save_file =
                create_checkbox(self.hwnd, IDC_SAVE_FILE, "保存到文件夹", 34, 54, 150, 24)?;
            set_checkbox(save_file, self.settings.save_to_file);
            let save_clipboard = create_checkbox(
                self.hwnd,
                IDC_SAVE_CLIPBOARD,
                "保存到剪贴板",
                190,
                54,
                150,
                24,
            )?;
            set_checkbox(save_clipboard, self.settings.save_to_clipboard);

            create_label(self.hwnd, "保存路径", 34, 92, 90, 22)?;
            create_edit(
                self.hwnd,
                IDC_SAVE_DIR,
                &self.settings.save_dir,
                120,
                88,
                390,
                26,
            )?;

            create_label(self.hwnd, "快捷键", 34, 132, 90, 22)?;
            create_edit(
                self.hwnd,
                IDC_HOTKEY,
                &self.settings.hotkey,
                120,
                128,
                180,
                26,
            )?;
            let autostart =
                create_checkbox(self.hwnd, IDC_AUTOSTART, "开机自启动", 320, 130, 140, 24)?;
            set_checkbox(autostart, self.settings.start_with_windows);

            create_button(self.hwnd, IDC_APPLY, "保存设置", 120, 178, 120, 32, false)?;
            create_button(self.hwnd, IDC_CAPTURE, "立即截图", 254, 178, 120, 32, true)?;

            create_label(self.hwnd, "状态", 34, 232, 70, 22)?;
            let status = create_label_with_id(self.hwnd, IDC_STATUS, "", 120, 232, 390, 44)?;
            set_window_text(status, "就绪");
        }

        self.register_current_hotkey();
        Ok(())
    }

    fn register_current_hotkey(&mut self) {
        match parse_hotkey(&self.settings.hotkey) {
            Ok(hotkey) => unsafe {
                let _ = UnregisterHotKey(self.hwnd, HOTKEY_ID);
                match RegisterHotKey(self.hwnd, HOTKEY_ID, hotkey.register_modifiers(), hotkey.vk) {
                    Ok(()) => {
                        self.hotkey = Some(hotkey.clone());
                        self.set_status(&format!("热键已启用: {}", hotkey.display));
                    }
                    Err(_) => {
                        self.hotkey = None;
                        self.set_status("热键注册失败，可能已被其他程序占用");
                    }
                }
            },
            Err(err) => {
                self.hotkey = None;
                self.set_status(&format!("热键无效: {err}"));
            }
        }
    }

    fn save_settings_from_controls(&mut self) {
        match self.read_settings_from_controls() {
            Ok(settings) => {
                if !settings.save_to_file && !settings.save_to_clipboard {
                    self.set_status("请至少勾选一种保存方式");
                    return;
                }
                if let Err(err) = set_autostart(settings.start_with_windows) {
                    self.show_error(&format!("{err:#}"));
                    return;
                }
                self.settings = settings;
                if let Err(err) = self.settings.save() {
                    self.show_error(&format!("{err:#}"));
                    return;
                }
                self.register_current_hotkey();
                self.set_status("设置已保存");
            }
            Err(err) => self.show_error(&format!("{err:#}")),
        }
    }

    fn read_settings_from_controls(&self) -> Result<Settings> {
        let save_to_file = unsafe { checkbox_checked(GetDlgItem(self.hwnd, IDC_SAVE_FILE)?) };
        let save_to_clipboard =
            unsafe { checkbox_checked(GetDlgItem(self.hwnd, IDC_SAVE_CLIPBOARD)?) };
        let start_with_windows = unsafe { checkbox_checked(GetDlgItem(self.hwnd, IDC_AUTOSTART)?) };
        let save_dir = get_control_text(self.hwnd, IDC_SAVE_DIR)?
            .trim()
            .to_string();
        let hotkey = get_control_text(self.hwnd, IDC_HOTKEY)?.trim().to_string();

        parse_hotkey(&hotkey).context("快捷键格式无效")?;

        Ok(Settings {
            save_to_file,
            save_to_clipboard,
            save_dir: if save_dir.is_empty() {
                crate::settings::default_save_dir()
            } else {
                save_dir
            },
            hotkey,
            start_with_windows,
        })
    }

    fn start_capture(&mut self) {
        if let Ok(settings) = self.read_settings_from_controls() {
            self.settings.save_to_file = settings.save_to_file;
            self.settings.save_to_clipboard = settings.save_to_clipboard;
            self.settings.save_dir = settings.save_dir;
        }

        if !self.settings.save_to_file && !self.settings.save_to_clipboard {
            self.set_status("请至少勾选一种保存方式");
            return;
        }

        let was_visible = unsafe { IsWindowVisible(self.hwnd).as_bool() };
        if was_visible {
            unsafe {
                let _ = ShowWindow(self.hwnd, SW_HIDE);
            }
            thread::sleep(Duration::from_millis(16));
        }

        let snapshot = capture::capture_desktop_snapshot();
        if let Err(err) = &snapshot {
            self.show_error(&format!("{err:#}"));
            if was_visible {
                self.restore_from_tray();
            }
            return;
        }
        let snapshot = snapshot.unwrap();

        self.set_status("冻结画面，选择截图区域");
        let selection = overlay::select_region(&snapshot);
        let selection = match selection {
            Ok(Some(rect)) => rect,
            Ok(None) => {
                if was_visible {
                    self.restore_from_tray();
                }
                self.set_status("截图已取消");
                return;
            }
            Err(err) => {
                self.show_error(&format!("{err:#}"));
                if was_visible {
                    self.restore_from_tray();
                }
                self.set_status("截图已取消");
                return;
            }
        };

        match capture::crop_snapshot(&snapshot, selection)
            .and_then(|image| self.output_captured_image(&image))
        {
            Ok(status) => self.set_status(&status),
            Err(err) => self.show_error(&format!("{err:#}")),
        }

        if was_visible {
            self.restore_from_tray();
        }
    }

    fn output_captured_image(&self, image: &capture::CapturedImage) -> Result<String> {
        let mut parts = Vec::new();

        if self.settings.save_to_file {
            let path = output::save_png_file(
                &image.data,
                image.width,
                image.height,
                &self.settings.save_dir,
            )?;
            parts.push(format!("已保存: {}", path.display()));
        }

        if self.settings.save_to_clipboard {
            output::copy_to_clipboard(self.hwnd, &image.data, image.width, image.height)?;
            parts.push("已复制到剪贴板".to_string());
        }

        if image.is_hdr_source {
            parts.push("HDR 已按显示效果映射".to_string());
        }

        Ok(parts.join("；"))
    }

    fn hide_to_tray(&mut self) {
        self.add_tray_icon();
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_HIDE);
        }
    }

    fn restore_from_tray(&mut self) {
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_SHOW);
            let _ = ShowWindow(self.hwnd, SW_RESTORE);
            let _ = BringWindowToTop(self.hwnd);
            let _ = SetForegroundWindow(self.hwnd);
        }
    }

    fn add_tray_icon(&mut self) {
        if self.tray_added {
            return;
        }

        let icon = unsafe {
            GetModuleHandleW(None)
                .map(|module| load_app_icon(module.into(), 16))
                .unwrap_or_else(|_| LoadIconW(None, IDI_APPLICATION).unwrap_or_default())
        };
        let mut nid = NOTIFYICONDATAW {
            cbSize: size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: self.hwnd,
            uID: TRAY_UID,
            uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_SHOWTIP,
            uCallbackMessage: WM_TRAYICON,
            hIcon: icon,
            ..Default::default()
        };
        copy_to_fixed_wide(&mut nid.szTip, "Kumokiri");

        unsafe {
            if Shell_NotifyIconW(NIM_ADD, &nid).as_bool() {
                self.tray_added = true;
            }
        }
    }

    fn remove_tray_icon(&mut self) {
        if !self.tray_added {
            return;
        }
        let nid = NOTIFYICONDATAW {
            cbSize: size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: self.hwnd,
            uID: TRAY_UID,
            ..Default::default()
        };
        unsafe {
            let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
        }
        self.tray_added = false;
    }

    fn handle_tray_message(&mut self, lparam: LPARAM) {
        let message = tray_event_code(lparam);
        match message {
            NIN_SELECT | WM_LBUTTONDBLCLK => self.restore_from_tray(),
            WM_CONTEXTMENU | WM_RBUTTONUP => self.show_tray_menu(),
            _ => {}
        }
    }

    fn show_tray_menu(&mut self) {
        unsafe {
            let Ok(menu) = CreatePopupMenu() else {
                return;
            };
            let _ = AppendMenuW(menu, MF_STRING, ID_TRAY_EXIT, w!("退出"));

            let mut point = POINT::default();
            if GetCursorPos(&mut point).is_ok() {
                let _ = SetForegroundWindow(self.hwnd);
                let command = TrackPopupMenu(
                    menu,
                    TPM_RIGHTBUTTON | TPM_RETURNCMD | TPM_NONOTIFY,
                    point.x,
                    point.y,
                    0,
                    self.hwnd,
                    None,
                )
                .0 as usize;
                let _ = PostMessageW(self.hwnd, WM_NULL, WPARAM(0), LPARAM(0));
                if command == ID_TRAY_EXIT {
                    self.exit_app();
                }
            }
            let _ = DestroyMenu(menu);
        }
    }

    fn exit_app(&mut self) {
        self.allow_exit = true;
        unsafe {
            let _ = DestroyWindow(self.hwnd);
        }
    }

    fn set_status(&self, text: &str) {
        if let Ok(hwnd) = unsafe { GetDlgItem(self.hwnd, IDC_STATUS) } {
            set_window_text(hwnd, text);
        }
    }

    fn show_error(&self, text: &str) {
        self.set_status(text);
        let wide = to_wide(text);
        unsafe {
            MessageBoxW(
                self.hwnd,
                PCWSTR(wide.as_ptr()),
                APP_NAME,
                MB_OK | MB_ICONERROR | MB_TOPMOST,
            );
        }
    }
}

unsafe fn create_label(
    parent: HWND,
    text: &str,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
) -> Result<HWND> {
    create_label_with_id(parent, 0, text, x, y, width, height)
}

unsafe fn create_label_with_id(
    parent: HWND,
    id: i32,
    text: &str,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
) -> Result<HWND> {
    let hwnd = create_control(
        parent,
        w!("STATIC"),
        text,
        WS_CHILD | WS_VISIBLE,
        id,
        x,
        y,
        width,
        height,
    )?;
    Ok(hwnd)
}

unsafe fn create_checkbox(
    parent: HWND,
    id: i32,
    text: &str,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
) -> Result<HWND> {
    create_control(
        parent,
        w!("BUTTON"),
        text,
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(BS_AUTOCHECKBOX as u32),
        id,
        x,
        y,
        width,
        height,
    )
}

unsafe fn create_edit(
    parent: HWND,
    id: i32,
    text: &str,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
) -> Result<HWND> {
    create_control(
        parent,
        w!("EDIT"),
        text,
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | WS_BORDER | WINDOW_STYLE(ES_AUTOHSCROLL as u32),
        id,
        x,
        y,
        width,
        height,
    )
}

unsafe fn create_button(
    parent: HWND,
    id: i32,
    text: &str,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    default_button: bool,
) -> Result<HWND> {
    let button_style = if default_button {
        BS_DEFPUSHBUTTON
    } else {
        BS_PUSHBUTTON
    };
    create_control(
        parent,
        w!("BUTTON"),
        text,
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(button_style as u32),
        id,
        x,
        y,
        width,
        height,
    )
}

unsafe fn create_control(
    parent: HWND,
    class_name: PCWSTR,
    text: &str,
    style: WINDOW_STYLE,
    id: i32,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
) -> Result<HWND> {
    let text = to_wide(text);
    let hwnd = CreateWindowExW(
        WINDOW_EX_STYLE::default(),
        class_name,
        PCWSTR(text.as_ptr()),
        style,
        x,
        y,
        width,
        height,
        parent,
        HMENU(id as usize as *mut _),
        None,
        None,
    )
    .context("创建控件失败")?;
    let font = GetStockObject(DEFAULT_GUI_FONT);
    SendMessageW(hwnd, WM_SETFONT, WPARAM(font.0 as usize), LPARAM(1));
    Ok(hwnd)
}

unsafe fn set_checkbox(hwnd: HWND, checked: bool) {
    SendMessageW(
        hwnd,
        BM_SETCHECK,
        WPARAM(if checked { 1 } else { 0 }),
        LPARAM(0),
    );
}

unsafe fn checkbox_checked(hwnd: HWND) -> bool {
    SendMessageW(hwnd, BM_GETCHECK, WPARAM(0), LPARAM(0)).0 == 1
}

fn get_control_text(parent: HWND, id: i32) -> Result<String> {
    let mut buffer = vec![0u16; 1024];
    let len = unsafe { GetDlgItemTextW(parent, id, &mut buffer) } as usize;
    if len >= buffer.len() {
        return Err(anyhow!("控件文本过长"));
    }
    Ok(String::from_utf16_lossy(&buffer[..len]))
}

fn set_window_text(hwnd: HWND, text: &str) {
    let text = to_wide(text);
    unsafe {
        let _ = SetWindowTextW(hwnd, PCWSTR(text.as_ptr()));
    }
}

fn to_wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

fn copy_to_fixed_wide(target: &mut [u16], text: &str) {
    let wide = to_wide(text);
    let count = target.len().min(wide.len());
    target[..count].copy_from_slice(&wide[..count]);
    if let Some(last) = target.last_mut() {
        *last = 0;
    }
}

fn tray_event_code(lparam: LPARAM) -> u32 {
    (lparam.0 as u32) & 0xffff
}

fn app_icon_resource() -> PCWSTR {
    PCWSTR(APP_ICON_ID as *const u16)
}

unsafe fn load_app_icon(hinstance: HINSTANCE, size: i32) -> HICON {
    let flags = if size == 0 {
        LR_DEFAULTSIZE
    } else {
        Default::default()
    };
    LoadImageW(
        hinstance,
        app_icon_resource(),
        IMAGE_ICON,
        size,
        size,
        flags,
    )
    .map(|handle| HICON(handle.0))
    .or_else(|_| LoadIconW(None, IDI_APPLICATION))
    .unwrap_or_default()
}

unsafe fn set_window_icons(hwnd: HWND, hinstance: HINSTANCE) {
    let big_icon = load_app_icon(hinstance, 32);
    let small_icon = load_app_icon(hinstance, 16);
    let _ = SendMessageW(
        hwnd,
        WM_SETICON,
        WPARAM(ICON_BIG as usize),
        LPARAM(big_icon.0 as isize),
    );
    let _ = SendMessageW(
        hwnd,
        WM_SETICON,
        WPARAM(ICON_SMALL as usize),
        LPARAM(small_icon.0 as isize),
    );
}

fn autostart_enabled() -> bool {
    let subkey = to_wide("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
    let mut key = HKEY::default();
    let opened = unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(subkey.as_ptr()),
            0,
            KEY_QUERY_VALUE,
            &mut key,
        )
    };
    if opened != ERROR_SUCCESS {
        return false;
    }
    let exists = registry_value_exists(key, AUTOSTART_NAME)
        || registry_value_exists(key, LEGACY_AUTOSTART_NAME);
    unsafe {
        let _ = RegCloseKey(key);
    }
    exists
}

fn set_autostart(enabled: bool) -> Result<()> {
    let subkey = to_wide("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
    let name = to_wide(AUTOSTART_NAME);
    let mut key = HKEY::default();
    let result = unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(subkey.as_ptr()),
            0,
            PCWSTR::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_WRITE | KEY_SET_VALUE,
            None,
            &mut key,
            None,
        )
    };
    if result != ERROR_SUCCESS {
        return Err(anyhow!("打开开机自启动注册表项失败: {}", result.0));
    }

    let op = if enabled {
        let exe = std::env::current_exe().context("获取当前 exe 路径失败")?;
        let command = format!("\"{}\" --startup", exe.display());
        let command = to_wide(&command);
        let bytes =
            unsafe { std::slice::from_raw_parts(command.as_ptr() as *const u8, command.len() * 2) };
        let result = unsafe { RegSetValueExW(key, PCWSTR(name.as_ptr()), 0, REG_SZ, Some(bytes)) };
        let _ = delete_registry_value(key, LEGACY_AUTOSTART_NAME);
        result
    } else {
        let deleted_current = delete_registry_value(key, AUTOSTART_NAME);
        let deleted_legacy = delete_registry_value(key, LEGACY_AUTOSTART_NAME);
        if deleted_current == ERROR_SUCCESS {
            deleted_current
        } else {
            deleted_legacy
        }
    };

    unsafe {
        let _ = RegCloseKey(key);
    }

    if op != ERROR_SUCCESS {
        return Err(anyhow!("更新开机自启动失败: {}", op.0));
    }

    Ok(())
}

fn registry_value_exists(key: HKEY, name: &str) -> bool {
    let name = to_wide(name);
    unsafe { RegQueryValueExW(key, PCWSTR(name.as_ptr()), None, None, None, None) == ERROR_SUCCESS }
}

fn delete_registry_value(key: HKEY, name: &str) -> windows::Win32::Foundation::WIN32_ERROR {
    let name = to_wide(name);
    let deleted = unsafe { RegDeleteValueW(key, PCWSTR(name.as_ptr())) };
    if deleted == ERROR_FILE_NOT_FOUND {
        ERROR_SUCCESS
    } else {
        deleted
    }
}
