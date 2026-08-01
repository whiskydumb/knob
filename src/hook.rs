use std::{
	ffi::c_void,
	sync::{
		OnceLock,
		atomic::{AtomicBool, AtomicIsize, AtomicU16, Ordering},
	},
};

use anyhow::{Context, Result};
use windows::Win32::{
	Foundation::{HWND, LPARAM, LRESULT, WPARAM},
	UI::{
		Input::KeyboardAndMouse::VIRTUAL_KEY,
		WindowsAndMessaging::{
			CallNextHookEx, HC_ACTION, HHOOK, KBDLLHOOKSTRUCT, PostMessageW, SetWindowsHookExW,
			UnhookWindowsHookEx, WH_KEYBOARD_LL, WM_APP, WM_KEYDOWN, WM_KEYUP, WM_SYSKEYDOWN,
			WM_SYSKEYUP,
		},
	},
};

use crate::{config::PressAction, win::Discard};

/// posted to the main window when the hook swallowed a knob key, with the
/// command encoded in `wparam`.
pub(crate) const WM_KNOB_COMMAND: u32 = WM_APP + 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Command {
	Raise,
	Lower,
	Press,
}

impl Command {
	const fn code(self) -> usize {
		match self {
			| Self::Raise => 1,
			| Self::Lower => 2,
			| Self::Press => 3,
		}
	}

	pub(crate) const fn from_code(code: usize) -> Option<Self> {
		match code {
			| 1 => Some(Self::Raise),
			| 2 => Some(Self::Lower),
			| 3 => Some(Self::Press),
			| _ => None,
		}
	}
}

/// kept in atomics rather than behind a lock: the callback must never block,
/// and a torn read here would at worst mis-route a single detent.
struct Shared {
	window: AtomicIsize,
	raise: AtomicU16,
	lower: AtomicU16,
	press: AtomicU16,
	/// whether the press key is ours at all, or belongs to the focused player.
	intercept_press: AtomicBool,
	/// set while the configured target actually owns an audio session; when it
	/// is clear the keys fall through and windows adjusts the system volume as
	/// usual.
	armed: AtomicBool,
}

static SHARED: OnceLock<Shared> = OnceLock::new();

fn shared() -> &'static Shared {
	SHARED.get_or_init(|| Shared {
		window: AtomicIsize::new(0),
		raise: AtomicU16::new(0),
		lower: AtomicU16::new(0),
		press: AtomicU16::new(0),
		intercept_press: AtomicBool::new(false),
		armed: AtomicBool::new(false),
	})
}

pub(crate) fn set_window(window: HWND) {
	shared()
		.window
		.store(window.0 as isize, Ordering::Relaxed);
}

pub(crate) fn set_binding(raise: VIRTUAL_KEY, lower: VIRTUAL_KEY, press: PressAction) {
	let state = shared();

	state.raise.store(raise.0, Ordering::Relaxed);
	state.lower.store(lower.0, Ordering::Relaxed);
	state
		.press
		.store(PressAction::virtual_key().0, Ordering::Relaxed);
	state
		.intercept_press
		.store(press != PressAction::PassThrough, Ordering::Relaxed);
}

pub(crate) fn set_armed(armed: bool) { shared().armed.store(armed, Ordering::Relaxed); }

/// an installed keyboard hook, removed when dropped.
pub(crate) struct Hook {
	handle: HHOOK,
}

impl Hook {
	/// a hook installed by a non-elevated process never sees keys pressed while
	/// an elevated window has focus, which is a uipi rule rather than a bug.
	pub(crate) fn install() -> Result<Self> {
		let handle = unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_proc), None, 0) }
			.context("failed to install the keyboard hook")?;

		Ok(Self { handle })
	}
}

impl Drop for Hook {
	fn drop(&mut self) { unsafe { UnhookWindowsHookEx(self.handle) }.discard(); }
}

fn classify(state: &Shared, key: u16) -> Option<Command> {
	if key == state.raise.load(Ordering::Relaxed) {
		return Some(Command::Raise);
	}

	if key == state.lower.load(Ordering::Relaxed) {
		return Some(Command::Lower);
	}

	let press = key == state.press.load(Ordering::Relaxed);
	if press && state.intercept_press.load(Ordering::Relaxed) {
		return Some(Command::Press);
	}

	None
}

/// windows evicts a callback that takes more than a few hundred milliseconds,
/// so this one only classifies and posts. returning a non-zero value is what
/// stops the system volume from moving and keeps the volume osd off the screen.
unsafe extern "system" fn keyboard_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
	let ours = code == HC_ACTION as i32 && unsafe { intercepts(wparam.0 as u32, lparam) };

	if ours {
		return LRESULT(1);
	}

	unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

/// only valid for `HC_ACTION`, the one code where `lparam` carries a key event.
unsafe fn intercepts(message: u32, lparam: LPARAM) -> bool {
	let state = shared();
	let window = state.window.load(Ordering::Relaxed);
	let armed = state.armed.load(Ordering::Relaxed);

	// SAFETY: under HC_ACTION windows guarantees a live KBDLLHOOKSTRUCT here
	let event = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };

	let key = event.vkCode as u16;
	let command = classify(state, key);

	if window == 0 || !armed {
		return false;
	}

	// @note the injected flag is deliberately not filtered. a knob on a bluetooth
	// or hid consumer-control device never produces a hardware key event: a system
	// component reads the hid report and synthesizes one, so every detent arrives
	// flagged as injected with a zero scan code. knob itself never calls SendInput,
	// so there is no feedback loop to guard against either
	let Some(command) = command else { return false };

	let pressed = matches!(message, WM_KEYDOWN | WM_SYSKEYDOWN);
	let released = matches!(message, WM_KEYUP | WM_SYSKEYUP);

	if pressed {
		let target = HWND(window as *mut c_void);

		unsafe { PostMessageW(Some(target), WM_KNOB_COMMAND, WPARAM(command.code()), LPARAM(0)) }
			.discard();
	}

	// the key-up has to disappear too, otherwise the shell still sees a complete
	// keystroke and acts on it
	pressed || released
}
