# Repopack

Repopack is a local-first repository inspection and build-preparation app. The first milestone accepts a public GitHub repository URL or a local ZIP, downloads/extracts it into the app cache, and detects common project types.

> **Safety note:** project build scripts are executable code. Repopack should show the proposed commands and ask for approval before running them. This first milestone does not execute project build scripts.

## Planned stack

- **Tauri 2 + Rust:** local files, archive extraction, native process/build orchestration.
- **React + TypeScript + Vite:** dashboard UI and live job logs.
- **Tauri dialog plugin:** native ZIP file picker.
- **Platform adapters:** Node/Electron/Tauri, Flutter, Android Gradle, Python, and static web projects.

## Requirements

- Node.js 20 or newer and npm
- Rust toolchain
- Native prerequisites for Tauri on your OS
- Android Studio / Android SDK only when developing the Android target

## Run the desktop development app

```sh
npm install
npm run tauri dev
```

Build the frontend:

```sh
npm run build
```

For Android, first initialize the Tauri Android project and install the Tauri Android prerequisites, then run `npm run tauri android dev`. See the official Tauri mobile guide for your host OS.

## Initial folder structure

```text
Repopack/
├── src/
│   ├── App.tsx
│   ├── main.tsx
│   └── styles.css
├── src-tauri/
│   ├── capabilities/default.json
│   ├── src/lib.rs
│   ├── src/main.rs
│   ├── build.rs
│   ├── Cargo.toml
│   └── tauri.conf.json
├── index.html
├── package.json
├── tsconfig.json
└── vite.config.ts
```

## Current milestone

- [x] Enter a public GitHub URL
- [x] Select a local ZIP file
- [x] Download/extract into a private app-cache job folder
- [x] Defend ZIP extraction against path traversal, symlinks, oversized archives, and excessive file counts
- [x] Detect common project markers and show a recommendation
- [x] Emit progress messages to the UI log
- [ ] Approved, allow-listed build profiles and cancellation
- [ ] Host-specific installer/package generation
- [ ] Android build runner constraints and device testing
- [ ] Signed/reproducible releases

## Important scope distinction

A source repository cannot always be turned into an app by one generic command. The supported output depends on its framework and required toolchain. Native Windows packages are built on Windows, Linux packages on Linux, and Android APKs need Android build tools. Android itself does not provide Node, Python, Flutter, or Android SDK toolchains by default; local build execution on the phone needs a separate, carefully designed strategy.
