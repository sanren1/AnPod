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

The Windows audio DLL is dynamically loaded from beside `AnPod.exe`; it is not
statically linked into the Rust desktop executable. **Before redistributing a
modified build:**

1. The modified LGPL sources above must be made available, or the build scripts
   must let the recipient rebuild and relink the LGPL portions.
2. Because `audio_capture.dll` is loaded dynamically at runtime (not statically
   linked), the "work that uses the library" is AnPod.exe and the "library" is
   `audio_capture.dll`; the recipient must be able to replace the library and
   still run. The current Rust `AudioActor` implementation satisfies this —
   it resolves every export dynamically from the DLL file next to the
   executable, with no static link and no signature check.
3. Include this notice and the applicable licence files with the binaries.

---

## 2. HeHang0/AudioShare — Apache-2.0

**Not used.** Inspected for structural reference only; no code was copied. The
licence text is retained in `LICENSES/Apache-2.0.txt` so the notice is in place
if that ever changes.

---

## 3. Android SDK Platform-Tools (`adb`)

Redistributed as a runtime dependency in the portable package. The package
includes the `NOTICE.txt` supplied with the exact Platform-Tools archive as
`LICENSES/PLATFORM_TOOLS_NOTICE.txt`. Keep that upstream notice with `adb.exe`
and verify the archive's current distribution terms before publishing.

## 4. Tauri desktop and frontend dependencies

The shipped Windows application uses Tauri 2 (MIT or Apache-2.0), Rust crates
listed in `src-tauri/Cargo.lock`, React and ReactDOM (MIT), `lucide-react`
(ISC, with Feather-derived icon attribution), and `qrcode.react` (ISC; it also
bundles Project Nayuki's QR Code Generator under MIT). Their notices and the
MIT, ISC, Apache-2.0 and LGPL-3.0-or-later licence texts are included in
`LICENSES/` where available. Each Rust crate retains the licence terms recorded
in its published package; see the lockfile for exact versions.

The WebView2 Runtime is a Microsoft system prerequisite and is not bundled by
the portable package. The WPF fallback under `src/AnPod.Desktop/` is an optional
.NET 8 application and is not part of the Tauri portable package.

## 5. Project code

AnPod's own code has no declared project-wide licence. Third-party notices do
not grant a licence to the rest of the project.
