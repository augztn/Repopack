# Repopack

Repopack is a local-first repository inspection and build-preparation app. The first milestone accepts a public GitHub repository URL or a local ZIP, downloads/extracts it into the app cache, and detects common project types.

> **Safety note:** repository build scripts are executable code and may be unsafe. Repopack asks for explicit approval before running a small allow-list of build profiles. Only build repositories you trust; approval is not a sandbox.

## Planned stack

- **Tauri 2 + Rust:** local files, archive extraction, native process/build orchestration.
- **React + TypeScript + Vite:** dashboard UI and live job logs.
- **Tauri dialog plugin:** native ZIP file picker.
- **Platform adapters:** Node/Electron/Tauri, Flutter, Android Gradle, Python/PyInstaller, Rust/Cargo, and static web projects.

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
- [x] Build cancellation button and a 30-minute main-process timeout (descendant-process cleanup remains platform-dependent)
- [x] Discover common package artifacts after successful builds, show their paths and sizes, open output folders, and export artifacts
- [ ] Artifact signing, install testing, and deeper host-specific package verification
- [ ] Android build runner constraints and device testing
- [ ] Signed/reproducible releases

## Important scope distinction

A source repository cannot always be turned into an app by one generic command. The supported output depends on its framework and required toolchain. Native Windows packages are built on Windows, Linux packages on Linux, and Android APKs need Android build tools. Android itself does not provide Node, Python, Flutter, or Android SDK toolchains by default; local build execution on the phone needs a separate, carefully designed strategy.


## Current build-runner limitations

- The `npm-install` profile uses `npm ci` when a package lock exists, otherwise `npm install`; dependency lifecycle scripts may execute arbitrary code, so review the repository before approving. The UI also exposes build cancellation and a 30-minute timeout for the main process, but subprocess trees are not yet fully sandboxed or guaranteed to terminate.
- The Electron build profile invokes `npm run build`; the separate Electron packaging profile requires a declared `dist`, `make`, or `package` script. Both depend on the repository's own packaging configuration.
- Python packaging supports a root `main.py`, `app.py`, or `run.py` entry point with PyInstaller installed; more complex project entry points need configuration.
- Tool detection is informational; a tool being present does not guarantee the project is compatible with the selected profile.
- Build subprocesses have a 30-minute main-process timeout, but this is not a complete sandbox and does not guarantee every descendant process is terminated. Do not use this preview with untrusted projects.
- Artifact discovery is best-effort and scans common output file types; it does not prove that a package is installable or signed. Artifacts can be exported through the native save dialog.
- Python dependency setup and PyInstaller packaging require Python 3 and the relevant project dependencies. Rust projects can use `cargo build --release`.
- Android APK creation needs Android SDK/Java/Gradle or Flutter tooling as appropriate. Available targets depend on the tools installed on the desktop host.


## Dependency preparation and export

The `Prepare Node dependencies` profile requires explicit approval and uses `npm ci` when a lockfile is present, or `npm install` otherwise. Both may run lifecycle scripts and download code. Build execution has a 30-minute timeout for the main process; this is not a security sandbox and descendant processes may need platform-specific cleanup. Recognized artifacts can be exported through the native save dialog or viewed in their output folder.

## Download cross-platform test installers

The `Cross-platform test bundle` workflow builds Repopack natively on Windows, macOS, and Linux, then combines the resulting installers/bundles into one ZIP. It runs on pushes to `main`, pull requests targeting `main`, and manual workflow dispatch.

To download it, open the repository's **Actions** tab, select **Cross-platform test bundle**, open a successful run, and download the `Repopack-cross-platform-test-bundle` artifact. The ZIP contains a folder for each desktop platform plus testing instructions. GitHub keeps these test artifacts for 14 days.

These are unsigned test builds; they are separate native packages, not one executable that runs on every OS. The workflow also attempts an Android APK build using Tauri's Android project initialization and Android SDK/NDK toolchain. Check the workflow run logs to confirm that the first Android build succeeds.

## Diagnostics and crash investigation

Repopack writes persistent diagnostic messages to its app data directory under `diagnostics/repopack.log`. The Process output panel includes **Save log** for a custom text/log destination and **Export logs ZIP** for a shareable diagnostic bundle. The log records frontend JavaScript errors, unhandled promise rejections, job events, and whether the previous app session appears to have ended unexpectedly.

If the app fails before its UI opens, it cannot display its export controls. Start it again to record an unclean previous session, or collect platform-level logs (for example, Android `adb logcat`) and attach those to your issue. Review diagnostic files before sharing: messages can include local paths and error details. The ZIP does not automatically include source ZIPs or arbitrary personal files.


## Automatic build profile and tool detection

After source inspection, Repopack selects the profile suggested by detected project markers and automatically scans the local machine for relevant toolchains (Node.js/npm, Python, Cargo, Flutter, Java, Gradle, ADB, Android SDK Manager, and Android SDK). The build panel shows which tools are available and lists expected tools for the selected profile. You can still change the suggested profile manually. Builds are not launched automatically: review the profile and explicitly approve execution because repository scripts may run arbitrary code.


## Python desktop projects (including DroidLab)

Repopack recognizes Python GUI repositories whose entry point is nested at `src/app.py`. For DroidLab-shaped projects containing both `src/app.py` and `scripts/build-linux.sh`, it recommends the **DroidLab — Linux AppImage** profile and scans the project-specific prerequisites: Tkinter, PyInstaller, appimagetool, wget, tar, xz and Bash. The profile validates its Linux-only requirement and reports missing dependencies before invoking the repository's build script. AppImage packaging is not supported from a Windows/macOS host; use a Linux build host for this target.
