use std::{path::{Path, PathBuf}, process::Stdio};
use tauri::{AppHandle, Emitter, Manager};
use tokio::{io::{AsyncBufReadExt, BufReader}, process::Command};

#[derive(Clone, serde::Serialize)]
struct RunnerLog { level: String, message: String }

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
    ];
    // Python is commonly installed under python3 on Linux/macOS.
    if !checks[2].available {
        checks[2] = check_command("python3", &["--version"], "Python 3").await;
    }
    checks
}

#[tauri::command]
pub async fn run_approved_build(app: AppHandle, root_path: String, profile: String) -> Result<BuildResult, String> {
    let root = approved_workspace(&app, &root_path)?;
    let (program, args, label): (&str, Vec<&str>, &str) = match profile.as_str() {
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
            let wrapper = if cfg!(windows) { "gradlew.bat" } else { "./gradlew" };
            (wrapper, vec!["assembleDebug"], "Gradle assembleDebug")
        }
        _ => return Err("Unknown build profile. Only listed, user-approved profiles are accepted.".into()),
    };

    emit(&app, "warning", format!("Starting approved build: {label}. Repository build scripts can execute arbitrary code."));
    let mut command = Command::new(program);
    command.args(&args).current_dir(&root).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());

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

    let status = child.wait().await.map_err(|e| format!("Could not wait for build process: {e}"))?;
    let _ = stdout_task.await;
    let _ = stderr_task.await;
    let success = status.success();
    let message = if success { format!("{label} completed successfully.") } else { format!("{label} failed with exit code {:?}.", status.code()) };
    emit(&app, if success { "success" } else { "error" }, &message);
    Ok(BuildResult { profile, exit_code: status.code(), success, message })
}
