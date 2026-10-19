mod runner;

use serde::Serialize;
use std::{
    fs::{self, File},
    io::{self, Cursor, Read},
    path::{Path, PathBuf},
    time::Duration,
};
use tauri::{AppHandle, Emitter, Manager};
use url::Url;
use uuid::Uuid;
use zip::ZipArchive;

const MAX_ARCHIVE_BYTES: usize = 100 * 1024 * 1024;
const MAX_EXTRACTED_BYTES: u64 = 400 * 1024 * 1024;
const MAX_ARCHIVE_ENTRIES: usize = 20_000;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SourceInfo {
    name: String,
    root_path: String,
    project_type: String,
    indicators: Vec<String>,
    next_step: String,
}

#[derive(Clone, Serialize)]
struct LogEvent {
    level: String,
    message: String,
}

fn log(app: &AppHandle, level: &str, message: impl Into<String>) {
    let _ = app.emit(
        "job-log",
        LogEvent {
            level: level.to_string(),
            message: message.into(),
        },
    );
}

#[tauri::command]
async fn prepare_source(app: AppHandle, input: String) -> Result<SourceInfo, String> {
    let input = input.trim();
    if input.is_empty() {
        return Err("Enter a GitHub repository URL or choose a ZIP file.".into());
    }

    log(&app, "info", "Starting source preparation.");
    let (archive_bytes, display_name) = if input.starts_with("https://") || input.starts_with("http://") {
        let (owner, repo) = parse_github_url(input)?;
        log(&app, "info", format!("Resolving public GitHub repository {owner}/{repo}."));
        let archive_url = format!("https://api.github.com/repos/{owner}/{repo}/zipball");
        let client = reqwest::Client::builder()
            .user_agent("Repopack/0.1")
            .timeout(Duration::from_secs(60))
            .build()
            .map_err(|e| format!("Could not initialize the downloader: {e}"))?;

        let mut response = client
            .get(&archive_url)
            .send()
            .await
            .map_err(|e| format!("Download failed: {e}"))?
            .error_for_status()
            .map_err(|e| format!("GitHub could not provide this archive. Check the URL and repository visibility: {e}"))?;

        if response.content_length().is_some_and(|n| n > MAX_ARCHIVE_BYTES as u64) {
            return Err("The repository archive exceeds the 100 MB download limit for this preview.".into());
        }

        // Enforce the size cap while reading; do not buffer an unbounded HTTP response.
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|e| format!("Could not read the downloaded archive: {e}"))?
        {
            if bytes.len().saturating_add(chunk.len()) > MAX_ARCHIVE_BYTES {
                return Err("The repository archive exceeds the 100 MB download limit for this preview.".into());
            }
            bytes.extend_from_slice(&chunk);
        }
        (bytes, repo)
    } else {
        let path = PathBuf::from(input);
        if path.extension().and_then(|e| e.to_str()).map(|e| e.eq_ignore_ascii_case("zip")) != Some(true) {
            return Err("Choose a .zip archive or enter an https://github.com/owner/repo URL.".into());
        }
        let metadata = fs::metadata(&path).map_err(|e| format!("Cannot read the selected ZIP file: {e}"))?;
        if !metadata.is_file() {
            return Err("The selected ZIP path is not a file.".into());
        }
        if metadata.len() > MAX_ARCHIVE_BYTES as u64 {
            return Err("The ZIP file exceeds the 100 MB size limit for this preview.".into());
        }
        let name = path.file_stem().and_then(|n| n.to_str()).unwrap_or("local-project").to_string();
        // The file may change after metadata is read, so cap the actual read too.
        let file = File::open(&path).map_err(|e| format!("Could not open the selected ZIP file: {e}"))?;
        let mut bytes = Vec::new();
        file.take(MAX_ARCHIVE_BYTES as u64 + 1).read_to_end(&mut bytes)
            .map_err(|e| format!("Could not read the selected ZIP file: {e}"))?;
        if bytes.len() > MAX_ARCHIVE_BYTES {
            return Err("The ZIP file exceeds the 100 MB size limit for this preview.".into());
        }
        (bytes, name)
    };

    log(&app, "info", format!("Archive ready ({} bytes). Extracting into a private app-cache workspace.", archive_bytes.len()));

    let mut jobs_dir = app.path().app_cache_dir().map_err(|e| format!("Could not locate the app cache: {e}"))?;
    jobs_dir.push("jobs");
    fs::create_dir_all(&jobs_dir).map_err(|e| format!("Could not create workspace: {e}"))?;

    let job_dir = jobs_dir.join(Uuid::new_v4().to_string());
    let extracted_dir = job_dir.join("source");
    fs::create_dir_all(&extracted_dir).map_err(|e| format!("Could not create extraction directory: {e}"))?;

    if let Err(error) = extract_archive(&archive_bytes, &extracted_dir) {
        let _ = fs::remove_dir_all(&job_dir);
        return Err(error);
    }
    log(&app, "success", "ZIP extraction finished. Path traversal and symlinks are rejected.");

    let root = find_project_root(&extracted_dir);
    let (project_type, indicators, next_step) = detect_project(&root);
    log(&app, "success", format!("Detected project type: {project_type}."));
    log(&app, "info", "No package scripts or build commands have been executed.");

    Ok(SourceInfo {
        name: display_name,
        root_path: root.to_string_lossy().into_owned(),
        project_type,
        indicators,
        next_step,
    })
}

