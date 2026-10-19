# Repopack

Repopack is a local-first repository inspection and build-preparation app. The first milestone accepts a public GitHub repository URL or a local ZIP, downloads/extracts it into the app cache, and detects common project types.

> **Safety note:** repository build scripts are executable code and may be unsafe. Repopack asks for explicit approval before running a small allow-list of build profiles. Only build repositories you trust; approval is not a sandbox.

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
- [x] Explicit approval and allow-listed build profiles for selected targets
- [ ] Process cancellation and timeouts
- [x] Discover common package artifacts after successful builds and show their paths, sizes, and output-folder actions
- [ ] Artifact signing, install testing, and deeper host-specific package verification
- [ ] Android build runner constraints and device testing
- [ ] Signed/reproducible releases

## Important scope distinction

A source repository cannot always be turned into an app by one generic command. The supported output depends on its framework and required toolchain. Native Windows packages are built on Windows, Linux packages on Linux, and Android APKs need Android build tools. Android itself does not provide Node, Python, Flutter, or Android SDK toolchains by default; local build execution on the phone needs a separate, carefully designed strategy.


## Current build-runner limitations

- Build profiles do not automatically install dependencies. Review the repository and lockfile before installing dependencies.
- The Electron profile currently invokes the repository's `npm run build` script; that script may not create a distributable installer.
- Tool detection is informational; a tool being present does not guarantee the project is compatible with the selected profile.
- Build subprocesses currently have no cancel button or enforced execution timeout. Do not use this preview with untrusted projects.
- Artifact discovery is best-effort and scans common output file types; it does not prove that a package is installable or signed.
- Android APK creation needs Android SDK/Java/Gradle or Flutter tooling as appropriate. Available targets depend on the tools installed on the desktop host.
