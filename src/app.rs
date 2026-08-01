use anyhow::Result;
use windows::Win32::{
	Foundation::HWND,
	UI::WindowsAndMessaging::{SW_HIDE, SW_SHOW, SetForegroundWindow, ShowWindow},
};

use crate::{
	audio::Mixer,
	autostart,
	config::{KnobKey, PressAction, STEP_MAX, STEP_MIN, Settings},
	hook::{self, Command, Hook},
	ui::{
		layout::{
			self, Controls, check_get, check_set, combo_add, combo_clear, combo_select,
			combo_selection, control_text, set_control_text, trackbar_get, trackbar_init,
			trackbar_set,
		},
		tray::{MenuChoice, Tray},
	},
	win::Discard,
};

/// how often the target is re-checked for live audio sessions.
pub(crate) const TICK_MS: u32 = 2000;

pub(crate) struct App {
	window: HWND,
	settings: Settings,
	mixer: Mixer,
	controls: Controls,
	tray: Tray,
	/// `None` while interception is suspended: the hook is physically removed
	/// rather than just ignored, so nothing of ours sits in the keyboard chain.
	hook: Option<Hook>,
	/// what the last tick concluded, so the polling loop only writes the status
	/// line when something actually changed and does not wipe out what the user
	/// just did.
	armed: Option<bool>,
}

impl App {
	pub(crate) fn new(window: HWND) -> Result<Self> {
		let (settings, load_error) = match Settings::load() {
			| Ok(settings) => (settings, None),
			| Err(error) => (Settings::default(), Some(format!("{error:#}"))),
		};

		let mut app = Self {
			window,
			settings,
			mixer: Mixer::new()?,
			controls: Controls::create(window)?,
			tray: Tray::new(window)?,
			hook: Some(Hook::install()?),
			armed: None,
		};

		app.controls.layout(window);
		app.populate();
		app.apply_binding();
		app.tick();

		if let Some(error) = load_error {
			app.set_status(&error);
		}

		Ok(app)
	}

	pub(crate) fn rescale(&mut self) {
		self.controls.refresh_font(self.window);
		self.controls.layout(self.window);
	}

	pub(crate) fn on_knob(&mut self, command: Command) {
		match command {
			| Command::Raise => self.step_volume(self.settings.step_scalar()),
			| Command::Lower => self.step_volume(-self.settings.step_scalar()),
			| Command::Press => self.on_press(),
		}
	}

	pub(crate) fn on_control(&mut self, control: i32, notification: u16) {
		const BN_CLICKED: u16 = 0;
		const CBN_SELCHANGE: u16 = 1;

		match (control, notification) {
			| (layout::id::REFRESH, BN_CLICKED) => {
				let found = self.refresh_targets();
				self.set_status(&format!("{found} applications to choose from"));
			},
			| (layout::id::SAVE, BN_CLICKED) => self.save(),
			| (layout::id::RAISE, CBN_SELCHANGE) => {
				let raise = KnobKey::from_index(combo_selection(self.controls.raise));
				combo_select(self.controls.lower, raise.opposite().index());
				self.mark_unsaved();
			},
			| (layout::id::LOWER, CBN_SELCHANGE) => {
				let lower = KnobKey::from_index(combo_selection(self.controls.lower));
				combo_select(self.controls.raise, lower.opposite().index());
				self.mark_unsaved();
			},
			| (layout::id::TARGET | layout::id::PRESS, CBN_SELCHANGE)
			| (layout::id::STARTUP, BN_CLICKED) => self.mark_unsaved(),
			| _ => {},
		}
	}

	pub(crate) fn on_slider(&self) {
		let step = trackbar_get(self.controls.step);
		set_control_text(self.controls.step_label, &format!("Volume step: {step}% per detent"));
		self.mark_unsaved();
	}

