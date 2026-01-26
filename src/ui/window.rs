use crate::mcp::log_debug;
use crate::ui::session::{ContinueCallback, SessionAction};
use crate::utils::project_paths::{detect_project_root, get_ace_dir};
use serde::{Deserialize, Serialize};
use std::ffi::OsStr;
use std::fs;
use std::os::windows::ffi::OsStrExt;
use std::ptr::null_mut;
use std::sync::mpsc::{channel, Sender};
use std::thread;
use std::time::{Duration, Instant};
use windows::core::PCWSTR;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM, RECT, COLORREF, GetLastError};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::WC_EDITW;
use windows::Win32::UI::Input::KeyboardAndMouse::EnableWindow;
use windows::Win32::UI::WindowsAndMessaging::{
  CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW, LoadCursorW, PostQuitMessage,
  RegisterClassW, SetTimer, TranslateMessage, CREATESTRUCTW, CS_HREDRAW, CS_VREDRAW,
  CW_USEDEFAULT, HMENU, MSG, WM_COMMAND, WM_CREATE, WM_DESTROY, WM_TIMER, WNDCLASSW,
  WS_CHILD, WS_OVERLAPPEDWINDOW, WS_VISIBLE, WS_VSCROLL, WS_EX_CLIENTEDGE, BN_CLICKED,
  GetWindowLongPtrW, SetWindowLongPtrW, GWLP_USERDATA, GetWindowTextLengthW, GetWindowTextW,
  ES_AUTOVSCROLL, ES_MULTILINE, WINDOW_STYLE, SetWindowPos, SetWindowTextW, HWND_TOPMOST,
  HWND_NOTOPMOST, SWP_NOMOVE, SWP_NOSIZE, SendMessageW, WM_SETFONT, WM_CTLCOLORDLG,
  WM_CTLCOLOREDIT, WM_CTLCOLORSTATIC, WM_CTLCOLORBTN, BS_FLAT, WM_ERASEBKGND, GetClientRect,
  PostMessageW, WM_APP, ShowWindow, SetForegroundWindow, MessageBoxW, MB_OK, SW_SHOWNORMAL,
};
use windows::Win32::Graphics::Gdi::{
  CreateFontW, CreateSolidBrush, DeleteObject, FillRect, SetBkColor, SetBkMode, SetTextColor,
  HBRUSH, HDC, HFONT, TRANSPARENT, CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET, DEFAULT_QUALITY,
  FF_DONTCARE, FW_NORMAL, OUT_DEFAULT_PRECIS,
};
const ID_BTN_SEND: usize = 1001;
const ID_BTN_ORIGINAL: usize = 1002;
const ID_BTN_CONTINUE: usize = 1003;
const ID_BTN_END: usize = 1004;
const ID_BTN_PIN: usize = 1005;
const ID_EDIT_PROMPT: usize = 2001;
const ID_TIMER_TIMEOUT: usize = 3001;
const ID_TIMER_COUNTDOWN: usize = 3002;
const WM_APP_ENHANCE_SUCCESS: u32 = WM_APP + 1;
const WM_APP_ENHANCE_ERROR: u32 = WM_APP + 2;

struct CreateParams {
  sender: Sender<SessionAction>,
  prompt: String,
  timeout_ms: u32,
  continue_cb: ContinueCallback,
}

struct WindowState {
  sender: Sender<SessionAction>,
  edit: HWND,
  pin_button: HWND,
  countdown_label: HWND,
  pinned: bool,
  continue_cb: ContinueCallback,
  btn_end: HWND,
  btn_continue: HWND,
  btn_original: HWND,
  btn_send: HWND,
  is_enhancing: bool,
  start_at: Instant,
  timeout_ms: u32,
  bg_brush: HBRUSH,
  edit_brush: HBRUSH,
  font: HFONT,
}

#[derive(Serialize, Deserialize)]
struct PinState {
  pinned: bool,
}

