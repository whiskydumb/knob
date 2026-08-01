use anyhow::{Context, Result};
use windows::{
	Win32::{
		Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, MAX_PATH},
		System::{
			LibraryLoader::GetModuleFileNameW,
			Registry::{
				HKEY, HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_SAM_FLAGS, REG_SZ,
				RegCloseKey, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
			},
		},
	},
	core::{PCWSTR, w},
};

use crate::win::{Discard, from_wide, wide};

const RUN_KEY: PCWSTR = w!(r"Software\Microsoft\Windows\CurrentVersion\Run");
const VALUE_NAME: PCWSTR = w!("knob");

/// passed to the autostarted instance so it comes up in the tray only.
pub(crate) const HIDDEN_FLAG: &str = "--hidden";

pub(crate) fn is_enabled() -> bool {
	let Ok(key) = open(KEY_QUERY_VALUE) else { return false };

	// querying with no output buffer only reports whether the value exists
	let status = unsafe { RegQueryValueExW(key.0, VALUE_NAME, None, None, None, None) };

	status == ERROR_SUCCESS
}

pub(crate) fn set(enabled: bool) -> Result<()> {
	let key = open(KEY_SET_VALUE | KEY_QUERY_VALUE)?;

	if enabled { write(key.0) } else { erase(key.0) }
}

/// an open registry key, closed when dropped.
struct OpenKey(HKEY);

impl Drop for OpenKey {
	fn drop(&mut self) { unsafe { RegCloseKey(self.0) }.discard(); }
}

fn open(access: REG_SAM_FLAGS) -> Result<OpenKey> {
	let mut key = HKEY::default();

	let status = unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, RUN_KEY, None, access, &raw mut key) };

	status
		.ok()
		.context("failed to open the startup registry key")?;

	Ok(OpenKey(key))
}

/// writes `"<exe path>" --hidden` into the run key.
fn write(key: HKEY) -> Result<()> {
	let command = format!("\"{}\" {HIDDEN_FLAG}", executable_path()?);
	let encoded = wide(&command);

	// a REG_SZ payload is measured in bytes and includes the terminator
	// SAFETY: reinterpreting the utf-16 buffer as bytes keeps the same allocation,
	// length and lifetime, and u8 has no alignment requirement
	let bytes =
		unsafe { std::slice::from_raw_parts(encoded.as_ptr().cast::<u8>(), encoded.len() * 2) };

	let status = unsafe { RegSetValueExW(key, VALUE_NAME, None, REG_SZ, Some(bytes)) };

	status
		.ok()
		.context("failed to write the startup registry value")
}

fn erase(key: HKEY) -> Result<()> {
	let status = unsafe { RegDeleteValueW(key, VALUE_NAME) };

	if status == ERROR_SUCCESS {
		return Ok(());
	}

	// nothing to delete is the state the caller asked for
	if status == ERROR_FILE_NOT_FOUND {
		return Ok(());
	}

	status
		.ok()
		.context("failed to remove the startup registry value")
}

fn executable_path() -> Result<String> {
	let mut buffer = [0_u16; MAX_PATH as usize];

	let length = unsafe { GetModuleFileNameW(None, &mut buffer) };
	if length == 0 {
		anyhow::bail!("failed to read the path of the running executable");
	}

	Ok(from_wide(&buffer))
}
