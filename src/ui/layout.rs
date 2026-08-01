use std::ffi::c_void;

use anyhow::{Context, Result};
use windows::{
	Win32::{
		Foundation::{HWND, LPARAM, WPARAM},
		Graphics::Gdi::{CreateFontIndirectW, DeleteObject, HFONT},
		UI::{
			Controls::{
				ICC_BAR_CLASSES, ICC_STANDARD_CLASSES, INITCOMMONCONTROLSEX,
				InitCommonControlsEx, TBM_SETPAGESIZE, TBM_SETPOS, TBM_SETRANGE, TBM_SETTICFREQ,
				TBS_AUTOTICKS, TBS_HORZ,
			},
			HiDpi::{GetDpiForWindow, SystemParametersInfoForDpi},
			WindowsAndMessaging::{
				BM_GETCHECK, BM_SETCHECK, BS_AUTOCHECKBOX, BS_DEFPUSHBUTTON, BS_PUSHBUTTON,
				CB_ADDSTRING, CB_GETCURSEL, CB_RESETCONTENT, CB_SETCURSEL, CBS_AUTOHSCROLL,
				CBS_DROPDOWN, CBS_DROPDOWNLIST, CreateWindowExW, GetWindowTextW, HMENU,
				NONCLIENTMETRICSW, SPI_GETNONCLIENTMETRICS, SWP_NOACTIVATE, SWP_NOZORDER,
				SendMessageW, SetWindowPos, SetWindowTextW, WINDOW_EX_STYLE, WINDOW_STYLE,
				WM_SETFONT, WM_USER, WS_CHILD, WS_TABSTOP, WS_VISIBLE, WS_VSCROLL,
			},
		},
	},
	core::{PCWSTR, w},
};

use crate::win::{Discard, from_wide, wide};

/// control identifiers, arriving in the low word of `wparam` on `WM_COMMAND`.
pub(crate) mod id {
	pub(crate) const TARGET: i32 = 1001;
	pub(crate) const REFRESH: i32 = 1002;
	pub(crate) const RAISE: i32 = 1003;
	pub(crate) const LOWER: i32 = 1004;
	pub(crate) const STEP: i32 = 1005;
	pub(crate) const PRESS: i32 = 1006;
	pub(crate) const STARTUP: i32 = 1007;
	pub(crate) const SAVE: i32 = 1008;
}

/// client area at 96 dpi, in device independent pixels.
pub(crate) const CLIENT_WIDTH: i32 = 400;
pub(crate) const CLIENT_HEIGHT: i32 = 322;

/// for a combo box this is the height of the dropped-down list, not of the
/// field.
const LIST_HEIGHT: i32 = 220;

/// styles and messages the windows metadata does not expose.
const SS_LEFT: u32 = 0x0000_0000;
const SS_END_ELLIPSIS: u32 = 0x0000_4000;
const BST_IS_CHECKED: usize = 1;
const TBM_GET_POS: u32 = WM_USER;

pub(crate) struct Controls {
	pub(crate) target: HWND,
	pub(crate) refresh: HWND,
	pub(crate) raise: HWND,
	pub(crate) lower: HWND,
	pub(crate) step: HWND,
	pub(crate) press: HWND,
	pub(crate) startup: HWND,
	pub(crate) save: HWND,
	pub(crate) step_label: HWND,
	pub(crate) status: HWND,
	labels: Vec<HWND>,
	font: HFONT,
}

impl Controls {
	pub(crate) fn create(parent: HWND) -> Result<Self> {
		register_common_controls();

		let mut labels = Vec::new();
		let mut label = |text: PCWSTR| -> Result<HWND> {
			let handle = static_text(parent, text, SS_LEFT)?;
			labels.push(handle);
			Ok(handle)
		};

		label(w!("Target application"))?;
		label(w!("Raise volume key"))?;
		label(w!("Lower volume key"))?;
		label(w!("Knob press"))?;

		let dropdown = CBS_DROPDOWNLIST as u32;

		// editable, so an application that is silent right now can be typed in
		let target = combo(parent, id::TARGET, CBS_DROPDOWN as u32 | CBS_AUTOHSCROLL as u32)?;
		let refresh = button(parent, id::REFRESH, w!("Refresh"), BS_PUSHBUTTON as u32)?;
		let raise = combo(parent, id::RAISE, dropdown)?;
		let lower = combo(parent, id::LOWER, dropdown)?;
		let step = trackbar(parent, id::STEP)?;
		let press = combo(parent, id::PRESS, dropdown)?;
		let startup =
			button(parent, id::STARTUP, w!("Launch on startup"), BS_AUTOCHECKBOX as u32)?;
		let save = button(parent, id::SAVE, w!("Save"), BS_DEFPUSHBUTTON as u32)?;
		let step_label = static_text(parent, w!("Volume step"), SS_LEFT)?;
		let status = static_text(parent, w!(""), SS_LEFT | SS_END_ELLIPSIS)?;

		let mut controls = Self {
			target,
			refresh,
			raise,
			lower,
			step,
			press,
			startup,
			save,
			step_label,
			status,
			labels,
			font: HFONT::default(),
		};

		controls.apply_font(parent);

		Ok(controls)
	}

