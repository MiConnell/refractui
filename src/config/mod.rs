use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

const MAX_HISTORY_PER_CONNECTION: usize = 100;

/// Get the config directory (~/.config/refractui)
pub fn config_dir() -> Result<PathBuf> {
    let home = dirs::home_dir().context("Could not find home directory")?;
    Ok(home.join(".config").join("refractui"))
}

/// Application state (last connection, preferences)
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct AppState {
    /// Last used connection (profile:target)
    pub last_connection: Option<String>,
}

impl AppState {
    pub fn load() -> Result<Self> {
        let path = config_dir()?.join("state.json");
        if !path.exists() {
            return Ok(Self::default());
        }
        let content = std::fs::read_to_string(&path)?;
        let state: Self = serde_json::from_str(&content)?;
        Ok(state)
    }

    pub fn save(&self) -> Result<()> {
        let dir = config_dir()?;
        std::fs::create_dir_all(&dir)?;
        let path = dir.join("state.json");
        let content = serde_json::to_string_pretty(self)?;
        std::fs::write(path, content)?;
        Ok(())
    }
}

/// Query history (per connection)
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct QueryHistory {
    /// Map of connection key (profile:target) to list of queries (newest last)
    pub connections: HashMap<String, Vec<String>>,
}

impl QueryHistory {
    pub fn load() -> Result<Self> {
        let path = config_dir()?.join("history.json");
        if !path.exists() {
            return Ok(Self::default());
        }
        let content = std::fs::read_to_string(&path)?;
        let history: Self = serde_json::from_str(&content)?;
        Ok(history)
    }

    pub fn save(&self) -> Result<()> {
        let dir = config_dir()?;
        std::fs::create_dir_all(&dir)?;
        let path = dir.join("history.json");
        let content = serde_json::to_string_pretty(self)?;
        std::fs::write(path, content)?;
        Ok(())
    }

    /// Add a query to history for a connection
    pub fn add(&mut self, connection_key: &str, query: &str) {
        let query = query.trim().to_string();
        if query.is_empty() {
            return;
        }

        let history = self
            .connections
            .entry(connection_key.to_string())
            .or_default();

        // Remove duplicate if exists (we'll add it to the end)
        if let Some(pos) = history.iter().position(|q| q == &query) {
            history.remove(pos);
        }

        history.push(query);

        // Trim to max size
        if history.len() > MAX_HISTORY_PER_CONNECTION {
            let excess = history.len() - MAX_HISTORY_PER_CONNECTION;
            history.drain(0..excess);
        }
    }

    /// Get history for a connection (newest last)
    pub fn get(&self, connection_key: &str) -> &[String] {
        self.connections
            .get(connection_key)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }
}

/// Get the queries directory (~/.config/refractui/queries)
pub fn queries_dir() -> Result<PathBuf> {
    let dir = config_dir()?.join("queries");
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// List saved query files
pub fn list_saved_queries() -> Result<Vec<String>> {
    let dir = queries_dir()?;
    let mut files = Vec::new();

    if dir.exists() {
        for entry in std::fs::read_dir(&dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().map(|e| e == "sql").unwrap_or(false) {
                if let Some(name) = path.file_name() {
                    files.push(name.to_string_lossy().to_string());
                }
            }
        }
    }

    files.sort();
    Ok(files)
}

/// Save a query to a file
pub fn save_query(name: &str, content: &str) -> Result<PathBuf> {
    let dir = queries_dir()?;
    let filename = if name.ends_with(".sql") {
        name.to_string()
    } else {
        format!("{}.sql", name)
    };
    let path = dir.join(&filename);
    std::fs::write(&path, content)?;
    Ok(path)
}

/// Load a query from a file
pub fn load_query(name: &str) -> Result<String> {
    let dir = queries_dir()?;
    let filename = if name.ends_with(".sql") {
        name.to_string()
    } else {
        format!("{}.sql", name)
    };
    let path = dir.join(&filename);
    let content = std::fs::read_to_string(&path)?;
    Ok(content)
}

/// Delete a saved query file
pub fn delete_query(name: &str) -> Result<()> {
    let dir = queries_dir()?;
    let filename = if name.ends_with(".sql") {
        name.to_string()
    } else {
        format!("{}.sql", name)
    };
    let path = dir.join(&filename);
    std::fs::remove_file(&path)?;
    Ok(())
}

/// Get the autosave file path
fn autosave_path() -> Result<PathBuf> {
    Ok(config_dir()?.join("autosave.sql"))
}

/// Save editor content for auto-restore on next launch
pub fn save_autosave(content: &str) -> Result<()> {
    let dir = config_dir()?;
    std::fs::create_dir_all(&dir)?;
    let path = autosave_path()?;
    std::fs::write(path, content)?;
    Ok(())
}

/// Load auto-saved editor content (returns empty string if none)
pub fn load_autosave() -> String {
    autosave_path()
        .ok()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .unwrap_or_default()
}

/// Get the exports directory (~/.config/refractui/exports)
#[allow(dead_code)]
pub fn exports_dir() -> Result<PathBuf> {
    let dir = config_dir()?.join("exports");
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Export results to CSV file (auto-named in exports dir)
#[allow(dead_code)]
pub fn export_csv(columns: &[String], rows: &[Vec<String>]) -> Result<PathBuf> {
    let dir = exports_dir()?;
    let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S");
    let filename = format!("export_{}.csv", timestamp);
    let path = dir.join(&filename);

    export_csv_to_path(columns, rows, &path)?;
    Ok(path)
}

/// Export results to CSV file at specified path
pub fn export_csv_to_path(
    columns: &[String],
    rows: &[Vec<String>],
    path: &std::path::Path,
) -> Result<()> {
    let mut writer = std::fs::File::create(path)?;
    use std::io::Write;

    // Write header
    let header: Vec<String> = columns.iter().map(|c| escape_csv_field(c)).collect();
    writeln!(writer, "{}", header.join(","))?;

    // Write rows
    for row in rows {
        let escaped: Vec<String> = row.iter().map(|c| escape_csv_field(c)).collect();
        writeln!(writer, "{}", escaped.join(","))?;
    }

    Ok(())
}

fn escape_csv_field(field: &str) -> String {
    if field.contains(',') || field.contains('"') || field.contains('\n') {
        format!("\"{}\"", field.replace('"', "\"\""))
    } else {
        field.to_string()
    }
}
