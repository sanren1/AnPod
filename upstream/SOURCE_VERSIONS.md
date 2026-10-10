# Upstream sources

Every external source AnPod reuses, pinned to a commit. Floating branch names
are deliberately not used (plan §2.2.1) — a rebuild must produce the same code.

---

## 1. ysbing/AudioShare — primary reuse

| Field | Value |
|---|---|
| Repository | <https://github.com/ysbing/AudioShare> |
| Pinned commit | `70db6b531db2c21b5a41a289722aa3996de67ebe` (branch `main`) |
| License | LGPL-3.0-or-later (see `LICENSES/LGPL-3.0-or-later.txt`) |
| Retrieved | 2026-10-10 |

### Files adapted into AnPod

| Upstream path | AnPod destination | Nature of change |
|---|---|---|
| `client/native/audio_capture.cpp` | `src/AnPod.Native/audio_capture.cpp` | Rewritten in place; see `PATCHES.md` |
| *(new)* | `src/AnPod.Native/audio_capture.h` | New public ABI header (upstream had none) |
| `server/src/main/java/com/ysbing/audioshare/Main.java` | `src/AnPod.AndroidServer/src/main/java/com/anpod/server/Main.java` | Package renamed, header validation added |
| `server/src/main/java/com/ysbing/audioshare/Options.java` | `src/AnPod.AndroidServer/src/main/java/com/anpod/server/Options.java` | Package renamed, length limits added |
| `server/build.gradle` | `src/AnPod.AndroidServer/build.gradle` | Payload copy target changed to `deps/server` |
| `client/lib/services/adb_service.dart` | `src/AnPod.Desktop/Services/AdbRunner.cs` + `ConnectionManager.cs` | Ported to C#, not reused at runtime |
| `client/lib/services/audio_capture.dart` | `src/AnPod.Desktop/Services/NativeAudioCapture.cs` | Ported to C#, not reused at runtime |
| `client/lib/data_source.dart` | `src/AnPod.Desktop/Services/ConnectionManager.cs` | Ported to C#, not reused at runtime |

The three Dart files are **reference implementations only**. They are not part
of the shipped application; their logic was translated so the behaviour is
familiar, but every gap listed in plan §3.4 was closed rather than reproduced.

### Files deliberately NOT reused

* `client/` Flutter UI, macOS `ScreenCaptureKit` path, l10n resources.
* The Dart FFI layer — replaced by `NativeAudioCapture`.

---

## 2. HeHang0/AudioShare — reference only

| Field | Value |
|---|---|
| Repository | <https://github.com/HeHang0/AudioShare> |
| Pinned commit | `dac02fdc7fdb04a1301bb4784c85580ab3e74e73` (branch `master`) |
| License | Apache-2.0 (see `LICENSES/Apache-2.0.txt`) |
| Retrieved | 2026-10-10 |

**No code from this repository is present in AnPod.** It was inspected for
structural guidance (WPF/NAudio layout, endpoint volume handling) only. Per plan
§2.2.4 its NAudio module was not copied, and its Android protocol was not mixed
with the ysbing server.

`LICENSES/Apache-2.0.txt` is retained so that if this ever changes, the notice
is already in place; today the file documents a dependency that is not used.

---

## 3. Android SDK Platform-Tools (ADB)

| Field | Value |
|---|---|
| Source | <https://dl.google.com/android/repository/platform-tools-latest-windows.zip> |
| Package revision | `37.0.1` (`Pkg.Revision` in `source.properties`) |
| `adb version` | `Android Debug Bridge version 1.0.41` / `Version 37.0.1-15733141` |
| Retrieved | 2026-10-10 |
| License | Android SDK terms; see `LICENSES/ADB-NOTICE.txt` |

AnPod shells out to `adb.exe` and does not link against it. The three files that
must be copied into `deps/` are `adb.exe`, `AdbWinApi.dll`, `AdbWinUsbApi.dll`.

### Verified properties of this revision

* The three-file set is **self-sufficient**. `adb.exe` was run from a directory
  containing nothing else and reported its version normally, so the newer
  platform-tools releases do not require `libwinpthread-1.dll` to be shipped
  alongside it. `deps/` therefore stays at three files, as plan §5.1 assumes.
* Sizes at this revision: `adb.exe` 8 273 560 B, `AdbWinApi.dll` 108 184 B,
  `AdbWinUsbApi.dll` 73 368 B.
* `NOTICE.txt` from the archive is kept at `deps/PLATFORM_TOOLS_NOTICE.txt` and
  is copied into the published package by `packaging/publish.ps1`.

> **Still required before release:** re-read the SDK terms for revision 37.0.1
> and confirm the redistribution clause permits bundling (plan §7.3/§P5.4).
> The version is now pinned; the terms review is a human step and is tracked in
> `LICENSES/ADB-NOTICE.txt`.