fn parse_github_url(input: &str) -> Result<(String, String), String> {
    let url = Url::parse(input).map_err(|_| "Enter a valid HTTPS GitHub URL or a local ZIP path.".to_string())?;
    if url.scheme() != "https" {
        return Err("Repository downloads must use HTTPS.".into());
    }
    if url.host_str() != Some("github.com") {
        return Err("For this preview, repository URLs must use github.com.".into());
    }
    let segments: Vec<_> = url.path_segments().ok_or("Invalid GitHub repository URL.")?
        .filter(|segment| !segment.is_empty())
        .take(2)
        .map(str::to_string)
        .collect();

    if segments.len() != 2 {
        return Err("Use a URL like https://github.com/owner/repository.".into());
    }

    let owner = segments[0].clone();
    let repo = segments[1].strip_suffix(".git").unwrap_or(&segments[1]).to_string();
    for (label, value) in [("owner", owner.as_str()), ("repository", repo.as_str())] {
        if value.is_empty() || !value.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')) {
            return Err(format!("The GitHub {label} name contains unsupported characters."));
        }
    }
    Ok((owner, repo))
}

fn extract_archive(bytes: &[u8], destination: &Path) -> Result<(), String> {
    let mut archive = ZipArchive::new(Cursor::new(bytes))
        .map_err(|e| format!("This file is not a valid ZIP archive: {e}"))?;

    if archive.len() > MAX_ARCHIVE_ENTRIES {
        return Err("The archive contains too many files for this preview.".into());
    }

    let mut extracted_bytes = 0_u64;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index)
            .map_err(|e| format!("Could not read ZIP entry {index}: {e}"))?;
        let relative = entry.enclosed_name()
            .ok_or_else(|| format!("Unsafe path in ZIP entry {}.", entry.name()))?
            .to_path_buf();

        if entry.is_symlink() {
            return Err(format!("Symbolic links are not accepted in ZIP archives ({}).", relative.display()));
        }

        let output_path = destination.join(relative);
        if entry.is_dir() {
            fs::create_dir_all(&output_path)
                .map_err(|e| format!("Could not create directory: {e}"))?;
            continue;
        }

        // Reject obviously oversized entries early, but also enforce the cap against
        // bytes actually decompressed and written (ZIP metadata is untrusted).
        if entry.size() > MAX_EXTRACTED_BYTES.saturating_sub(extracted_bytes) {
            return Err("The expanded archive exceeds the 400 MB extraction limit.".into());
        }

        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("Could not create extraction directory: {e}"))?;
        }
        let mut target = File::create(&output_path)
            .map_err(|e| format!("Could not create extracted file: {e}"))?;
        let remaining = MAX_EXTRACTED_BYTES.saturating_sub(extracted_bytes);
        let copied = io::copy(&mut entry.take(remaining.saturating_add(1)), &mut target)
            .map_err(|e| format!("Could not extract {}: {e}", output_path.display()))?;
        if copied > remaining {
            return Err("The expanded archive exceeds the 400 MB extraction limit.".into());
        }
        extracted_bytes = extracted_bytes.saturating_add(copied);
    }
    Ok(())
}

