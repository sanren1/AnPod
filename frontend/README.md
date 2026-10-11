# AnPod frontend

React and TypeScript interface served locally by the Tauri desktop application. The desktop commands and application lifecycle are implemented in `../src-tauri/`.

## Build and run

```powershell
npm ci
npm run build
npm run tauri -- dev
```

`npm run build` type-checks the app and writes Vite output to `dist/`. The Tauri development command starts Vite through its `beforeDevCommand` setting.

## Optional browser check

The UI check uses plain Node.js, Playwright and a local browser; it does not add a test runner or production dependency. Start Vite first, then from the repository root run:

```powershell
cd frontend
npm run dev
```

In another terminal:

```powershell
node tests/AnPod.Frontend.Check.cjs
```

Install Playwright locally only if needed:

```powershell
cd frontend
npm install --no-save --package-lock=false playwright
npx playwright install chromium
```

Or point the check at an existing Playwright module, browser executable, or dev-server URL with `ANPOD_PLAYWRIGHT_MODULE`, `ANPOD_BROWSER_EXECUTABLE`, and `ANPOD_UI_URL`. Results and screenshots are written to `.tmp-frontend-check/`.