/// 启动 Win32 提示词确认窗口，阻塞直到用户选择或超时。
pub fn run_prompt_window(
  prompt: &str,
  timeout: Duration,
  continue_cb: ContinueCallback,
) -> SessionAction {
  let (sender, receiver) = channel();
  let timeout_ms = timeout.as_millis().min(u32::MAX as u128) as u32;

  unsafe {
    let hinstance = GetModuleHandleW(None).unwrap();
    let class_name = to_wstring("AceToolPromptWindow");
    let wnd_class = WNDCLASSW {
      style: CS_HREDRAW | CS_VREDRAW,
      lpfnWndProc: Some(wnd_proc),
      hInstance: hinstance.into(),
      lpszClassName: PCWSTR(class_name.as_ptr()),
      hCursor: LoadCursorW(None, windows::Win32::UI::WindowsAndMessaging::IDC_ARROW).unwrap(),
      ..Default::default()
    };

    let atom = RegisterClassW(&wnd_class);
    if atom == 0 {
      log_debug(format!("ui: register class failed err={} ", unsafe { GetLastError().0 }));
    } else {
      log_debug("ui: register class ok".to_string());
    }

    let params = Box::new(CreateParams {
      sender: sender.clone(),
      prompt: prompt.to_string(),
      timeout_ms,
      continue_cb,
    });

    let window = windows::Win32::UI::WindowsAndMessaging::CreateWindowExW(
      Default::default(),
      PCWSTR(class_name.as_ptr()),
      PCWSTR(to_wstring("ace-tool 提示词增强").as_ptr()),
      WS_OVERLAPPEDWINDOW | WS_VISIBLE,
      CW_USEDEFAULT,
      CW_USEDEFAULT,
      900,
      600,
      None,
      None,
      hinstance,
      Some(Box::into_raw(params) as *const _),
    )
    .unwrap_or(HWND(null_mut()));

    if window.0.is_null() {
      log_debug(format!("ui: create window failed err={}", unsafe { GetLastError().0 }));
      return SessionAction::Timeout;
    }

    log_debug(format!("ui: window created hwnd={:?}", window));
    let _ = unsafe { ShowWindow(window, SW_SHOWNORMAL) };
    let _ = unsafe { SetForegroundWindow(window) };

    let mut msg = MSG::default();
    while GetMessageW(&mut msg, HWND(null_mut()), 0, 0).into() {
      let _ = TranslateMessage(&msg);
      DispatchMessageW(&msg);
    }
  }

  receiver.recv().unwrap_or(SessionAction::Timeout)
}

