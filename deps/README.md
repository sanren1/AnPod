# deps/ — runtime dependencies (not source-controlled)

Everything in this folder is **produced by another toolchain** and is deliberately
absent from version control. `AnPod.Desktop.csproj` copies each file next to
`AnPod.exe` when it exists, so a clean checkout still compiles and starts.

| File | Produced by | Notes |
|---|---|---|
| `audio_capture.dll` | `packaging/build-native.ps1` | x64 WASAPI capture, from `src/AnPod.Native/` |
| `server` | `packaging/build-android.ps1` | Android payload (APK used as a classes.dex container) |
| `adb.exe` | Android SDK Platform-Tools | must be a trusted, recorded version |
| `AdbWinApi.dll` | Android SDK Platform-Tools | ships with `adb.exe` |
| `AdbWinUsbApi.dll` | Android SDK Platform-Tools | ships with `adb.exe` |

## Rules

1. **Do not download the Android payload from the internet.** It must be built
   from `src/AnPod.AndroidServer/` so the version is known and reproducible
   (plan §7.1).
2. Record the platform-tools version in `upstream/SOURCE_VERSIONS.md` before any
   redistribution.
3. Verify `server` is a zip whose root contains `classes.dex`:
   `unzip -l deps/server | grep classes.dex`
4. Do not commit these binaries. `.gitignore` excludes everything here except
   this file.

## Quick check

```bash
# Are all five present?
ls deps/{audio_capture.dll,adb.exe,AdbWinApi.dll,AdbWinUsbApi.dll,server}

# Is the DLL the right architecture?
dumpbin /headers deps/audio_capture.dll | grep -i machine   # expect: x64
```
