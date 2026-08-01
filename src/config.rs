use std::{
	fs,
	path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use windows::Win32::UI::Input::KeyboardAndMouse::{
	VIRTUAL_KEY, VK_MEDIA_PLAY_PAUSE, VK_VOLUME_DOWN, VK_VOLUME_UP,
};

pub(crate) const STEP_MIN: u8 = 1;
pub(crate) const STEP_MAX: u8 = 25;

/// only the clockwise direction is stored: the counter-clockwise one is always
/// the other key, which keeps the two ui dropdowns from ever describing an
/// impossible mapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum KnobKey {
	VolumeUp,
	VolumeDown,
}

impl KnobKey {
	pub(crate) fn virtual_key(self) -> VIRTUAL_KEY {
		match self {
			| Self::VolumeUp => VK_VOLUME_UP,
			| Self::VolumeDown => VK_VOLUME_DOWN,
		}
	}

	pub(crate) fn opposite(self) -> Self {
		match self {
			| Self::VolumeUp => Self::VolumeDown,
			| Self::VolumeDown => Self::VolumeUp,
		}
	}

	/// index into the ui dropdown, which lists volume up first.
	pub(crate) fn index(self) -> i32 {
		match self {
			| Self::VolumeUp => 0,
			| Self::VolumeDown => 1,
		}
	}

	pub(crate) fn from_index(index: i32) -> Self {
		if index == 1 { Self::VolumeDown } else { Self::VolumeUp }
	}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum PressAction {
	MuteToggle,
	NextTarget,
	/// let the key through, so the focused player pauses as it normally would.
	PassThrough,
	/// swallow the key and do nothing.
	Disabled,
}

impl PressAction {
	pub(crate) fn virtual_key() -> VIRTUAL_KEY { VK_MEDIA_PLAY_PAUSE }

	/// index into the ui dropdown, in declaration order.
	pub(crate) fn index(self) -> i32 {
		match self {
			| Self::MuteToggle => 0,
			| Self::NextTarget => 1,
			| Self::PassThrough => 2,
			| Self::Disabled => 3,
		}
	}

	pub(crate) fn from_index(index: i32) -> Self {
		match index {
			| 1 => Self::NextTarget,
			| 2 => Self::PassThrough,
			| 3 => Self::Disabled,
			| _ => Self::MuteToggle,
		}
	}
}

/// the whole contents of `config.toml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub(crate) struct Settings {
	/// executable name of the application the knob currently drives.
	pub(crate) target: String,
	/// every application the user has picked, cycled through by `NextTarget`.
	pub(crate) targets: Vec<String>,
	/// the key that raises the volume; the other one lowers it.
	pub(crate) raise_key: KnobKey,
	pub(crate) step_percent: u8,
	pub(crate) press_action: PressAction,
	pub(crate) launch_on_startup: bool,
}

impl Default for Settings {
	fn default() -> Self {
		Self {
			target: String::new(),
			targets: Vec::new(),
			raise_key: KnobKey::VolumeUp,
			step_percent: 5,
			press_action: PressAction::MuteToggle,
			launch_on_startup: false,
		}
	}
}

impl Settings {
	pub(crate) fn path() -> Result<PathBuf> {
		let roaming =
			std::env::var_os("APPDATA").context("the APPDATA environment variable is not set")?;

		Ok(Path::new(&roaming)
			.join("knob")
			.join("config.toml"))
	}

	/// missing file means first run, anything else is a real error worth
	/// showing.
	pub(crate) fn load() -> Result<Self> {
		let path = Self::path()?;
		if !path.exists() {
			return Ok(Self::default());
		}

		let raw = fs::read_to_string(&path)
			.with_context(|| format!("failed to read {}", path.display()))?;

		let settings: Self = toml::from_str(&raw)
			.with_context(|| format!("failed to parse {}", path.display()))?;

		Ok(settings.normalized())
	}

	pub(crate) fn save(&self) -> Result<()> {
		let path = Self::path()?;
		if let Some(parent) = path.parent() {
			fs::create_dir_all(parent)
				.with_context(|| format!("failed to create {}", parent.display()))?;
		}

		let raw = toml::to_string_pretty(self).context("failed to serialize the settings")?;

		fs::write(&path, raw).with_context(|| format!("failed to write {}", path.display()))
	}

	pub(crate) fn lower_key(&self) -> KnobKey { self.raise_key.opposite() }

	pub(crate) fn step_scalar(&self) -> f32 { f32::from(self.step_percent) / 100.0 }

	/// the target after the current one, wrapping around, or `None` when fewer
	/// than two are configured.
	pub(crate) fn next_target(&self) -> Option<&str> {
		if self.targets.len() < 2 {
			return None;
		}

		let current = self
			.targets
			.iter()
			.position(|name| name == &self.target);
		let next = current.map_or(0, |index| (index + 1) % self.targets.len());

		self.targets.get(next).map(String::as_str)
	}

	pub(crate) fn remember_target(&mut self, name: &str) {
		if name.is_empty() {
			return;
		}

		let known = self
			.targets
			.iter()
			.any(|known| known.eq_ignore_ascii_case(name));
		if !known {
			self.targets.push(name.to_owned());
		}
	}

	/// clamps anything a hand-edited config file could have put out of range.
	fn normalized(mut self) -> Self {
		self.step_percent = self.step_percent.clamp(STEP_MIN, STEP_MAX);
		self.targets.retain(|name| !name.is_empty());

		let target = self.target.clone();
		self.remember_target(&target);

		self
	}
}