/// 窗口过程：处理创建、按钮点击、定时器与绘制等消息。
unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
  match msg {
    WM_CREATE => {
      let createstruct = unsafe { &*(lparam.0 as *const CREATESTRUCTW) };
      let params = unsafe { Box::from_raw(createstruct.lpCreateParams as *mut CreateParams) };

      let pinned = load_pin_state();
      let pin_label = if pinned { "📌" } else { "📍" };
      let pin_button = create_button(hwnd, pin_label, 820, 16, 44, 32, ID_BTN_PIN, true);
      let countdown_text = format!("剩余时间 {}", format_remaining(params.timeout_ms));
      let countdown_label = create_label(hwnd, &countdown_text, 20, 24, 200, 24);

      let bg_brush = unsafe { CreateSolidBrush(rgb(245, 246, 248)) };
      let edit_brush = unsafe { CreateSolidBrush(rgb(255, 255, 255)) };
      let font = unsafe {
        CreateFontW(
          -16,
          0,
          0,
          0,
          FW_NORMAL.0 as i32,
          0,
          0,
          0,
          DEFAULT_CHARSET.0 as u32,
          OUT_DEFAULT_PRECIS.0 as u32,
          CLIP_DEFAULT_PRECIS.0 as u32,
          DEFAULT_QUALITY.0 as u32,
          FF_DONTCARE.0 as u32,
          PCWSTR(to_wstring("Microsoft YaHei UI").as_ptr()),
        )
      };

      let edit_style = WINDOW_STYLE(
        WS_CHILD.0
          | WS_VISIBLE.0
          | WS_VSCROLL.0
          | (ES_MULTILINE as u32)
          | (ES_AUTOVSCROLL as u32),
      );
      let edit = unsafe {
        CreateWindowExW(
          WS_EX_CLIENTEDGE,
          WC_EDITW,
          PCWSTR(to_wstring(&params.prompt).as_ptr()),
          edit_style,
          20,
          60,
          840,
          400,
          hwnd,
          HMENU(ID_EDIT_PROMPT as usize as *mut core::ffi::c_void),
          None,
          None,
        )
      }
      .unwrap_or(HWND(null_mut()));

      let btn_end = create_button(hwnd, "结束对话", 20, 480, 160, 32, ID_BTN_END, false);
      let btn_continue = create_button(hwnd, "继续增强", 200, 480, 160, 32, ID_BTN_CONTINUE, false);
      let btn_original = create_button(hwnd, "使用原始", 380, 480, 160, 32, ID_BTN_ORIGINAL, false);
      let btn_send = create_button(hwnd, "发送增强", 600, 480, 160, 32, ID_BTN_SEND, false);

      set_control_font(edit, font);
      set_control_font(pin_button, font);
      set_control_font(countdown_label, font);
      set_control_font(btn_end, font);
      set_control_font(btn_continue, font);
      set_control_font(btn_original, font);
      set_control_font(btn_send, font);

      let state = Box::new(WindowState {
        sender: params.sender,
        edit,
        pin_button,
        countdown_label,
        pinned,
        continue_cb: params.continue_cb,
        btn_end,
        btn_continue,
        btn_original,
        btn_send,
        is_enhancing: false,
        start_at: Instant::now(),
        timeout_ms: params.timeout_ms,
        bg_brush,
        edit_brush,
        font,
      });

      unsafe {
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(state) as isize);
        SetTimer(hwnd, ID_TIMER_TIMEOUT, params.timeout_ms, None);
        SetTimer(hwnd, ID_TIMER_COUNTDOWN, 1000, None);
        if pinned {
          let _ = SetWindowPos(hwnd, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE);
        }
      }

      LRESULT(0)
    }
    WM_COMMAND => {
      if hiword(wparam.0 as u32) == (BN_CLICKED as u16) {
        let id = loword(wparam.0 as u32) as usize;
        let state = unsafe { &mut *(GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut WindowState) };
        if id == ID_BTN_PIN {
          state.pinned = !state.pinned;
          save_pin_state(state.pinned);
          let label = if state.pinned { "📌" } else { "📍" };
          let _ = unsafe { SetWindowTextW(state.pin_button, PCWSTR(to_wstring(label).as_ptr())) };
          let _ = unsafe {
            SetWindowPos(
              hwnd,
              if state.pinned { HWND_TOPMOST } else { HWND_NOTOPMOST },
              0,
              0,
              0,
              0,
              SWP_NOMOVE | SWP_NOSIZE,
            )
          };
          return LRESULT(0);
        }

        if id == ID_BTN_CONTINUE {
          log_debug("ui: click continue".to_string());
          if state.is_enhancing {
            return LRESULT(0);
          }

          state.is_enhancing = true;
          set_loading(state, true);

          let cb = state.continue_cb.clone();
          let hwnd_value = hwnd.0 as isize;
          let current = read_edit_text(state.edit);
          thread::spawn(move || {
            let hwnd_copy = HWND(hwnd_value as *mut core::ffi::c_void);
            let result = cb(current);
            unsafe {
              match result {
                Ok(text) => {
                  let ptr = Box::into_raw(Box::new(text));
                  let _ = PostMessageW(hwnd_copy, WM_APP_ENHANCE_SUCCESS, WPARAM(0), LPARAM(ptr as isize));
                }
                Err(err) => {
                  let ptr = Box::into_raw(Box::new(err));
                  let _ = PostMessageW(hwnd_copy, WM_APP_ENHANCE_ERROR, WPARAM(0), LPARAM(ptr as isize));
                }
              }
            }
          });
          return LRESULT(0);
        }

        let content = read_edit_text(state.edit);
        let action = match id {
          ID_BTN_SEND => {
            log_debug("ui: click send enhanced".to_string());
            SessionAction::UseEnhanced(content)
          },
          ID_BTN_ORIGINAL => {
            log_debug("ui: click use original".to_string());
            SessionAction::UseOriginal
          },
          ID_BTN_END => {
            log_debug("ui: click end conversation".to_string());
            SessionAction::EndConversation
          },
          _ => SessionAction::Timeout,
        };
        let _ = state.sender.send(action);
        unsafe { let _ = DestroyWindow(hwnd); };
        return LRESULT(0);
      }
      LRESULT(0)
    }
    WM_TIMER => {
      let timer_id = wparam.0 as usize;
      if timer_id == ID_TIMER_TIMEOUT {
        let state = unsafe { &mut *(GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut WindowState) };
        let _ = state.sender.send(SessionAction::Timeout);
        unsafe { let _ = DestroyWindow(hwnd); };
      } else if timer_id == ID_TIMER_COUNTDOWN {
        let state = unsafe { &mut *(GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut WindowState) };
        let elapsed = state.start_at.elapsed().as_millis() as u64;
        let remaining = state.timeout_ms.saturating_sub(elapsed as u32);
        let text = format!("剩余时间 {}", format_remaining(remaining));
        let _ = unsafe { SetWindowTextW(state.countdown_label, PCWSTR(to_wstring(&text).as_ptr())) };
      }
      LRESULT(0)
    }
    WM_APP_ENHANCE_SUCCESS => {
      let state = unsafe { &mut *(GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut WindowState) };
      let text = unsafe { Box::from_raw(lparam.0 as *mut String) };
      let _ = unsafe { SetWindowTextW(state.edit, PCWSTR(to_wstring(&text).as_ptr())) };
      state.is_enhancing = false;
      set_loading(state, false);
      LRESULT(0)
    }
    WM_APP_ENHANCE_ERROR => {
      let state = unsafe { &mut *(GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut WindowState) };
      let err = unsafe { Box::from_raw(lparam.0 as *mut String) };
      state.is_enhancing = false;
      set_loading(state, false);
      let _ = unsafe {
        MessageBoxW(
          hwnd,
          PCWSTR(to_wstring(&format!("增强失败，请重试：{}", err)).as_ptr()),
          PCWSTR(to_wstring("提示").as_ptr()),
          MB_OK,
        )
      };
      LRESULT(0)
    }
    WM_ERASEBKGND => {
      let state = unsafe { &mut *(GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut WindowState) };
      let hdc = HDC(wparam.0 as *mut core::ffi::c_void);
      let mut rect = RECT::default();
      let _ = unsafe { GetClientRect(hwnd, &mut rect) };
      unsafe { FillRect(hdc, &rect, state.bg_brush) };
      LRESULT(1)
    }
    WM_CTLCOLORDLG | WM_CTLCOLORSTATIC | WM_CTLCOLORBTN => {
      let state = unsafe { &mut *(GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut WindowState) };
      let hdc = HDC(wparam.0 as *mut core::ffi::c_void);
      unsafe {
        SetBkMode(hdc, TRANSPARENT);
        SetTextColor(hdc, rgb(48, 48, 48));
        SetBkColor(hdc, rgb(245, 246, 248));
      }
      LRESULT(state.bg_brush.0 as isize)
    }
    WM_CTLCOLOREDIT => {
      let state = unsafe { &mut *(GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut WindowState) };
      let hdc = HDC(wparam.0 as *mut core::ffi::c_void);
      unsafe {
        SetBkMode(hdc, TRANSPARENT);
        SetTextColor(hdc, rgb(32, 32, 32));
        SetBkColor(hdc, rgb(255, 255, 255));
      }
      LRESULT(state.edit_brush.0 as isize)
    }
    WM_DESTROY => {
      let ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut WindowState };
      if !ptr.is_null() {
        let state = unsafe { Box::from_raw(ptr) };
        unsafe {
          let _ = DeleteObject(state.bg_brush);
          let _ = DeleteObject(state.edit_brush);
          let _ = DeleteObject(state.font);
        }
      }
      unsafe { PostQuitMessage(0) };
      LRESULT(0)
    }
    _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
  }
}

