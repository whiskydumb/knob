use std::{
	ffi::c_void,
	path::Path,
	sync::{
		OnceLock,
		mpsc::{self, SyncSender},
	},
	thread,
};

use windows::{
	Media::Control::{
		GlobalSystemMediaTransportControlsSession,
		GlobalSystemMediaTransportControlsSessionManager,
	},
	Win32::{
		Foundation::{HWND, LPARAM, WPARAM},
		System::Com::{COINIT_MULTITHREADED, CoInitializeEx},
		UI::WindowsAndMessaging::{PostMessageW, WM_APP},
	},
};

use crate::win::Discard;

/// posted to the main window once a skip has been attempted, with the outcome
/// in `wparam`. the offsets below it are the hook's command, the tray's
/// callback and the second instance's show.
pub(crate) const WM_KNOB_MEDIA: u32 = WM_APP + 4;

/// how many presses may be waiting at once. a queue this deep already means the
/// player is not answering, and holding more of them would only replay the
/// backlog once it wakes up.
const QUEUE_DEPTH: usize = 4;

type Manager = GlobalSystemMediaTransportControlsSessionManager;
type Session = GlobalSystemMediaTransportControlsSession;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Skip {
	Next,
	Previous,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Report {
	Skipped(Skip),
	/// nothing on the machine is holding a media session.
	NoSession,
	/// a session was there and would not move.
	Refused,
}

impl Report {
	const fn code(self) -> usize {
		match self {
			| Self::Skipped(Skip::Next) => 1,
			| Self::Skipped(Skip::Previous) => 2,
			| Self::NoSession => 3,
			| Self::Refused => 4,
		}
	}

	pub(crate) const fn from_code(code: usize) -> Option<Self> {
		match code {
			| 1 => Some(Self::Skipped(Skip::Next)),
			| 2 => Some(Self::Skipped(Skip::Previous)),
			| 3 => Some(Self::NoSession),
			| 4 => Some(Self::Refused),
			| _ => None,
		}
	}
}

struct Request {
	/// the window handle travels as an integer because `HWND` is not `Send`.
	window: isize,
	target: String,
	skip: Skip,
}

/// a `SyncSender` rather than a `Sender` because a static has to be `Sync`, and
/// the bound it brings is wanted anyway.
static REQUESTS: OnceLock<SyncSender<Request>> = OnceLock::new();

/// returns at once. the calling thread owns a single threaded apartment and a
/// message loop, neither of which may sit waiting on a winrt call.
pub(crate) fn skip(window: HWND, target: &str, skip: Skip) {
	let request = Request {
		window: window.0 as isize,
		target: target.to_owned(),
		skip,
	};

	// a full queue is a player that stopped answering, and a press dropped now is
	// better than the same press acted on a minute late
	REQUESTS
		.get_or_init(spawn)
		.try_send(request)
		.discard();
}

fn spawn() -> SyncSender<Request> {
	let (sender, requests) = mpsc::sync_channel(QUEUE_DEPTH);

	thread::spawn(move || {
		// multithreaded, so a completion never has to be marshalled into an
		// apartment that is busy waiting for it. the thread lives as long as the
		// process, so there is nothing here to unwind
		unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.discard();

		let mut manager = None;

		for request in requests {
			let report = serve(&mut manager, &request);
			let window = HWND(request.window as *mut c_void);

			unsafe {
				PostMessageW(Some(window), WM_KNOB_MEDIA, WPARAM(report.code()), LPARAM(0))
			}
			.discard();
		}
	});

	sender
}

/// the manager is asked for once and kept afterwards: acquiring it is the slow
/// part of the whole call and it stays good for the life of the process, while
/// the sessions behind it come and go and are re-read every time.
fn serve(manager: &mut Option<Manager>, request: &Request) -> Report {
	if manager.is_none() {
		*manager = Manager::RequestAsync()
			.and_then(|pending| pending.join())
			.ok();
	}

	let Some(manager) = manager.as_ref() else { return Report::NoSession };
	let Some(session) = session_for(manager, &request.target) else {
		return Report::NoSession;
	};

	let moved = match request.skip {
		| Skip::Next => session.TrySkipNextAsync(),
		| Skip::Previous => session.TrySkipPreviousAsync(),
	};

	match moved.and_then(|pending| pending.join()) {
		| Ok(true) => Report::Skipped(request.skip),
		| _ => Report::Refused,
	}
}

/// a plain win32 player registers its executable path or file name as its
/// application user model id, so the target usually matches one of the sessions
/// outright. a packaged player registers its package identity instead and never
/// will, which is what falling back to whatever windows calls the current
/// session is for: the same session the media keys would have driven.
fn session_for(manager: &Manager, target: &str) -> Option<Session> {
	let wanted = stem(target);

	if !wanted.is_empty()
		&& let Ok(sessions) = manager.GetSessions()
	{
		let count = sessions.Size().unwrap_or(0);

		for index in 0..count {
			let Ok(session) = sessions.GetAt(index) else { continue };
			let Ok(id) = session.SourceAppUserModelId() else { continue };

			if stem(&id.to_string_lossy()) == wanted {
				return Some(session);
			}
		}
	}

	manager.GetCurrentSession().ok()
}

/// the comparable part of a name: no directory, no extension, lower case. the
/// id can arrive as a full path, as a bare file name or as a word the player
/// picked for itself, and all three should meet `spotify.exe` in the middle.
fn stem(name: &str) -> String {
	Path::new(name)
		.file_stem()
		.map(|stem| stem.to_string_lossy().to_lowercase())
		.unwrap_or_default()
}
