use std::{fs::{self, OpenOptions, File}, io::{Write, Read}, path::PathBuf};
use tauri::{AppHandle, Manager};
use zip::{write::SimpleFileOptions, ZipWriter};

fn directory(app: &AppHandle) -> Result<PathBuf, String> {
    let path = app.path().app_data_dir().map_err(|e| e.to_string())?.join("diagnostics");
    fs::create_dir_all(&path).map_err(|e| e.to_string())?;
    Ok(path)
}

#[tauri::command]
pub fn write_diagnostic_log(app: AppHandle, level: String, message: String) -> Result<(), String> {
    let dir = directory(&app)?;
    let mut file = OpenOptions::new().create(true).append(true).open(dir.join("repopack.log")).map_err(|e| e.to_string())?;
    let level: String = level.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').take(16).collect();
    let message: String = message.chars().take(12000).collect();
    writeln!(file, "[{}] [{}] {}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs(), level, message.replace('\n', "\\n")).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn export_diagnostic_logs(app: AppHandle, destination_path: String) -> Result<String, String> {
    let destination = PathBuf::from(destination_path);
    if destination.extension().and_then(|s| s.to_str()).map(|s| s.eq_ignore_ascii_case("zip")) != Some(true) {
        return Err("Select a ZIP file destination.".into());
    }
    if let Some(parent) = destination.parent() { fs::create_dir_all(parent).map_err(|e| e.to_string())?; }
    let mut zip = ZipWriter::new(File::create(&destination).map_err(|e| e.to_string())?);
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let log_path = directory(&app)?.join("repopack.log");
    if log_path.is_file() {
        zip.start_file("diagnostics/repopack.log", options).map_err(|e| e.to_string())?;
        let mut input = File::open(log_path).map_err(|e| e.to_string())?;
        let mut bytes = Vec::new();
        input.read_to_end(&mut bytes).map_err(|e| e.to_string())?;
        zip.write_all(&bytes).map_err(|e| e.to_string())?;
    } else {
        zip.start_file("diagnostics/README.txt", options).map_err(|e| e.to_string())?;
        zip.write_all(b"No diagnostic log has been recorded yet.").map_err(|e| e.to_string())?;
    }
    zip.start_file("diagnostics/ABOUT.txt", options).map_err(|e| e.to_string())?;
    zip.write_all(b"Review this diagnostic bundle before sharing; logs can include local paths and error messages.").map_err(|e| e.to_string())?;
    zip.finish().map_err(|e| e.to_string())?;
    Ok(destination.to_string_lossy().into_owned())
}

#[tauri::command]
pub fn export_diagnostic_text(app: AppHandle, destination_path: String) -> Result<String, String> {
    let destination = PathBuf::from(destination_path);
    if let Some(parent) = destination.parent() { fs::create_dir_all(parent).map_err(|e| e.to_string())?; }
    let source = directory(&app)?.join("repopack.log");
    if source.is_file() { fs::copy(source, &destination).map_err(|e| e.to_string())?; }
    else { fs::write(&destination, "No diagnostic log has been recorded yet.").map_err(|e| e.to_string())?; }
    Ok(destination.to_string_lossy().into_owned())
}
