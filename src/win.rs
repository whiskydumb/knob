use anyhow::{Context, Result, bail};
use windows::{
	Win32::{
		Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE},
		System::{
			Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize},
			Threading::CreateMutexW,
		},
		UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW},
	},
	core::{HSTRING, PCWSTR},
};

/// throws away a status win32 reports but the caller cannot act on, keeping
/// that decision visible instead of hiding it behind a bare semicolon.
pub(crate) trait Discard {
	fn discard(self);
}

impl<T> Discard for T {
	fn discard(self) {}
}

pub(crate) fn wide(value: &str) -> Vec<u16> {
	value
		.encode_utf16()
		.chain(std::iter::once(0))
		.collect()
}

pub(crate) fn from_wide(buffer: &[u16]) -> String {
	let end = buffer
		.iter()
		.position(|&unit| unit == 0)
		.unwrap_or(buffer.len());
	String::from_utf16_lossy(&buffer[..end])
}

/// under `windows_subsystem = "windows"` there is no stderr, so a dialog is the
/// only way a startup failure reaches the user.
pub(crate) fn error_dialog(message: &str) {
	let text = HSTRING::from(message);
	let caption = HSTRING::from("knob");

	unsafe {
		MessageBoxW(None, &text, &caption, MB_OK | MB_ICONERROR);
	}
}

pub(crate) struct SingleInstance {
	handle: HANDLE,
}

impl SingleInstance {
	/// `Local\` scopes the name to the current logon session, so two users on
	/// the same machine each get their own knob.
	pub(crate) fn acquire(name: &str) -> Result<Self> {
		let name = wide(&format!("Local\\{name}"));

		let handle = unsafe { CreateMutexW(None, true, PCWSTR(name.as_ptr())) }
			.context("failed to create the single-instance mutex")?;

		if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
			unsafe { CloseHandle(handle) }.discard();
			bail!("another instance is already running");
		}

		Ok(Self { handle })
	}
}

impl Drop for SingleInstance {
	fn drop(&mut self) { unsafe { CloseHandle(self.handle) }.discard(); }
}

/// the apartment has to outlive every core audio interface pointer, so this
/// guard is created first in `main` and dropped last.
pub(crate) struct ComGuard;

impl ComGuard {
	/// sta rather than mta because the same thread also owns the window and its
	/// message loop.
	pub(crate) fn enter() -> Result<Self> {
		unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }
			.ok()
			.context("failed to initialize com")?;

		Ok(Self)
	}
}

impl Drop for ComGuard {
	fn drop(&mut self) { unsafe { CoUninitialize() }; }
}
