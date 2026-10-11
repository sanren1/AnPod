# deps/ — local runtime dependencies

These files are generated or obtained from their upstream distributions and are excluded from Git. The Tauri portable build copies them beside `AnPod.exe`; the retained WPF fallback can also use the same runtime files.

| File | Source | Notes |
| --- | --- | --- |
| `audio_capture.dll` | `packaging/build-native.ps1` | x64 WASAPI capture from `src/AnPod.Native/` |
| `server` | `packaging/build-android.ps1` | Android `app_process` payload built from `src/AnPod.AndroidServer/` |
| `adb.exe` | Android SDK Platform-Tools | Record the exact version in `upstream/SOURCE_VERSIONS.md` |
| `AdbWinApi.dll` | Same Platform-Tools archive | Keep the version matched to `adb.exe` |
| `AdbWinUsbApi.dll` | Same Platform-Tools archive | Keep the version matched to `adb.exe` |
| `PLATFORM_TOOLS_NOTICE.txt` | Same Platform-Tools archive | Shipped as `LICENSES/PLATFORM_TOOLS_NOTICE.txt` |

Do not download the Android payload separately; build it from repository source. Do not commit runtime binaries. Before redistribution, retain the ADB archive's notice and verify the terms for the selected Platform-Tools release.

```powershell
Get-ChildItem deps\adb.exe, deps\AdbWinApi.dll, deps\AdbWinUsbApi.dll, deps\audio_capture.dll, deps\server, deps\PLATFORM_TOOLS_NOTICE.txt
```
