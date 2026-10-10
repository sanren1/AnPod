# Third-party notice template for a released package

Copy this file next to the binaries whenever AnPod is distributed, fill in the
blanks, and ship it together with `LICENSES/` and `upstream/`.

---

## AnPod <VERSION> — third-party components

Released: <YYYY-MM-DD>

### 1. ysbing/AudioShare — LGPL-3.0-or-later (MODIFIED)

* Revision used: `70db6b531db2c21b5a41a289722aa3996de67ebe`
* Modified files: `src/AnPod.Native/audio_capture.cpp`, `audio_capture.h`,
  `src/AnPod.AndroidServer/src/main/java/com/anpod/server/Main.java`,
  `Options.java`
* Change list: see `upstream/PATCHES.md`
* Corresponding source: `src/AnPod.Native/` and `src/AnPod.AndroidServer/`
* The LGPL component `audio_capture.dll` is loaded dynamically at runtime and can
  be replaced by the recipient without rebuilding `AnPod.exe`.

### 2. Android SDK Platform-Tools (`adb`)

* Version shipped: <PLATFORM-TOOLS VERSION>  ← must not be left blank
* `NOTICE.txt` from the platform-tools archive is included at
  `LICENSES/platform-tools-NOTICE.txt`
* Terms reviewed on: <YYYY-MM-DD>

### 3. .NET 8 Desktop Runtime

* Distributed by Microsoft under the MIT licence; not redistributed inside this
  package (framework-dependent publish).

### 4. HeHang0/AudioShare — Apache-2.0

* Not used in this release. Licence text included for completeness.

---

## Verification checklist before shipping

- [ ] `upstream/SOURCE_VERSIONS.md` has no "not pinned" entries.
- [ ] Every file listed in `upstream/PATCHES.md` matches the shipped binary
      (rebuild from the recorded revisions and compare hashes).
- [ ] `LICENSES/` contains LGPL-3.0-or-later, Apache-2.0, ADB notice and the
      platform-tools `NOTICE.txt`.
- [ ] This notice is present in the package root.
- [ ] `deps/` binaries were produced by `packaging/` scripts, not downloaded
      from a third party (plan §7.1).