	/// coordinates are written for 96 dpi and scaled here, which is also what
	/// makes `WM_DPICHANGED` a matter of relaying out rather than rebuilding
	/// the window.
	pub(crate) fn layout(&self, parent: HWND) {
		let dpi = unsafe { GetDpiForWindow(parent) };
		let scale = |value: i32| value * dpi as i32 / 96;

		let full = 368;
		let half = 179;
		let right = 205;

		let place = |handle: HWND, x: i32, y: i32, width: i32, height: i32| {
			unsafe {
				SetWindowPos(
					handle,
					None,
					scale(x),
					scale(y),
					scale(width),
					scale(height),
					SWP_NOZORDER | SWP_NOACTIVATE,
				)
			}
			.discard();
		};

		let labels = [
			(16, 12, full, 18),
			(16, 68, half, 18),
			(right, 68, half, 18),
			(16, 186, full, 18),
		];
		for (handle, (x, y, width, height)) in self.labels.iter().zip(labels) {
			place(*handle, x, y, width, height);
		}

		place(self.target, 16, 32, 262, LIST_HEIGHT);
		place(self.refresh, 288, 32, 96, 24);
		place(self.raise, 16, 88, half, LIST_HEIGHT);
		place(self.lower, right, 88, half, LIST_HEIGHT);
		place(self.step_label, 16, 124, full, 18);
		place(self.step, 16, 144, full, 32);
		place(self.press, 16, 206, full, LIST_HEIGHT);
		place(self.startup, 16, 244, full, 22);
		place(self.status, 16, 282, 240, 20);
		place(self.save, 274, 276, 110, 30);
	}

	pub(crate) fn refresh_font(&mut self, parent: HWND) {
		let previous = self.font;
		self.apply_font(parent);

		if !previous.is_invalid() && previous.0 != self.font.0 {
			unsafe { DeleteObject(previous.into()) }.discard();
		}
	}

	fn handles(&self) -> impl Iterator<Item = HWND> {
		[
			self.target,
			self.refresh,
			self.raise,
			self.lower,
			self.step,
			self.press,
			self.startup,
			self.save,
			self.step_label,
			self.status,
		]
		.into_iter()
		.chain(self.labels.clone())
	}

	fn apply_font(&mut self, parent: HWND) {
		let Some(font) = ui_font(parent) else { return };

		for handle in self.handles() {
			unsafe {
				SendMessageW(handle, WM_SETFONT, Some(WPARAM(font.0 as usize)), Some(LPARAM(1)));
			}
		}

		self.font = font;
	}
}

impl Drop for Controls {
	fn drop(&mut self) {
		if !self.font.is_invalid() {
			unsafe { DeleteObject(self.font.into()) }.discard();
		}
	}
}

fn register_common_controls() {
	let init = INITCOMMONCONTROLSEX {
		dwSize: size_of::<INITCOMMONCONTROLSEX>() as u32,
		dwICC: ICC_BAR_CLASSES | ICC_STANDARD_CLASSES,
	};

	unsafe { InitCommonControlsEx(&raw const init) }.discard();
}

/// `control` doubles as the child window id, zero meaning a control that never
/// reports back.
fn child(parent: HWND, class: PCWSTR, text: PCWSTR, style: u32, control: i32) -> Result<HWND> {
	let menu = if control == 0 {
		None
	} else {
		Some(HMENU(control as isize as *mut c_void))
	};

	unsafe {
		CreateWindowExW(
			WINDOW_EX_STYLE(0),
			class,
			text,
			WINDOW_STYLE(style) | WS_CHILD | WS_VISIBLE,
			0,
			0,
			0,
			0,
			Some(parent),
			menu,
			None,
			None,
		)
	}
	.context("failed to create a control")
}

