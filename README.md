# knob

a tiny sound mixer to adjust the volume of programs using keyboard buttons or knob

the knob's media keys are swallowed by a low-level keyboard hook and translated
into volume changes on one application's audio session, so turning it moves
spotify, discord or the browser instead of the system volume.

## features

- per-application volume from the knob, system volume left alone
- picks up whatever is currently playing, or takes an executable name by hand
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
| knob press | mute the target, cycle to the next remembered target, pass the key through to whatever is focused, or swallow it |
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
anti-cheats actually look for in macro tools.

that said, some anti-cheats dislike the combination of a global hook and an
elevated process and may refuse to start until the program is closed. the tray
menu's **suspend interception** exists for exactly that: one click and nothing of
ours is left in the keyboard chain.

## license

mit
