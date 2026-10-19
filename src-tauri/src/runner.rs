use std::{fs, path::{Path, PathBuf}, process::{Command as OsCommand, Stdio}, sync::Mutex, time::Duration};
use tauri::{AppHandle, Emitter, Manager};
use tokio::{io::{AsyncBufReadExt, BufReader}, process::Command, sync::oneshot, time};

#[derive(Clone, serde::Serialize)]
struct RunnerLog { level: String, message: String }

#[derive(Default)]
pub struct BuildJobState { cancellation: Mutex<Option<oneshot::Sender<()>>> }

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

async fn check_command(command: &str, args: &[&str], tool: &str) -> ToolCheck {
    match Command::new(command).args(args).stdout(Stdio::piped()).stderr(Stdio::piped()).output().await {
        Ok(output) if output.status.success() => {
            let version = String::from_utf8_lossy(&output.stdout).lines().next().unwrap_or("").trim().to_string();
            ToolCheck { tool: tool.into(), available: true, version: if version.is_empty() { None } else { Some(version) }, message: "Available".into() }
        }
        Ok(output) => {
            let error = String::from_utf8_lossy(&output.stderr).lines().next().unwrap_or("Command returned an error").trim().to_string();
            ToolCheck { tool: tool.into(), available: false, version: None, message: if error.is_empty() { "Command failed".into() } else { error } }
        }
        Err(e) => ToolCheck { tool: tool.into(), available: false, version: None, message: format!("Not available: {e}") }
    }
}

#[tauri::command]
pub async fn check_build_tools() -> Vec<ToolCheck> {
    let mut checks = vec![
        check_command("node", &["--version"], "Node.js").await,
        check_command("npm", &["--version"], "npm").await,
        check_command("python", &["--version"], "Python").await,
        check_command("cargo", &["--version"], "Rust / Cargo").await,
        check_command("flutter", &["--version"], "Flutter").await,
        check_command("java", &["-version"], "Java").await,
        check_command("gradle", &["--version"], "Gradle").await,
        check_command("adb", &["version"], "Android Debug Bridge").await,
        check_command("sdkmanager", &["--version"], "Android SDK Manager").await,
    ];
    // Python is commonly installed under python3 on Linux/macOS.
    if !checks[2].available {
        checks[2] = check_command("python3", &["--version"], "Python 3").await;
    }
    checks
}

#[tauri::command]
pub async fn run_approved_build(app: AppHandle, state: tauri::State<'_, BuildJobState>, root_path: String, profile: String) -> Result<BuildResult, String> {
    let root = approved_workspace(&app, &root_path)?;
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
        "node-build" => {
            if !root.join("package.json").is_file() {
                return Err("This profile requires package.json in the project root.".into());
            }
            ("npm", vec!["run", "build"], "npm run build")
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
                ("./gradlew", vec!["assembleDebug"], "Gradle assembleDebug")
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
    let stdout_task = tokio::spawn(async move {
        if let Some(stream) = stdout {
            let mut lines = BufReader::new(stream).lines();
            while let Ok(Some(line)) = lines.next_line().await { emit(&app_out, "stdout", line); }
        }
    });
    let app_err = app.clone();
    let stderr_task = tokio::spawn(async move {
        if let Some(stream) = stderr {
            let mut lines = BufReader::new(stream).lines();
            while let Ok(Some(line)) = lines.next_line().await { emit(&app_err, "stderr", line); }
        }
    });

    let (cancel_tx, cancel_rx) = oneshot::channel();
    {
        let mut slot = state.cancellation.lock().map_err(|_| "Build state is unavailable.")?;
        if slot.is_some() {
            let _ = child.kill().await;
            return Err("Another build is already running.".into());
        }
        *slot = Some(cancel_tx);
    }
    let outcome = tokio::select! {
        waited = time::timeout(Duration::from_secs(1800), child.wait()) => match waited {
            Ok(Ok(status)) => (status.code(), status.success(), false, false),
            Ok(Err(error)) => {
                let _ = child.kill().await;
                stdout_task.abort();
                stderr_task.abort();
                if let Ok(mut slot) = state.cancellation.lock() { *slot = None; }
                return Err(format!("Could not wait for build process: {error}"));
            }
            Err(_) => {
                emit(&app, "warning", "Build exceeded the 30-minute limit; terminating the main build process.");
                let _ = child.kill().await;
                let _ = child.wait().await;
                stdout_task.abort();
                stderr_task.abort();
                (None, false, true, false)
            }
        },
        _ = cancel_rx => {
            emit(&app, "warning", "Build cancellation requested; terminating the main build process.");
            let _ = child.kill().await;
            let _ = child.wait().await;
            stdout_task.abort();
            stderr_task.abort();
            (None, false, false, true)
        }
    };
    if let Ok(mut slot) = state.cancellation.lock() { *slot = None; }
    let (exit_code, success, timed_out, cancelled) = outcome;
    if !timed_out && !cancelled {
        let _ = stdout_task.await;
        let _ = stderr_task.await;
    }
    let artifacts = if success { discover_artifacts(&root) } else { Vec::new() };
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