/// 创建按钮控件。
fn create_button(hwnd: HWND, text: &str, x: i32, y: i32, width: i32, height: i32, id: usize, flat: bool) -> HWND {
  unsafe {
    let mut style = WS_CHILD | WS_VISIBLE;
    if flat {
      style = WINDOW_STYLE(style.0 | BS_FLAT as u32);
    }
    windows::Win32::UI::WindowsAndMessaging::CreateWindowExW(
      Default::default(),
      PCWSTR(to_wstring("BUTTON").as_ptr()),
      PCWSTR(to_wstring(text).as_ptr()),
      style,
      x,
      y,
      width,
      height,
      hwnd,
      HMENU(id as usize as *mut core::ffi::c_void),
      None,
      None,
    )
    .unwrap_or(HWND(null_mut()))
  }
}

/// 创建静态文本控件。
fn create_label(hwnd: HWND, text: &str, x: i32, y: i32, width: i32, height: i32) -> HWND {
  unsafe {
    windows::Win32::UI::WindowsAndMessaging::CreateWindowExW(
      Default::default(),
      PCWSTR(to_wstring("STATIC").as_ptr()),
      PCWSTR(to_wstring(text).as_ptr()),
      WS_CHILD | WS_VISIBLE,
      x,
      y,
      width,
      height,
      hwnd,
      HMENU(null_mut()),
      None,
      None,
    )
    .unwrap_or(HWND(null_mut()))
  }
}