	/// returns false when the program should quit.
	pub(crate) fn on_tray(&mut self, choice: MenuChoice) -> bool {
		match choice {
			| MenuChoice::Open => self.show(),
			| MenuChoice::ToggleSuspend => self.toggle_suspend(),
			| MenuChoice::Exit => return false,
		}

		true
	}

	pub(crate) fn is_suspended(&self) -> bool { self.hook.is_none() }

	pub(crate) fn show(&self) {
		unsafe {
			ShowWindow(self.window, SW_SHOW).discard();
			SetForegroundWindow(self.window).discard();
		}
	}

	/// closing the window only hides it, the program stays in the tray.
	pub(crate) fn hide(&self) { unsafe { ShowWindow(self.window, SW_HIDE) }.discard(); }

	/// when the target is not playing anything the keys are handed back to
	/// windows, so closing spotify turns the knob into a system volume knob
	/// again instead of making it dead.
	pub(crate) fn tick(&mut self) {
		self.mixer.invalidate();

		let live = !self.settings.target.is_empty()
			&& self
				.mixer
				.volume(&self.settings.target)
				.ok()
				.flatten()
				.is_some();

		let armed = live && !self.is_suspended();
		hook::set_armed(armed);
		self.update_tooltip();

		// only speak up on a transition, otherwise polling would keep overwriting
		// whatever the user was just told
		if self.armed != Some(armed) {
			self.armed = Some(armed);
			self.set_status(&self.state_line(armed));
		}
	}

	fn state_line(&self, armed: bool) -> String {
		if self.is_suspended() {
			return "interception suspended".to_owned();
		}

		if self.settings.target.is_empty() {
			return "no target selected, the knob still controls system volume".to_owned();
		}

		let target = &self.settings.target;
		if armed {
			format!("driving {target}")
		} else {
			format!("{target} is not playing, the knob falls back to system volume")
		}
	}

	fn step_volume(&mut self, delta: f32) {
		let target = self.settings.target.clone();
		if target.is_empty() {
			return;
		}

		match self.mixer.adjust(&target, delta) {
			| Ok(Some(level)) => {
				let percent = (level * 100.0).round() as i32;
				self.set_status(&format!("{target} - {percent}%"));
				self.update_tooltip();
			},
			| Ok(None) => {
				// nothing is playing under that name, stop swallowing the keys
				hook::set_armed(false);
				self.set_status(&format!("{target} has no audio session"));
			},
			| Err(error) => self.set_status(&format!("{error:#}")),
		}
	}

	fn on_press(&mut self) {
		match self.settings.press_action {
			| PressAction::MuteToggle => self.toggle_mute(),
			| PressAction::NextTarget => self.cycle_target(),
			| PressAction::PassThrough | PressAction::Disabled => {},
		}
	}

	fn toggle_mute(&mut self) {
		let target = self.settings.target.clone();
		if target.is_empty() {
			return;
		}

		match self.mixer.toggle_mute(&target) {
			| Ok(Some(muted)) => {
				let state = if muted { "muted" } else { "unmuted" };
				self.set_status(&format!("{target} {state}"));
				self.update_tooltip();
			},
			| Ok(None) => hook::set_armed(false),
			| Err(error) => self.set_status(&format!("{error:#}")),
		}
	}

	fn cycle_target(&mut self) {
		let Some(next) = self.settings.next_target().map(str::to_owned) else { return };

		self.settings.target.clone_from(&next);
		self.mixer.invalidate();
		set_control_text(self.controls.target, &next);
		self.set_status(&format!("switched to {next}"));

		if let Err(error) = self.settings.save() {
			self.set_status(&format!("{error:#}"));
		}

		self.forget_state();
		self.tick();
	}

	fn toggle_suspend(&mut self) {
		if self.hook.is_some() {
			self.hook = None;
		} else if let Err(error) = Hook::install().map(|hook| self.hook = Some(hook)) {
			self.set_status(&format!("{error:#}"));
			return;
		}

		self.forget_state();
		self.tick();
	}

