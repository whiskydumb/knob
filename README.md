# knob

a tiny sound mixer to adjust the volume of programs using keyboard buttons or knob

the knob's media keys are swallowed by a low-level keyboard hook and translated
into volume changes on one application's audio session, so turning it moves
spotify, discord or the browser instead of the system volume.

## features

- per-application volume from the knob, system volume left alone
- picks up whatever is currently playing, or takes an executable name by hand
- a smaller step near the bottom of the range, where a whole percent is a jump
- configurable direction, step size and press action
- falls back to the system volume when the target is silent
- lives in the tray, optional autostart, no elevation required

## how it works

- **interception**: a `WH_KEYBOARD_LL` hook takes `VK_VOLUME_UP`, `VK_VOLUME_DOWN`
  and `VK_MEDIA_PLAY_PAUSE`, posts a message to the main window and returns a
  non-zero value, which is what keeps windows from moving the system volume and
  from showing the volume osd. the callback does nothing else: it has a few hundred
  milliseconds before windows evicts it, so the real work happens in the window
  procedure.
- **mixing**: `IAudioSessionManager2` enumerates the sessions on the default
  output device, `IAudioSessionControl2::GetProcessId` resolves each one to an
  executable name, and `ISimpleAudioVolume` moves the level. a target that owns
  several sessions, as every chromium-based browser does, gets all of them moved
  together.
- **stepping**: the scalar core audio exposes is linear in amplitude, so the same
  percent is a far bigger jump at 5% than at 80%. below the configured threshold
  the detent shrinks to the fine step. the threshold belongs to the fine zone on
  the way down and to the coarse one on the way up, which makes every detent
  exactly reversible: 10% lowers to 9.5%, 9.5% raises back to 10%, and 10% raises
  to 15%. each write is snapped to a tenth of a percentage point so a long turn
  cannot accumulate rounding error.
- **skipping**: the media session api is a separate subsystem from the mixer's
  and has no notion of a process, so the target is matched against the
  application user model id its player registered. that is the executable path
  for some players and a product name for others, so a miss is expected and falls
  back to whatever windows considers the current session, which is the session
  the media keys would have driven anyway. the call is winrt and cannot be waited
  on from the window's apartment, so it runs on a worker thread and posts the
  outcome back.
- **falling back**: while the target is not playing anything the hook is disarmed
  and the keys pass through untouched, so closing spotify turns the knob back into
  a plain system volume knob instead of a dead one.

a bluetooth or hid consumer-control knob produces no hardware key event of its
own. a system component reads the hid report and synthesizes one, so every detent
arrives at the hook flagged as injected with a zero scan code — which is why knob
does not filter that flag.

## build

```
cargo build --release
```

`just dist` produces the shipping build with fat lto, `just lint` runs formatting,
spelling and clippy. formatting needs the nightly toolchain, which `just setup`
installs.

the build script embeds `knob.manifest` and `assets/knob.ico` through the msvc
linker, so neither a resource compiler nor a build dependency is involved.
replacing the icon is a matter of dropping a different multi-size `.ico` at that
path.

## use

the window is the whole configuration surface:

| control | what it does |
| --- | --- |
| target application | the executable whose volume the knob drives. the dropdown lists everything currently holding an audio session, and the field is editable, so an application that is silent right now can be typed in by hand |
| raise / lower volume key | which physical direction raises the volume; picking one flips the other |
| volume step | how much one detent moves the level, 1-25% |
| fine step | the smaller step used inside the fine zone, 0.1-2.5% |
| fine step below | the level under which the knob switches to the fine step, or off |
| knob press | mute the target, cycle to the next remembered target, skip a track forward or back, pass the key through to whatever is focused, or swallow it. one press does one thing, so the direction of the skip is picked here rather than from a gesture |
| launch on startup | writes `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` |
| save | applies everything and writes the config |

settings live in `%APPDATA%\knob\config.toml`. closing the window leaves the
program running in the tray; the tray menu has **suspend interception**, which
physically removes the hook, and **exit**.

## elevation

changing another process's volume needs no elevation, even when that process runs
elevated: audio sessions are enumerated per logon session and uipi does not apply
to core audio.

the one thing elevation buys is interception while an elevated window has focus,
because a hook installed by a normal process never sees those keystrokes. the
manifest therefore asks for `asInvoker` and startup registration goes through the
run key. running the executable as administrator covers elevated windows too, but
then autostart has to move to a scheduled task with highest privileges, since the
run key does not launch elevated entries.

## anti-cheat

`WH_KEYBOARD_LL` is a documented user-mode api. it injects no dll, reads no foreign
memory, and knob never synthesizes input with `SendInput`, which is the pattern
anti-cheats actually look for in macro tools. track skipping goes through the
media session api rather than a synthetic media key, so that stays true.

that said, some anti-cheats dislike the combination of a global hook and an
elevated process and may refuse to start until the program is closed. the tray
menu's **suspend interception** exists for exactly that: one click and nothing of
ours is left in the keyboard chain.

## license

mit
