# Third-party notices

AnPod incorporates or depends on the components below. This file is the
human-readable index; the full licence texts live in `LICENSES/`, and the
pinned revisions plus the exact list of modified files live in
`upstream/SOURCE_VERSIONS.md` and `upstream/PATCHES.md`.

---

## 1. ysbing/AudioShare — LGPL-3.0-or-later

Used for: the Windows WASAPI capture algorithm (`src/AnPod.Native/`) and the
Android `app_process` playback service (`src/AnPod.AndroidServer/`).

```
AudioShare
Copyright (c) ysbing
Licensed under the GNU Lesser General Public License v3.0 or later.
https://github.com/ysbing/AudioShare
Pinned revision: 70db6b531db2c21b5a41a289722aa3996de67ebe
```

**This is modified LGPL source.** The modifications are enumerated in
`upstream/PATCHES.md`, and the complete corresponding source is the contents of
`src/AnPod.Native/` and `src/AnPod.AndroidServer/` in this repository.

### LGPL compliance notes for redistribution

AnPod is a personal-use tool and is not currently distributed. **If it is ever
distributed, the following must be reviewed first** (plan §2.2.5, §11):

1. The modified LGPL sources above must be made available, or the build scripts
   must let the recipient rebuild and relink the LGPL portions.
2. Because `audio_capture.dll` is loaded dynamically at runtime (not statically
   linked), the "work that uses the library" is AnPod.exe and the "library" is
   `audio_capture.dll`; the recipient must be able to replace the library and
   still run. The current `NativeAudioCapture` implementation satisfies this —
   it resolves every export dynamically from the DLL file next to the
   executable, with no static link and no signature check.
3. No LGPL code is inlined into the managed assembly.
4. This notice must ship with the binaries.

---

## 2. HeHang0/AudioShare — Apache-2.0

**Not used.** Inspected for structural reference only; no code was copied. The
licence text is retained in `LICENSES/Apache-2.0.txt` so the notice is in place
if that ever changes.

---

## 3. Android SDK Platform-Tools (`adb`)

Redistributed as a runtime dependency in `deps/` (not part of this repository).
See `LICENSES/ADB-NOTICE.txt` for the terms that must be verified before any
release.

---

## 4. .NET 8 runtime and Windows Desktop runtime

AnPod targets `net8.0-windows` and is published as a framework-dependent
application. The .NET runtime is distributed by Microsoft under the MIT licence
and its own third-party notices, which are not bundled here because AnPod does
not redistribute the runtime.

---

## 5. No other dependencies

The Windows application has **no NuGet package references**. WPF and Windows
Forms are part of the .NET SDK's Windows Desktop workload. This is deliberate:
plan §8 forbids unnecessary third-party dependencies, and every problem solved
here (MVVM plumbing, logging, settings) was smaller than the dependency that
would have solved it.