	fn save(&mut self) {
		let target = control_text(self.controls.target);
		let raise = KnobKey::from_index(combo_selection(self.controls.raise));
		let press = PressAction::from_index(combo_selection(self.controls.press));
		let step =
			trackbar_get(self.controls.step).clamp(i32::from(STEP_MIN), i32::from(STEP_MAX));
		let startup = check_get(self.controls.startup);

		self.settings.target.clone_from(&target);
		self.settings.raise_key = raise;
		self.settings.press_action = press;
		self.settings.step_percent = step as u8;
		self.settings.launch_on_startup = startup;
		self.settings.remember_target(&target);

		if let Err(error) = autostart::set(startup) {
			self.set_status(&format!("{error:#}"));
			return;
		}

		if let Err(error) = self.settings.save() {
			self.set_status(&format!("{error:#}"));
			return;
		}

		self.apply_binding();
		self.mixer.invalidate();
		self.populate();
		self.tick();
		self.set_status("settings saved");
	}

	fn populate(&self) {
		let raise = self.settings.raise_key;

		combo_clear(self.controls.raise);
		combo_clear(self.controls.lower);
		for name in ["Volume Up", "Volume Down"] {
			combo_add(self.controls.raise, name);
			combo_add(self.controls.lower, name);
		}
		combo_select(self.controls.raise, raise.index());
		combo_select(self.controls.lower, raise.opposite().index());

		combo_clear(self.controls.press);
		for name in ["Mute / unmute target", "Next target", "Pass through", "Do nothing"] {
			combo_add(self.controls.press, name);
		}
		combo_select(self.controls.press, self.settings.press_action.index());

		trackbar_init(self.controls.step, i32::from(STEP_MIN), i32::from(STEP_MAX));
		trackbar_set(self.controls.step, i32::from(self.settings.step_percent));
		set_control_text(
			self.controls.step_label,
			&format!("Volume step: {}% per detent", self.settings.step_percent),
		);

		check_set(self.controls.startup, autostart::is_enabled());

		self.refresh_targets();
	}

	/// forces the next tick to restate what the knob is doing.
	fn forget_state(&mut self) { self.armed = None; }

	/// the remembered names plus whatever is currently making sound.
	fn refresh_targets(&self) -> usize {
		let mut names = self.settings.targets.clone();

		match self.mixer.active_processes() {
			| Ok(active) => merge(&mut names, active),
			| Err(error) => self.set_status(&format!("{error:#}")),
		}

		combo_clear(self.controls.target);
		for name in &names {
			combo_add(self.controls.target, name);
		}

		// the edit field keeps the active target, whether or not it is in the list
		set_control_text(self.controls.target, &self.settings.target);

		names.len()
	}

	fn apply_binding(&self) {
		hook::set_binding(
			self.settings.raise_key.virtual_key(),
			self.settings.lower_key().virtual_key(),
			self.settings.press_action,
		);
	}

	fn mark_unsaved(&self) { set_control_text(self.controls.status, "unsaved changes"); }

	fn set_status(&self, text: &str) { set_control_text(self.controls.status, text); }

	fn update_tooltip(&mut self) {
		let target = self.settings.target.clone();

		let text = if self.is_suspended() {
			"knob - suspended".to_owned()
		} else if target.is_empty() {
			"knob - no target".to_owned()
		} else {
			match self.mixer.volume(&target).ok().flatten() {
				| Some(level) => {
					format!("knob - {target} {}%", (level * 100.0).round() as i32)
				},
				| None => format!("knob - {target} (silent)"),
			}
		};

		self.tray.set_tooltip(&text);
	}
}

fn merge(names: &mut Vec<String>, extra: Vec<String>) {
	for name in extra {
		if !names
			.iter()
			.any(|known| known.eq_ignore_ascii_case(&name))
		{
			names.push(name);
		}
	}
}