/// 读取编辑框内容。
fn read_edit_text(hwnd: HWND) -> String {
  unsafe {
    let length = GetWindowTextLengthW(hwnd);
    if length == 0 {
      return String::new();
    }
    let mut buffer = vec![0u16; (length + 1) as usize];
    let read = GetWindowTextW(hwnd, &mut buffer);
    if read == 0 {
      return String::new();
    }
    String::from_utf16_lossy(&buffer[..read as usize])
  }
}

/// 将 Rust 字符串转换为 Win32 宽字符串（以 0 结尾）。
fn to_wstring(input: &str) -> Vec<u16> {
  OsStr::new(input).encode_wide().chain(std::iter::once(0)).collect()
}

fn loword(value: u32) -> u16 {
  (value & 0xFFFF) as u16
}

fn hiword(value: u32) -> u16 {
  ((value >> 16) & 0xFFFF) as u16
}

fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
  COLORREF((r as u32) | ((g as u32) << 8) | ((b as u32) << 16))
}

/// 设置控件字体。
fn set_control_font(hwnd: HWND, font: HFONT) {
  unsafe {
    let _ = SendMessageW(hwnd, WM_SETFONT, WPARAM(font.0 as usize), LPARAM(1));
  }
}

/// 将剩余毫秒数格式化为 mm:ss。
fn format_remaining(ms: u32) -> String {
  let total = ms / 1000;
  let minutes = total / 60;
  let seconds = total % 60;
  format!("{:02}:{:02}", minutes, seconds)
}

/// 切换加载态，避免重复点击。
fn set_loading(state: &WindowState, loading: bool) {
  let label = if loading { "增强中..." } else { "继续增强" };
  unsafe {
    let _ = SetWindowTextW(state.btn_continue, PCWSTR(to_wstring(label).as_ptr()));
    let enable = !loading;
    let _ = EnableWindow(state.btn_continue, enable);
    let _ = EnableWindow(state.btn_send, enable);
    let _ = EnableWindow(state.btn_original, enable);
    let _ = EnableWindow(state.btn_end, enable);
    let _ = EnableWindow(state.pin_button, enable);
    let _ = EnableWindow(state.edit, enable);
  }
}

/// 固定窗口状态持久化路径。
fn pin_state_path() -> std::path::PathBuf {
  let project_root = detect_project_root();
  let ace_dir = get_ace_dir(&project_root);
  ace_dir.join("pin.json")
}

/// 读取固定窗口状态。
fn load_pin_state() -> bool {
  let path = pin_state_path();
  if !path.exists() {
    return false;
  }
  let content = fs::read_to_string(path).unwrap_or_default();
  serde_json::from_str::<PinState>(&content)
    .map(|state| state.pinned)
    .unwrap_or(false)
}

/// 保存固定窗口状态。
fn save_pin_state(pinned: bool) {
  let path = pin_state_path();
  if let Some(parent) = path.parent() {
    let _ = fs::create_dir_all(parent);
  }
  let content = serde_json::to_string_pretty(&PinState { pinned })
    .unwrap_or_else(|_| format!("{{\"pinned\":{}}}", pinned));
  let _ = fs::write(path, content);
}



