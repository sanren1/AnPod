# Tauri portable release notice checklist

The Tauri portable packaging script copies the maintained [`THIRD_PARTY_NOTICES.md`](../THIRD_PARTY_NOTICES.md), `LICENSES/`, and `upstream/` into every package. Review these files whenever dependencies or runtime payloads change.

Before publishing a release:

- Confirm the exact Android SDK Platform-Tools version is recorded in `upstream/SOURCE_VERSIONS.md`.
- Include the matching upstream `NOTICE.txt` as `LICENSES/PLATFORM_TOOLS_NOTICE.txt`.
- Keep the modified LGPL source and patch list available in `src/` and `upstream/`.
- Check the package contains the notice, licence files and source-version records.
- Do not describe the portable package as self-contained with respect to WebView2; that Microsoft runtime is a system prerequisite.

`packaging/publish.ps1` remains for the optional WPF fallback. Its .NET runtime notes apply only to packages made with that legacy entry point, not the Tauri release.
