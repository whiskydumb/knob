use anyhow::{Context, Result};
use windows::{
	Win32::{
		Foundation::{HWND, LPARAM, POINT},
		UI::{
			Shell::{
				NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY, NOTIFYICONDATAW,
				Shell_NotifyIconW,
			},
			WindowsAndMessaging::{
				AppendMenuW, CreatePopupMenu, DestroyMenu, GetCursorPos, HMENU, MENU_ITEM_FLAGS,
				MF_CHECKED, MF_SEPARATOR, MF_STRING, MF_UNCHECKED, SetForegroundWindow,
				TPM_BOTTOMALIGN, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenu, WM_APP,
			},
		},
	},
	core::PCWSTR,
};

use crate::{
	ui::{app_icon, small_icon},
	win::{Discard, wide},
};

/// posted by the shell when the user interacts with the tray icon.
pub(crate) const WM_TRAY: u32 = WM_APP + 2;

const ID_OPEN: usize = 1;
const ID_SUSPEND: usize = 2;
const ID_EXIT: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MenuChoice {
	Open,
	ToggleSuspend,
	Exit,
}

/// the tray icon, removed from the notification area when dropped.
pub(crate) struct Tray {
	data: NOTIFYICONDATAW,
}

impl Tray {
	pub(crate) fn new(window: HWND) -> Result<Self> {
		let icon = app_icon(small_icon()).context("failed to load the tray icon")?;

		let mut data = NOTIFYICONDATAW {
			cbSize: size_of::<NOTIFYICONDATAW>() as u32,
			hWnd: window,
			uID: 1,
			uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP,
			uCallbackMessage: WM_TRAY,
			hIcon: icon,
			..Default::default()
		};

		write_tip(&mut data, "knob");

		unsafe { Shell_NotifyIconW(NIM_ADD, &raw const data) }
			.ok()
			.context("failed to add the tray icon")?;

		Ok(Self { data })
	}

	pub(crate) fn set_tooltip(&mut self, text: &str) {
		write_tip(&mut self.data, text);

		unsafe { Shell_NotifyIconW(NIM_MODIFY, &raw const self.data) }.discard();
	}

	/// blocks until the user picks an entry or dismisses the menu.
	pub(crate) fn show_menu(window: HWND, suspended: bool) -> Option<MenuChoice> {
		let mut point = POINT::default();

		unsafe { GetCursorPos(&raw mut point) }.ok()?;

		let menu = unsafe { CreatePopupMenu() }.ok()?;

		let open = wide("Open");
		let suspend = wide("Suspend interception");
		let exit = wide("Exit");

		let suspend_flags = MF_STRING | if suspended { MF_CHECKED } else { MF_UNCHECKED };

		append(menu, MF_STRING, ID_OPEN, PCWSTR(open.as_ptr()));
		append(menu, suspend_flags, ID_SUSPEND, PCWSTR(suspend.as_ptr()));
		append(menu, MF_SEPARATOR, 0, PCWSTR::null());
		append(menu, MF_STRING, ID_EXIT, PCWSTR(exit.as_ptr()));

		// without this the menu refuses to close when the user clicks away, a
		// documented quirk of tray menus
		unsafe { SetForegroundWindow(window) }.discard();

		let chosen = unsafe {
			TrackPopupMenu(
				menu,
				TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_BOTTOMALIGN,
				point.x,
				point.y,
				None,
				window,
				None,
			)
		};

		unsafe { DestroyMenu(menu) }.discard();

		match chosen.0 as usize {
			| ID_OPEN => Some(MenuChoice::Open),
			| ID_SUSPEND => Some(MenuChoice::ToggleSuspend),
			| ID_EXIT => Some(MenuChoice::Exit),
			| _ => None,
		}
	}
}

impl Drop for Tray {
	fn drop(&mut self) {
		unsafe { Shell_NotifyIconW(NIM_DELETE, &raw const self.data) }.discard();
	}
}

/// `text` must outlive the menu, which lives until it is tracked and destroyed.
fn append(menu: HMENU, flags: MENU_ITEM_FLAGS, id: usize, text: PCWSTR) {
	unsafe { AppendMenuW(menu, flags, id, text) }.discard();
}

/// copies `text` into the fixed-size tooltip buffer, truncating if needed.
fn write_tip(data: &mut NOTIFYICONDATAW, text: &str) {
	let encoded = wide(text);
	let length = encoded.len().min(data.szTip.len());

	data.szTip.fill(0);
	data.szTip[..length].copy_from_slice(&encoded[..length]);
	// keep the buffer terminated even when the text was cut
	if let Some(last) = data.szTip.get_mut(length.saturating_sub(1)) {
		*last = 0;
	}
}

/// the low word of `lparam` carries the `WM_*` mouse message.
pub(crate) const fn tray_event(lparam: LPARAM) -> u32 { (lparam.0 as u32) & 0xFFFF }
