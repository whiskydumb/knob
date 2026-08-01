use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use windows::{
	Win32::{
		Foundation::{CloseHandle, MAX_PATH},
		Media::Audio::{
			AudioSessionStateExpired, IAudioSessionControl2, IAudioSessionManager2,
			IMMDeviceEnumerator, ISimpleAudioVolume, MMDeviceEnumerator, eMultimedia, eRender,
		},
		System::{
			Com::{CLSCTX_ALL, CoCreateInstance},
			Threading::{
				OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
				QueryFullProcessImageNameW,
			},
		},
	},
	core::{Interface, PWSTR},
};

use crate::win::from_wide;

/// a turn of the knob fires far faster than this, so a burst of detents costs
/// one enumeration rather than one per click.
const CACHE_TTL: Duration = Duration::from_millis(1500);

pub(crate) struct Mixer {
	enumerator: IMMDeviceEnumerator,
	cache: Option<Cache>,
}

struct Cache {
	target: String,
	sessions: Vec<ISimpleAudioVolume>,
	at: Instant,
}

impl Mixer {
	/// the calling thread must already have entered a com apartment.
	pub(crate) fn new() -> Result<Self> {
		let enumerator: IMMDeviceEnumerator =
			unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }
				.context("failed to create the audio device enumerator")?;

		Ok(Self { enumerator, cache: None })
	}

	/// applications that are merely idle still show up, so a paused player can
	/// be picked as a target; only sessions the system has retired are
	/// skipped.
	pub(crate) fn active_processes(&self) -> Result<Vec<String>> {
		let manager = self.session_manager()?;

		let sessions = unsafe { manager.GetSessionEnumerator() }
			.context("failed to enumerate audio sessions")?;

		let count = unsafe { sessions.GetCount() }.unwrap_or(0);

		let mut names = Vec::new();
		for index in 0..count {
			let Ok(control) = (unsafe { sessions.GetSession(index) }) else { continue };
			let Ok(control) = control.cast::<IAudioSessionControl2>() else { continue };

			let expired = unsafe { control.GetState() }
				.is_ok_and(|state| state == AudioSessionStateExpired);
			if expired {
				continue;
			}

			let Ok(pid) = (unsafe { control.GetProcessId() }) else { continue };
			let Some(name) = process_name(pid) else { continue };

			if !names
				.iter()
				.any(|known: &String| known.eq_ignore_ascii_case(&name))
			{
				names.push(name);
			}
		}

		names.sort_unstable_by_key(|name| name.to_lowercase());

		Ok(names)
	}

	/// a target can own several sessions at once, since every chromium-based
	/// browser spreads audio over a handful of renderer processes, so the delta
	/// is applied to all of them and the first result is reported back.
	pub(crate) fn adjust(&mut self, target: &str, delta: f32) -> Result<Option<f32>> {
		let sessions = self.sessions_for(target)?;
		let mut landed = None;

		for session in &sessions {
			let moved = unsafe { session.GetMasterVolume() }.and_then(|current| {
				let next = (current + delta).clamp(0.0, 1.0);
				unsafe { session.SetMasterVolume(next, std::ptr::null()) }.map(|()| next)
			});

			match moved {
				| Ok(next) => drop(landed.get_or_insert(next)),
				// the session died between the lookup and now, rebuild next time
				| Err(_) => self.cache = None,
			}
		}

		Ok(landed)
	}

	/// when the sessions disagree, one muted and one not, they are all forced
	/// to muted, which matches what the user sees in the volume mixer.
	pub(crate) fn toggle_mute(&mut self, target: &str) -> Result<Option<bool>> {
		let sessions = self.sessions_for(target)?;

		let any_unmuted = sessions
			.iter()
			.any(|session| unsafe { session.GetMute() }.is_ok_and(|muted| !muted.as_bool()));

		let mut applied = None;
		for session in &sessions {
			match unsafe { session.SetMute(any_unmuted, std::ptr::null()) } {
				| Ok(()) => drop(applied.get_or_insert(any_unmuted)),
				| Err(_) => self.cache = None,
			}
		}

		Ok(applied)
	}

	pub(crate) fn volume(&mut self, target: &str) -> Result<Option<f32>> {
		let sessions = self.sessions_for(target)?;

		Ok(sessions
			.first()
			.and_then(|session| unsafe { session.GetMasterVolume() }.ok()))
	}

	pub(crate) fn invalidate(&mut self) { self.cache = None; }

	fn sessions_for(&mut self, target: &str) -> Result<Vec<ISimpleAudioVolume>> {
		let fresh = self.cache.as_ref().is_some_and(|cache| {
			cache.target.eq_ignore_ascii_case(target) && cache.at.elapsed() < CACHE_TTL
		});

		if !fresh {
			let sessions = self.collect(target)?;
			self.cache = Some(Cache {
				target: target.to_owned(),
				sessions,
				at: Instant::now(),
			});
		}

		Ok(self
			.cache
			.as_ref()
			.map(|cache| cache.sessions.clone())
			.unwrap_or_default())
	}

	fn collect(&self, target: &str) -> Result<Vec<ISimpleAudioVolume>> {
		if target.is_empty() {
			return Ok(Vec::new());
		}

		let manager = self.session_manager()?;

		let sessions = unsafe { manager.GetSessionEnumerator() }
			.context("failed to enumerate audio sessions")?;

		let count = unsafe { sessions.GetCount() }.unwrap_or(0);

		let mut matched = Vec::new();
		for index in 0..count {
			let Ok(control) = (unsafe { sessions.GetSession(index) }) else { continue };
			let Ok(control2) = control.cast::<IAudioSessionControl2>() else { continue };

			let Ok(pid) = (unsafe { control2.GetProcessId() }) else { continue };
			let Some(name) = process_name(pid) else { continue };

			if !name.eq_ignore_ascii_case(target) {
				continue;
			}

			if let Ok(volume) = control.cast::<ISimpleAudioVolume>() {
				matched.push(volume);
			}
		}

		Ok(matched)
	}

	fn session_manager(&self) -> Result<IAudioSessionManager2> {
		let device = unsafe {
			self.enumerator
				.GetDefaultAudioEndpoint(eRender, eMultimedia)
		}
		.context("no default audio output device")?;

		unsafe { device.Activate::<IAudioSessionManager2>(CLSCTX_ALL, None) }
			.context("failed to open the audio session manager")
	}
}

/// `None` for the system sounds pseudo-session and for processes that have
/// exited.
///
/// querying limited information needs no elevation for processes in the same
/// logon session, which is exactly what owning an audio session implies. that
/// is why the mixer works against elevated targets without being elevated
/// itself.
fn process_name(pid: u32) -> Option<String> {
	if pid == 0 {
		return None;
	}

	let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }.ok()?;

	let mut buffer = [0_u16; MAX_PATH as usize];
	let mut length = buffer.len() as u32;

	let queried = unsafe {
		QueryFullProcessImageNameW(
			handle,
			PROCESS_NAME_WIN32,
			PWSTR(buffer.as_mut_ptr()),
			&raw mut length,
		)
	};

	unsafe { CloseHandle(handle) }.ok();
	queried.ok()?;

	let path = from_wide(&buffer);

	std::path::Path::new(&path)
		.file_name()
		.map(|name| name.to_string_lossy().into_owned())
}
