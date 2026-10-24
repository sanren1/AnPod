# AnPod frontend

React and TypeScript interface served locally by the Tauri desktop application. The desktop commands and application lifecycle are implemented in `../src-tauri/`.

## Build and run

```powershell
npm ci
npm run build
npm run tauri -- dev
```

`npm run build` type-checks the app and writes Vite output to `dist/`. The Tauri development command starts Vite through its `beforeDevCommand` setting.
