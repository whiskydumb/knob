pub(crate) mod layout;
pub(crate) mod tray;

use std::ffi::c_void;

use anyhow::{Context, Result};
use windows::{
	Win32::{
		Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM},
		Graphics::Gdi::{COLOR_BTNFACE, HBRUSH},
		System::LibraryLoader::GetModuleHandleW,
		UI::{
			HiDpi::{AdjustWindowRectExForDpi, GetDpiForWindow},
			WindowsAndMessaging::{
				CW_USEDEFAULT, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW,
				GWLP_USERDATA, GetMessageW, GetSystemMetrics, GetWindowLongPtrW, HCURSOR, HICON,
				HMENU, IDC_ARROW, IMAGE_ICON, IsDialogMessageW, KillTimer, LR_DEFAULTSIZE,
				LR_SHARED, LoadCursorW, LoadImageW, MSG, PostQuitMessage, RegisterClassExW,
				SM_CXSMICON, SW_SHOW, SWP_NOMOVE, SWP_NOZORDER, SetTimer, SetWindowLongPtrW,
				SetWindowPos, ShowWindow, TranslateMessage, WINDOW_EX_STYLE, WINDOW_STYLE,
				WM_APP, WM_CLOSE, WM_COMMAND, WM_DESTROY, WM_DPICHANGED, WM_HSCROLL,
				WM_LBUTTONDBLCLK, WM_LBUTTONUP, WM_NCDESTROY, WM_RBUTTONUP, WM_TIMER,
				WNDCLASSEXW, WS_CAPTION, WS_MINIMIZEBOX, WS_OVERLAPPED, WS_SYSMENU,
			},
		},
	},
	core::{PCWSTR, w},
};

use crate::{
	app::{App, TICK_MS},
	hook::{self, Command, WM_KNOB_COMMAND},
	media::{Report, WM_KNOB_MEDIA},
	ui::{
		layout::{CLIENT_HEIGHT, CLIENT_WIDTH},
		tray::{Tray, WM_TRAY, tray_event},
	},
	win::Discard,
};

/// window class name, also used to find an already running instance.
pub(crate) const CLASS_NAME: PCWSTR = w!("knob.window");

/// sent by a second instance to bring the first one's window up.
pub(crate) const WM_KNOB_SHOW: u32 = WM_APP + 3;

const TIMER_ID: usize = 1;

/// resource id of the icon group, matching `ICON_ID` in the build script.
const ICON_ID: u16 = 1;

/// loads the embedded icon at the given square size, or at the system default
/// when `size` is zero.
pub(crate) fn app_icon(size: i32) -> Option<HICON> {
	let instance = unsafe { GetModuleHandleW(None) }.ok()?;

	let handle = unsafe {
		LoadImageW(
			Some(instance.into()),
			PCWSTR(ICON_ID as *const u16),
			IMAGE_ICON,
			size,
			size,
			LR_DEFAULTSIZE | LR_SHARED,
		)
	}
	.ok()?;

	Some(HICON(handle.0))
}

/// the size windows expects for title bar and notification area icons.
pub(crate) fn small_icon() -> i32 { unsafe { GetSystemMetrics(SM_CXSMICON) } }

/// used both at creation and when sizing the frame.
const STYLE: u32 = WS_OVERLAPPED.0 | WS_CAPTION.0 | WS_SYSMENU.0 | WS_MINIMIZEBOX.0;

/// `hidden` comes from the autostart entry, which starts in the tray.
pub(crate) fn run(hidden: bool) -> Result<()> {
	let instance =
		{ unsafe { GetModuleHandleW(None) }.context("failed to get the module handle")? };

	let cursor = unsafe { LoadCursorW(None, IDC_ARROW) }.unwrap_or_default();

	// the large icon shows in alt-tab, the small one in the title bar
	let class = WNDCLASSEXW {
		cbSize: size_of::<WNDCLASSEXW>() as u32,
		lpfnWndProc: Some(window_proc),
		hInstance: instance.into(),
		lpszClassName: CLASS_NAME,
		hCursor: HCURSOR(cursor.0),
		hIcon: app_icon(0).unwrap_or_default(),
		hIconSm: app_icon(small_icon()).unwrap_or_default(),
		hbrBackground: HBRUSH((COLOR_BTNFACE.0 + 1) as isize as *mut c_void),
		..Default::default()
	};

	if unsafe { RegisterClassExW(&raw const class) } == 0 {
		anyhow::bail!("failed to register the window class");
	}

	let window = unsafe {
		CreateWindowExW(
			WINDOW_EX_STYLE(0),
			CLASS_NAME,
			w!("knob"),
			WINDOW_STYLE(STYLE),
			CW_USEDEFAULT,
			CW_USEDEFAULT,
			CLIENT_WIDTH,
			CLIENT_HEIGHT,
			None,
			HMENU::default().into(),
			Some(instance.into()),
			None,
		)
	}
	.context("failed to create the main window")?;

	fit_to_content(window);

	let app = Box::new(App::new(window)?);

	unsafe { SetWindowLongPtrW(window, GWLP_USERDATA, Box::into_raw(app) as isize) };

	hook::set_window(window);

	unsafe { SetTimer(Some(window), TIMER_ID, TICK_MS, None) }.discard();

	if !hidden {
		unsafe { ShowWindow(window, SW_SHOW) }.discard();
	}

	pump(window);

	Ok(())
}

