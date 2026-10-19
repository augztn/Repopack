use std::{fs, path::{Path, PathBuf}, process::{Command as OsCommand, Stdio}, sync::Mutex, time::Duration};
use tauri::{AppHandle, Emitter, Manager};
use tokio::{io::{AsyncBufReadExt, BufReader}, process::Command, sync::oneshot, time};

#[derive(Clone, serde::Serialize)]
struct RunnerLog { level: String, message: String }

#[derive(Default)]
pub struct BuildJobState { cancellation: Mutex<Option<oneshot::Sender<()>>>, active: Mutex<bool> }

#[tauri::command]
pub fn cancel_build(app: AppHandle, state: tauri::State<'_, BuildJobState>) -> Result<(), String> {
    let sender = state.cancellation.lock().map_err(|_| "Build state is unavailable.")?.take();
    match sender {
        Some(sender) => { let _ = sender.send(()); emit(&app, "warning", "Cancellation requested."); Ok(()) },
        None => Err("There is no active build to cancel.".into()),
    }
}

fn emit(app: &AppHandle, level: &str, message: impl Into<String>) {
    let _ = app.emit("job-log", RunnerLog { level: level.into(), message: message.into() });
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCheck {
    pub tool: String,
    pub available: bool,
    pub version: Option<String>,
    pub message: String,
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildResult {
    pub profile: String,
    pub exit_code: Option<i32>,
    pub success: bool,
    pub message: String,
    pub artifacts: Vec<BuildArtifact>,
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildArtifact {
    pub name: String,
    pub path: String,
    pub size_bytes: u64,
    pub kind: String,
}

#[cfg(unix)]
fn is_executable_file(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    fs::metadata(path).map(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0).unwrap_or(false)
}

fn discover_artifacts(root: &Path) -> Vec<BuildArtifact> {
    let mut found = Vec::new();
    let mut stack: Vec<(PathBuf, usize)> = vec![(root.to_path_buf(), 0)];
    let mut visited = 0usize;
    while let Some((directory, depth)) = stack.pop() {
        if depth > 10 || visited >= 20_000 { break; }
        let entries = match fs::read_dir(&directory) { Ok(entries) => entries, Err(_) => continue };
        for entry in entries.flatten() {
            visited += 1;
            if visited >= 20_000 { break; }
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            let kind = match entry.file_type() { Ok(kind) => kind, Err(_) => continue };
            if kind.is_symlink() { continue; }
            if kind.is_dir() {
                if !matches!(name.as_str(), ".git" | "node_modules" | ".dart_tool" | ".idea" | ".vscode") {
                    stack.push((path, depth + 1));
                }
                continue;
            }
            if !kind.is_file() { continue; }
            let lower = name.to_ascii_lowercase();
            let ext = path.extension().and_then(|v| v.to_str()).unwrap_or("").to_ascii_lowercase();
            let artifact_kind = if lower.ends_with(".tar.gz") { Some("TAR.GZ") } else {
                match ext.as_str() {
                    "apk" => Some("APK"), "aab" => Some("AAB"), "exe" => Some("EXE"),
                    "msi" => Some("MSI"), "appimage" => Some("AppImage"), "deb" => Some("DEB"),
                    "rpm" => Some("RPM"), "dmg" => Some("DMG"), "pkg" => Some("PKG"),
                    "appx" => Some("APPX"), "msix" => Some("MSIX"), "zip" => Some("ZIP"),
                    "jar" => Some("JAR"), "ipa" => Some("IPA"), "bin" => Some("BIN"),
                    _ => None,
                }
            };
            if let Some(kind) = artifact_kind {
                let size_bytes = entry.metadata().map(|m| m.len()).unwrap_or(0);
                found.push(BuildArtifact { name, path: path.to_string_lossy().into_owned(), size_bytes, kind: kind.into() });
            }
        }
    }
    #[cfg(unix)]
    {
        for relative_dir in ["target/release", "dist"] {
            let output_dir = root.join(relative_dir);
            if let Ok(entries) = fs::read_dir(&output_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.extension().is_none() && is_executable_file(&path) {
                        if let Ok(metadata) = fs::metadata(&path) {
                            let artifact_path = path.to_string_lossy().into_owned();
                            if !found.iter().any(|artifact| artifact.path == artifact_path) {
                                found.push(BuildArtifact {
                                    name: entry.file_name().to_string_lossy().into_owned(),
                                    path: artifact_path,
                                    size_bytes: metadata.len(),
                                    kind: "BIN".into(),
                                });
                            }
                        }
                    }
                }
            }
        }
    }
    found.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    found
}

#[tauri::command]
pub fn open_artifact_folder(app: AppHandle, artifact_path: String) -> Result<(), String> {
    let path = Path::new(&artifact_path).canonicalize().map_err(|e| format!("Artifact path is unavailable: {e}"))?;
    let cache = app.path().app_cache_dir().map_err(|e| format!("Cannot resolve app cache: {e}"))?
        .canonicalize().map_err(|e| format!("Cannot resolve app cache: {e}"))?;
    if !path.starts_with(&cache) || !path.is_file() {
        return Err("Only files inside Repopack's private workspace can be opened this way.".into());
    }
    let folder = path.parent().ok_or("Artifact has no parent folder.")?;
    #[cfg(target_os = "windows")]
    let result = OsCommand::new("explorer").arg(folder).spawn();
    #[cfg(target_os = "macos")]
    let result = OsCommand::new("open").arg(folder).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let result = OsCommand::new("xdg-open").arg(folder).spawn();
    result.map(|_| ()).map_err(|e| format!("Could not open artifact folder: {e}"))
}


#[tauri::command]
pub fn export_artifact(app: AppHandle, artifact_path: String, destination_path: String) -> Result<(), String> {
    let source = Path::new(&artifact_path).canonicalize()
        .map_err(|e| format!("Artifact path is unavailable: {e}"))?;
    let cache = app.path().app_cache_dir().map_err(|e| format!("Cannot resolve app cache: {e}"))?
        .canonicalize().map_err(|e| format!("Cannot resolve app cache: {e}"))?;
    if !source.starts_with(&cache) || !source.is_file() {
        return Err("Only files inside Repopack's private workspace can be exported.".into());
    }
    let destination = PathBuf::from(destination_path);
    if destination.exists() && destination.is_dir() {
        return Err("Choose a destination filename, not a folder.".into());
    }
    if let Some(parent) = destination.parent() {
        if !parent.is_dir() {
            return Err("The destination folder does not exist.".into());
        }
    }
    fs::copy(&source, &destination).map_err(|e| format!("Could not export artifact: {e}"))?;
    Ok(())
}

fn approved_workspace(app: &AppHandle, root: &str) -> Result<PathBuf, String> {
    let path = Path::new(root).canonicalize().map_err(|e| format!("Workspace path is unavailable: {e}"))?;
    let cache = app.path().app_cache_dir().map_err(|e| format!("Cannot resolve app cache: {e}"))?
        .canonicalize().map_err(|e| format!("Cannot resolve app cache: {e}"))?;
    if !path.starts_with(&cache) || !path.is_dir() {
        return Err("Builds are limited to a project directory inside Repopack's private workspace.".into());
    }
    Ok(path)
}

fn check_android_sdk() -> ToolCheck {
    let sdk_path = std::env::var_os("ANDROID_HOME")
        .or_else(|| std::env::var_os("ANDROID_SDK_ROOT"))
        .map(PathBuf::from);
    match sdk_path.filter(|path| path.is_dir()) {
        Some(path) => ToolCheck {
            tool: "Android SDK".into(),
            available: true,
            version: Some(path.to_string_lossy().into_owned()),
            message: "SDK directory found".into(),
        },
        None => ToolCheck {
            tool: "Android SDK".into(),
            available: false,
            version: None,
            message: "Set ANDROID_HOME or ANDROID_SDK_ROOT to an existing SDK directory".into(),
        },
    }
}

async fn check_command(command: &str, args: &[&str], tool: &str) -> ToolCheck {
    let mut process = Command::new(command);
    process.args(args).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
    match time::timeout(Duration::from_secs(8), process.output()).await {
        Ok(Ok(output)) if output.status.success() => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            let version = stdout.lines().next().filter(|line| !line.trim().is_empty()).or_else(|| stderr.lines().next()).unwrap_or("").trim().to_string();
            ToolCheck { tool: tool.into(), available: true, version: if version.is_empty() { None } else { Some(version) }, message: "Available".into() }
        }
        Ok(Ok(output)) => {
            let error = String::from_utf8_lossy(&output.stderr).lines().next().unwrap_or("Command returned an error").trim().to_string();
            ToolCheck { tool: tool.into(), available: false, version: None, message: if error.is_empty() { "Command failed".into() } else { error } }
        }
        Ok(Err(error)) => ToolCheck { tool: tool.into(), available: false, version: None, message: format!("Not available: {error}") },
        Err(_) => ToolCheck { tool: tool.into(), available: false, version: None, message: "Version check timed out after 8 seconds".into() },
    }
}

fn executable_available(command: &str) -> bool {
    let path_value = match std::env::var_os("PATH") { Some(value) => value, None => return false };
    for dir in std::env::split_paths(&path_value) {
        let candidate = dir.join(command);
        if candidate.is_file() { return true; }
        #[cfg(windows)]
        {
            for extension in ["exe", "cmd", "bat"] {
                if dir.join(format!("{command}.{extension}")).is_file() { return true; }
            }
        }
    }
    false
}

#[tauri::command]
pub async fn check_project_build_tools(root_path: String) -> Result<Vec<ToolCheck>, String> {
    let root = PathBuf::from(&root_path).canonicalize().map_err(|e| format!("Project workspace is unavailable: {e}"))?;
    if !root.is_dir() { return Err("Project root is not a directory.".into()); }

    let checks = check_build_tools().await;
    let is_droidlab = root.join("scripts/build-linux.sh").is_file() && root.join("src/app.py").is_file();
    if !is_droidlab { return Ok(checks); }

    let mut all = checks;
    if cfg!(target_os = "linux") {
        let python = "python3";
        for (name, available, message) in [
            ("Tkinter", OsCommand::new(python).args(["-c", "import tkinter"]).stdout(Stdio::null()).stderr(Stdio::null()).status().map(|s| s.success()).unwrap_or(false), "Python Tkinter module"),
            ("TkinterDnD", OsCommand::new(python).args(["-c", "import tkinterdnd2"]).stdout(Stdio::null()).stderr(Stdio::null()).status().map(|s| s.success()).unwrap_or(false), "Python tkinterdnd2 dependency from requirements.txt"),
            ("PyInstaller", OsCommand::new(python).args(["-m", "PyInstaller", "--version"]).stdout(Stdio::null()).stderr(Stdio::null()).status().map(|s| s.success()).unwrap_or(false), "Python packaging module"),
        ] {
            all.push(ToolCheck { tool: name.into(), available, version: None, message: message.into() });
        }
        for (name, available) in [
            ("appimagetool", executable_available("appimagetool")),
            ("wget", executable_available("wget")),
            ("tar", executable_available("tar")),
            ("xz", executable_available("xz")),
            ("Bash", executable_available("bash")),
        ] {
            all.push(ToolCheck { tool: name.into(), available, version: None, message: if available { "Executable found on PATH".into() } else { "Not found on PATH".into() } });
        }
    } else {
        all.push(ToolCheck {
            tool: "DroidLab AppImage target".into(),
            available: false,
            version: None,
            message: "AppImage packaging requires a Linux build host; this host cannot create a Linux AppImage.".into(),
        });
    }
    Ok(all)
}

#[tauri::command]
pub async fn check_build_tools() -> Vec<ToolCheck> {
    // Run independent probes concurrently so a slow tool cannot block the entire report.
    let (node, npm, python, cargo, flutter, java, gradle, adb, sdkmanager) = tokio::join!(
        check_command("node", &["--version"], "Node.js"),
        check_command("npm", &["--version"], "npm"),
        check_command("python", &["--version"], "Python"),
        check_command("cargo", &["--version"], "Rust / Cargo"),
        check_command("flutter", &["--version"], "Flutter"),
        check_command("java", &["-version"], "Java"),
        check_command("gradle", &["--version"], "Gradle"),
        check_command("adb", &["version"], "Android Debug Bridge"),
        check_command("sdkmanager", &["--version"], "Android SDK Manager"),
    );
    let mut checks = vec![node, npm, python, cargo, flutter, java, gradle, adb, sdkmanager, check_android_sdk()];
    // Python is commonly installed under python3 on Linux/macOS.
    if !checks[2].available {
        checks[2] = check_command("python3", &["--version"], "Python 3").await;
    }
    checks
}
#[tauri::command]
pub async fn run_approved_build(app: AppHandle, state: tauri::State<'_, BuildJobState>, root_path: String, profile: String) -> Result<BuildResult, String> {
    let root = approved_workspace(&app, &root_path)?;
    // Before executing a project-specific profile, verify its essential prerequisites
    // and return actionable errors instead of a generic process-spawn failure.
    if profile == "droidlab-appimage" {
        if !cfg!(target_os = "linux") {
            return Err("DroidLab AppImage creation requires a Linux build host.".into());
        }
        if !root.join("scripts/build-linux.sh").is_file() || !root.join("src/app.py").is_file() {
            return Err("The inspected folder is not a complete DroidLab source root (missing scripts/build-linux.sh or src/app.py). Re-import the full repository ZIP.".into());
        }
        for (command, label) in [("bash", "Bash"), ("wget", "wget"), ("tar", "tar"), ("xz", "xz")] {
            let output = OsCommand::new(command).arg("--version").stdout(Stdio::null()).stderr(Stdio::null()).status();
            if !matches!(output, Ok(status) if status.success()) {
                return Err(format!("Missing required tool or failed version check: {label}. Install it and ensure it is on PATH, then run Check tools again."));
            }
        }
        let python = if cfg!(windows) { "python" } else { "python3" };
        let tkinter = OsCommand::new(python).args(["-c", "import tkinter"]).stdout(Stdio::null()).stderr(Stdio::null()).status()
            .map(|status| status.success()).unwrap_or(false);
        if !tkinter {
            return Err("Tkinter is missing from the active Python installation. On Ubuntu/Debian, install python3-tk.".into());
        }
        let pyinstaller = OsCommand::new(python).args(["-m", "PyInstaller", "--version"]).stdout(Stdio::null()).stderr(Stdio::null()).status()
            .map(|status| status.success()).unwrap_or(false);
        if !pyinstaller {
            return Err("PyInstaller is not installed for the active Python interpreter. Install it in the environment used by Repopack.".into());
        }
        let appimagetool = OsCommand::new("appimagetool").arg("--version").stdout(Stdio::null()).stderr(Stdio::null()).status();
        if !matches!(appimagetool, Ok(status) if status.success()) {
            return Err("appimagetool is missing or failed to run. Install appimagetool or follow DroidLab's Linux build instructions before building.".into());
        }
    }
    let (program, args, label): (&str, Vec<&str>, &str) = match profile.as_str() {
        "npm-install" => {
            if !root.join("package.json").is_file() {
                return Err("Dependency preparation requires package.json in the project root.".into());
            }
            if root.join("package-lock.json").is_file() || root.join("npm-shrinkwrap.json").is_file() {
                ("npm", vec!["ci", "--no-audit", "--no-fund"], "npm ci (dependency preparation)")
            } else {
                ("npm", vec!["install", "--no-audit", "--no-fund"], "npm install (dependency preparation)")
            }
        }
        "tauri-build" => {
            if !root.join("src-tauri/tauri.conf.json").is_file() || !root.join("package.json").is_file() {
                return Err("Tauri build requires package.json and src-tauri/tauri.conf.json in the project root.".into());
            }
            ("npm", vec!["run", "tauri", "--", "build"], "npm run tauri build")
        }
        "python-install" => {
            let python = if cfg!(windows) { "python" } else { "python3" };
            if root.join("requirements.txt").is_file() {
                (python, vec!["-m", "pip", "install", "-r", "requirements.txt"], "Python dependency installation")
            } else if root.join("pyproject.toml").is_file() {
                (python, vec!["-m", "pip", "install", "."], "Install Python project from pyproject.toml")
            } else {
                return Err("Python dependency setup requires requirements.txt or pyproject.toml.".into());
            }
        }
        "python-package" | "python-desktop-package" => {
            let entrypoint = ["src/app.py", "main.py", "app.py", "run.py"].iter().copied()
                .find(|candidate| root.join(candidate).is_file())
                .ok_or("Could not infer a Python entry point. Supported locations include src/app.py, main.py, app.py, and run.py.")?;
            let python = if cfg!(windows) { "python" } else { "python3" };
            (python, vec!["-m", "PyInstaller", "--onefile", "--clean", "--noconfirm", entrypoint], "PyInstaller desktop package")
        }
        "droidlab-appimage" => {
            if !root.join("scripts/build-linux.sh").is_file() || !root.join("src/app.py").is_file() {
                return Err("DroidLab AppImage profile requires scripts/build-linux.sh and src/app.py.".into());
            }
            if !cfg!(target_os = "linux") {
                return Err("DroidLab's AppImage profile must run on Linux. Use a Linux build host or choose a supported Windows workflow.".into());
            }
            if !executable_available("bash") { return Err("Bash is missing from PATH.".into()); }
            ("bash", vec!["scripts/build-linux.sh"], "DroidLab Linux AppImage packaging script")
        }
        "rust-release" => {
            if !root.join("Cargo.toml").is_file() {
                return Err("Rust release builds require Cargo.toml in the project root.".into());
            }
            ("cargo", vec!["build", "--release"], "cargo build --release")
        }
        "node-build" => {
            if !root.join("package.json").is_file() {
                return Err("This profile requires package.json in the project root.".into());
            }
            ("npm", vec!["run", "build"], "npm run build")
        }
        "electron-package" => {
            if !root.join("package.json").is_file() {
                return Err("Electron packaging requires package.json in the project root.".into());
            }
            if crate::package_has_script(&root, "dist") {
                ("npm", vec!["run", "dist"], "npm run dist (Electron packaging)")
            } else if crate::package_has_script(&root, "make") {
                ("npm", vec!["run", "make"], "npm run make (Electron packaging)")
            } else if crate::package_has_script(&root, "package") {
                ("npm", vec!["run", "package"], "npm run package (Electron packaging)")
            } else {
                return Err("No recognized Electron packaging script found. Add a reviewed 'dist', 'make', or 'package' script to package.json.".into());
            }
        }
        "electron-build" => {
            if !root.join("package.json").is_file() {
                return Err("Electron build requires package.json in the project root.".into());
            }
            ("npm", vec!["run", "build"], "npm run build (Electron project)")
        }
        "flutter-windows" => {
            if !root.join("pubspec.yaml").is_file() { return Err("Flutter profile requires pubspec.yaml.".into()); }
            if !cfg!(target_os = "windows") { return Err("Windows-target Flutter builds must run on Windows.".into()); }
            ("flutter", vec!["build", "windows"], "flutter build windows")
        }
        "flutter-linux" => {
            if !root.join("pubspec.yaml").is_file() { return Err("Flutter profile requires pubspec.yaml.".into()); }
            if !cfg!(target_os = "linux") { return Err("Linux-target Flutter builds must run on Linux.".into()); }
            ("flutter", vec!["build", "linux"], "flutter build linux")
        }
        "flutter-apk" => {
            if !root.join("pubspec.yaml").is_file() { return Err("Flutter profile requires pubspec.yaml.".into()); }
            ("flutter", vec!["build", "apk"], "flutter build apk")
        }
        "android-debug-apk" => {
            let gradlew = if cfg!(windows) { root.join("gradlew.bat") } else { root.join("gradlew") };
            if !gradlew.is_file() { return Err("Android Gradle wrapper not found in project root.".into()); }
            if cfg!(windows) {
                ("cmd", vec!["/C", "gradlew.bat", "assembleDebug"], "Gradle assembleDebug")
            } else {
                ("sh", vec!["gradlew", "assembleDebug"], "Gradle assembleDebug")
            }
        }
        _ => return Err("Unknown build profile. Only listed, user-approved profiles are accepted.".into()),
    };

    emit(&app, "warning", format!("Starting approved build: {label}. Repository build scripts can execute arbitrary code."));
    // Windows tools such as npm and Flutter are commonly exposed as .cmd/.bat
    // launchers, which need cmd.exe rather than direct CreateProcess execution.
    let mut command = if cfg!(windows) && matches!(program, "npm" | "flutter") {
        let mut cmd = Command::new("cmd");
        cmd.arg("/C").arg(program).args(&args);
        cmd
    } else if program == "bash" && args.first() == Some(&"scripts/build-linux.sh") {
        // DroidLab's build script invokes wget, tar, PyInstaller, and appimagetool.
        let mut cmd = Command::new("bash");
        cmd.args(&args);
        cmd.env("PYTHON", "python3");
        cmd
    } else {
        let mut cmd = Command::new(program);
        cmd.args(&args);
        cmd
    };
    command.current_dir(&root).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());

    #[cfg(unix)]
    { command.env("CI", "1"); }

    let mut child = command.spawn().map_err(|e| format!("Could not start {label}: {e}. Check that the required tool is installed and on PATH."))?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let app_out = app.clone();
    let mut stdout_task = tokio::spawn(async move {
        if let Some(stream) = stdout {
            let mut lines = BufReader::new(stream).lines();
            while let Ok(Some(line)) = lines.next_line().await { emit(&app_out, "stdout", line); }
        }
    });
    let app_err = app.clone();
    let mut stderr_task = tokio::spawn(async move {
        if let Some(stream) = stderr {
            let mut lines = BufReader::new(stream).lines();
            while let Ok(Some(line)) = lines.next_line().await { emit(&app_err, "stderr", line); }
        }
    });

    let already_running = {
        let mut active = state.active.lock().map_err(|_| "Build state is unavailable.")?;
        if *active { true } else { *active = true; false }
    };
    if already_running {
        let _ = child.kill().await;
        return Err("Another build is already running.".into());
    }
    let (cancel_tx, cancel_rx) = oneshot::channel();
    {
        let mut slot = state.cancellation.lock().map_err(|_| "Build state is unavailable.")?;
        *slot = Some(cancel_tx);
    }
    let wait_outcome = tokio::select! {
        waited = time::timeout(Duration::from_secs(1800), child.wait()) => Some(waited),
        _ = cancel_rx => None,
    };
    let outcome = match wait_outcome {
        Some(Ok(Ok(status))) => (status.code(), status.success(), false, false),
        Some(Ok(Err(error))) => {
            let _ = child.kill().await;
            stdout_task.abort();
            stderr_task.abort();
            if let Ok(mut slot) = state.cancellation.lock() { *slot = None; }
            if let Ok(mut active) = state.active.lock() { *active = false; }
            return Err(format!("Could not wait for build process: {error}"));
        }
        Some(Err(_)) => {
            emit(&app, "warning", "Build exceeded the 30-minute limit; terminating the main build process.");
            let _ = child.kill().await;
            let _ = child.wait().await;
            stdout_task.abort();
            stderr_task.abort();
            (None, false, true, false)
        }
        None => {
            emit(&app, "warning", "Build cancellation requested; terminating the main build process.");
            let _ = child.kill().await;
            let _ = child.wait().await;
            stdout_task.abort();
            stderr_task.abort();
            (None, false, false, true)
        }
    };
    if let Ok(mut slot) = state.cancellation.lock() { *slot = None; }
    if let Ok(mut active) = state.active.lock() { *active = false; }
    let (exit_code, success, timed_out, cancelled) = outcome;
    if !timed_out && !cancelled {
        if time::timeout(Duration::from_secs(5), &mut stdout_task).await.is_err() {
            stdout_task.abort();
            emit(&app, "warning", "Stopped waiting for stdout pipe closure after 5 seconds.");
        }
        if time::timeout(Duration::from_secs(5), &mut stderr_task).await.is_err() {
            stderr_task.abort();
            emit(&app, "warning", "Stopped waiting for stderr pipe closure after 5 seconds.");
        }
    }
    let mut artifacts = if success { discover_artifacts(&root) } else { Vec::new() };
    if success && profile == "droidlab-appimage" {
        // The AppImage script writes to dist/; preserve the artifact even if a
        // different AppImage naming pattern is used by the upstream script.
        if let Ok(entries) = fs::read_dir(root.join("dist")) {
            for entry in entries.flatten() {
                let path = entry.path();
                let is_appimage = path.extension().and_then(|e| e.to_str()).map(|ext| ext.eq_ignore_ascii_case("AppImage")).unwrap_or(false)
                    || path.file_name().and_then(|n| n.to_str()).map(|name| name.to_ascii_lowercase().contains(".appimage")).unwrap_or(false);
                if is_appimage && path.is_file() && !artifacts.iter().any(|item| item.path == path.to_string_lossy().to_string()) {
                    let metadata = entry.metadata().ok();
                    artifacts.push(BuildArtifact {
                        name: entry.file_name().to_string_lossy().into_owned(),
                        path: path.to_string_lossy().into_owned(),
                        size_bytes: metadata.map(|m| m.len()).unwrap_or(0),
                        kind: "AppImage".into(),
                    });
                }
            }
        }
    }
    let message = if cancelled {
        format!("{label} was cancelled. Child processes spawned by the build tool may require platform-specific cleanup.")
    } else if timed_out {
        format!("{label} timed out after 30 minutes. The main process was terminated; child-process cleanup is platform-dependent.")
    } else if success {
        if artifacts.is_empty() { format!("{label} completed successfully, but no recognized output artifact was found.") }
        else { format!("{label} completed successfully; found {} artifact(s).", artifacts.len()) }
    } else { format!("{label} failed with exit code {:?}.", exit_code) };
    emit(&app, if success { "success" } else { "error" }, &message);
    if success && artifacts.is_empty() {
        emit(&app, "warning", "Build exited successfully, but no recognized installer/package was found. A successful process does not guarantee an installable artifact.");
    }
    for artifact in &artifacts {
        emit(&app, "info", format!("Artifact: {} ({} bytes) — {}", artifact.name, artifact.size_bytes, artifact.path));
    }
    Ok(BuildResult { profile, exit_code, success, message, artifacts })
}