fn find_project_root(extracted_dir: &Path) -> PathBuf {
    if has_project_marker(extracted_dir) {
        return extracted_dir.to_path_buf();
    }

    let children: Vec<PathBuf> = match fs::read_dir(extracted_dir) {
        Ok(entries) => entries
            .filter_map(Result::ok)
            .filter_map(|entry| {
                entry.file_type().ok().filter(|kind| kind.is_dir()).map(|_| entry.path())
            })
            .collect(),
        Err(_) => Vec::new(),
    };

    if children.len() == 1 && has_project_marker(&children[0]) {
        children[0].clone()
    } else {
        extracted_dir.to_path_buf()
    }
}

fn has_project_marker(root: &Path) -> bool {
    [
        "package.json", "pubspec.yaml", "pyproject.toml", "requirements.txt",
        "Cargo.toml", "index.html", "gradlew", "settings.gradle",
        "settings.gradle.kts", "build.gradle", "build.gradle.kts",
        "src-tauri/tauri.conf.json",
    ].iter().any(|name| root.join(name).is_file())
}

fn detect_project(root: &Path) -> (String, Vec<String>, String) {
    let markers = [
        "src-tauri/tauri.conf.json",
        "package.json",
        "pubspec.yaml",
        "pyproject.toml",
        "requirements.txt",
        "Cargo.toml",
        "gradlew",
        "gradlew.bat",
        "settings.gradle",
        "settings.gradle.kts",
        "build.gradle",
        "build.gradle.kts",
        "index.html",
    ];
    let indicators: Vec<String> = markers.iter()
        .filter(|marker| root.join(marker).is_file())
        .map(|marker| marker.to_string())
        .collect();

    if root.join("src-tauri/tauri.conf.json").is_file() {
        return ("tauri".into(), indicators, "Review the project's scripts and target before running its Tauri packaging command.".into());
    }
    if package_has_dependency(root, "electron") {
        return ("electron".into(), indicators, "Review package.json scripts and the project's packaging configuration; Electron builds vary by repository.".into());
    }
    if root.join("pubspec.yaml").is_file() {
        return ("flutter".into(), indicators, "Install Flutter and the target platform toolchain, then choose a supported target such as Windows, Linux, or Android.".into());
    }
    if root.join("gradlew").is_file() || root.join("gradlew.bat").is_file() || root.join("settings.gradle").is_file() || root.join("settings.gradle.kts").is_file() {
        return ("android-gradle".into(), indicators, "Check the Android SDK and Gradle prerequisites. Debug APKs and signed release packages require different handling.".into());
    }
    if root.join("package.json").is_file() {
        return ("node".into(), indicators, "A Node project was found. A successful web build does not automatically create a native installer; review its build script first.".into());
    }
    if root.join("pyproject.toml").is_file() || root.join("requirements.txt").is_file() {
        return ("python".into(), indicators, "Choose an explicit packaging profile (for example PyInstaller) and review dependency installation before execution.".into());
    }
    if root.join("Cargo.toml").is_file() {
        return ("rust".into(), indicators, "Review Cargo targets and manifests; this repository may be a library or CLI rather than a desktop GUI app.".into());
    }
    if root.join("index.html").is_file() {
        return ("static-web".into(), indicators, "This looks like static web content. A Tauri wrapper can be added if you want to package it as a desktop app.".into());
    }
    ("unknown".into(), indicators, "Project type was not recognized. Inspect the README and define a reviewed build profile before running commands.".into())
}

fn package_has_dependency(root: &Path, dependency: &str) -> bool {
    let manifest = match fs::read_to_string(root.join("package.json")) {
        Ok(text) => text,
        Err(_) => return false,
    };
    let value: serde_json::Value = match serde_json::from_str(&manifest) {
        Ok(value) => value,
        Err(_) => return false,
    };
    ["dependencies", "devDependencies", "optionalDependencies"]
        .iter()
        .any(|section| value.get(*section).and_then(|deps| deps.get(dependency)).is_some())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![prepare_source, runner::check_build_tools, runner::run_approved_build, runner::open_artifact_folder])
        .run(tauri::generate_context!())
        .expect("error while running Repopack");
}
