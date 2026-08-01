#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod audio;
mod autostart;
mod config;
mod hook;
mod ui;
mod win;

use anyhow::Result;
use windows::Win32::{
	Foundation::{LPARAM, WPARAM},
	UI::WindowsAndMessaging::{FindWindowW, PostMessageW},
};

use crate::win::Discard;

fn main() {
	if let Err(error) = run() {
		win::error_dialog(&format!("{error:#}"));
	}
}

fn run() -> Result<()> {
	let hidden = std::env::args().any(|argument| argument == autostart::HIDDEN_FLAG);

	// a second instance would install a second hook and the two would fight over
	// the same keys, so hand focus to the one already running and step aside
	let Ok(_instance) = win::SingleInstance::acquire("knob") else {
		raise_existing();
		return Ok(());
	};

	let _com = win::ComGuard::enter()?;

	ui::run(hidden)
}

fn raise_existing() {
	let Ok(window) = (unsafe { FindWindowW(ui::CLASS_NAME, None) }) else { return };

	unsafe { PostMessageW(Some(window), ui::WM_KNOB_SHOW, WPARAM(0), LPARAM(0)) }.discard();
}