fn pump(window: HWND) {
	let mut message = MSG::default();

	loop {
		let result = unsafe { GetMessageW(&raw mut message, None, 0, 0) };
		if result.0 <= 0 {
			break;
		}

		// dialog-style keyboard navigation between the controls
		let handled = unsafe { IsDialogMessageW(window, &raw const message) }.as_bool();
		if !handled {
			unsafe {
				TranslateMessage(&raw const message).discard();
				DispatchMessageW(&raw const message).discard();
			}
		}
	}
}

/// sizes the frame so the client area matches the layout at the window's dpi.
fn fit_to_content(window: HWND) {
	let dpi = unsafe { GetDpiForWindow(window) };
	let scale = |value: i32| value * dpi as i32 / 96;

	let mut rect = RECT {
		left: 0,
		top: 0,
		right: scale(CLIENT_WIDTH),
		bottom: scale(CLIENT_HEIGHT),
	};

	unsafe {
		AdjustWindowRectExForDpi(
			&raw mut rect,
			WINDOW_STYLE(STYLE),
			false,
			WINDOW_EX_STYLE(0),
			dpi,
		)
		.discard();

		SetWindowPos(
			window,
			None,
			0,
			0,
			rect.right - rect.left,
			rect.bottom - rect.top,
			SWP_NOMOVE | SWP_NOZORDER,
		)
		.discard();
	}
}

/// `None` before `App` is attached, which is the case for the messages windows
/// sends during creation.
fn app_of(window: HWND) -> Option<&'static mut App> {
	let pointer = unsafe { GetWindowLongPtrW(window, GWLP_USERDATA) } as *mut App;

	// SAFETY: the box stays alive until WM_NCDESTROY frees it, and the ui is
	// single threaded, so no second borrow can exist while this one is in use
	unsafe { pointer.as_mut() }
}

fn on_tray_event(window: HWND, app: &mut App, event: u32) {
	match event {
		| WM_LBUTTONDBLCLK | WM_LBUTTONUP => app.show(),
		| WM_RBUTTONUP => {
			let Some(choice) = Tray::show_menu(window, app.is_suspended()) else { return };

			if !app.on_tray(choice) {
				unsafe { DestroyWindow(window) }.discard();
			}
		},
		| _ => {},
	}
}

unsafe extern "system" fn window_proc(
	window: HWND,
	message: u32,
	wparam: WPARAM,
	lparam: LPARAM,
) -> LRESULT {
	match message {
		| WM_KNOB_COMMAND => {
			if let (Some(app), Some(command)) = (app_of(window), Command::from_code(wparam.0)) {
				app.on_knob(command);
			}

			return LRESULT(0);
		},
		| WM_KNOB_MEDIA => {
			if let (Some(app), Some(report)) = (app_of(window), Report::from_code(wparam.0)) {
				app.on_media(report);
			}

			return LRESULT(0);
		},
		| WM_KNOB_SHOW => {
			if let Some(app) = app_of(window) {
				app.show();
			}

			return LRESULT(0);
		},
		| WM_TIMER => {
			if let Some(app) = app_of(window) {
				app.tick();
			}

			return LRESULT(0);
		},
		| WM_COMMAND => {
			if let Some(app) = app_of(window) {
				let control = (wparam.0 & 0xFFFF) as i32;
				let notification = ((wparam.0 >> 16) & 0xFFFF) as u16;
				app.on_control(control, notification);
			}

			return LRESULT(0);
		},
		| WM_HSCROLL => {
			if let Some(app) = app_of(window) {
				app.on_slider();
			}

			return LRESULT(0);
		},
		| WM_TRAY => {
			if let Some(app) = app_of(window) {
				on_tray_event(window, app, tray_event(lparam));
			}

			return LRESULT(0);
		},
		| WM_DPICHANGED => {
			// SAFETY: for this message lparam points at the frame rect windows
			// suggests for the new dpi
			let suggested = unsafe { &*(lparam.0 as *const RECT) };

			unsafe {
				SetWindowPos(
					window,
					None,
					suggested.left,
					suggested.top,
					suggested.right - suggested.left,
					suggested.bottom - suggested.top,
					SWP_NOZORDER,
				)
			}
			.discard();

			if let Some(app) = app_of(window) {
				app.rescale();
			}

			return LRESULT(0);
		},
		| WM_CLOSE => {
			// closing keeps the program alive in the tray, exiting is a tray action
			if let Some(app) = app_of(window) {
				app.hide();
			}

			return LRESULT(0);
		},
		| WM_DESTROY => {
			unsafe { KillTimer(Some(window), TIMER_ID) }.discard();
			unsafe { PostQuitMessage(0) };

			return LRESULT(0);
		},
		| WM_NCDESTROY => {
			let pointer = unsafe { SetWindowLongPtrW(window, GWLP_USERDATA, 0) } as *mut App;
			if !pointer.is_null() {
				// SAFETY: the pointer came from Box::into_raw in `run` and the slot
				// was cleared above, so this drop happens exactly once
				drop(unsafe { Box::from_raw(pointer) });
			}
		},
		| _ => {},
	}

	unsafe { DefWindowProcW(window, message, wparam, lparam) }
}