fn static_text(parent: HWND, text: PCWSTR, style: u32) -> Result<HWND> {
	child(parent, w!("STATIC"), text, style, 0)
}

fn button(parent: HWND, control: i32, text: PCWSTR, style: u32) -> Result<HWND> {
	child(parent, w!("BUTTON"), text, style | WS_TABSTOP.0, control)
}

fn combo(parent: HWND, control: i32, style: u32) -> Result<HWND> {
	child(parent, w!("COMBOBOX"), w!(""), style | WS_TABSTOP.0 | WS_VSCROLL.0, control)
}

fn trackbar(parent: HWND, control: i32) -> Result<HWND> {
	child(
		parent,
		w!("msctls_trackbar32"),
		w!(""),
		TBS_HORZ | TBS_AUTOTICKS | WS_TABSTOP.0,
		control,
	)
}

/// the font windows uses for dialog text at the given window's dpi.
fn ui_font(parent: HWND) -> Option<HFONT> {
	let dpi = unsafe { GetDpiForWindow(parent) };

	let mut metrics = NONCLIENTMETRICSW {
		cbSize: size_of::<NONCLIENTMETRICSW>() as u32,
		..Default::default()
	};

	unsafe {
		SystemParametersInfoForDpi(
			SPI_GETNONCLIENTMETRICS.0,
			size_of::<NONCLIENTMETRICSW>() as u32,
			Some((&raw mut metrics).cast()),
			0,
			dpi,
		)
	}
	.ok()?;

	let font = unsafe { CreateFontIndirectW(&raw const metrics.lfMessageFont) };

	(!font.is_invalid()).then_some(font)
}

pub(crate) fn combo_clear(handle: HWND) {
	unsafe { SendMessageW(handle, CB_RESETCONTENT, None, None) };
}

pub(crate) fn combo_add(handle: HWND, text: &str) {
	let encoded = wide(text);

	unsafe {
		SendMessageW(handle, CB_ADDSTRING, None, Some(LPARAM(encoded.as_ptr() as isize)));
	}
}

pub(crate) fn combo_select(handle: HWND, index: i32) {
	unsafe { SendMessageW(handle, CB_SETCURSEL, Some(WPARAM(index as usize)), None) };
}

/// `-1` when the user typed a value instead of picking one.
pub(crate) fn combo_selection(handle: HWND) -> i32 {
	unsafe { SendMessageW(handle, CB_GETCURSEL, None, None) }.0 as i32
}

/// for an editable combo box this is what the user typed.
pub(crate) fn control_text(handle: HWND) -> String {
	let mut buffer = [0_u16; 260];

	unsafe { GetWindowTextW(handle, &mut buffer) };

	from_wide(&buffer).trim().to_owned()
}

pub(crate) fn set_control_text(handle: HWND, text: &str) {
	let encoded = wide(text);

	unsafe { SetWindowTextW(handle, PCWSTR(encoded.as_ptr())) }.ok();
}

pub(crate) fn trackbar_init(handle: HWND, min: i32, max: i32) {
	unsafe {
		SendMessageW(
			handle,
			TBM_SETRANGE,
			Some(WPARAM(1)),
			Some(LPARAM(((max << 16) | min) as isize)),
		);
		SendMessageW(handle, TBM_SETTICFREQ, Some(WPARAM(1)), None);
		SendMessageW(handle, TBM_SETPAGESIZE, None, Some(LPARAM(5)));
	}
}

pub(crate) fn trackbar_set(handle: HWND, value: i32) {
	unsafe { SendMessageW(handle, TBM_SETPOS, Some(WPARAM(1)), Some(LPARAM(value as isize))) };
}

pub(crate) fn trackbar_get(handle: HWND) -> i32 {
	unsafe { SendMessageW(handle, TBM_GET_POS, None, None) }.0 as i32
}

pub(crate) fn check_set(handle: HWND, checked: bool) {
	let state = if checked { BST_IS_CHECKED } else { 0 };

	unsafe { SendMessageW(handle, BM_SETCHECK, Some(WPARAM(state)), None) };
}

pub(crate) fn check_get(handle: HWND) -> bool {
	let state = unsafe { SendMessageW(handle, BM_GETCHECK, None, None) };

	state.0 as usize == BST_IS_CHECKED
}
