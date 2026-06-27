use anyhow::{Context, Result};
use std::path::PathBuf;
use std::process::Stdio;
use tokio::process::Command;

/// Get the refractui home directory (~/.refractui)
pub fn get_refractui_home() -> Result<PathBuf> {
    let home = dirs::home_dir().context("Could not find home directory")?;
    Ok(home.join(".refractui"))
}

/// Get the venv directory (~/.refractui/venv)
pub fn get_venv_dir() -> Result<PathBuf> {
    Ok(get_refractui_home()?.join("venv"))
}

/// Get the stub project directory (~/.refractui/stub_project)
pub fn get_stub_project_dir() -> Result<PathBuf> {
    Ok(get_refractui_home()?.join("stub_project"))
}

/// Get the path to dbt executable in the venv
pub fn get_dbt_path() -> Result<PathBuf> {
    let venv = get_venv_dir()?;
    // On Unix, binaries are in venv/bin/
    // On Windows, they'd be in venv/Scripts/
    #[cfg(unix)]
    let dbt = venv.join("bin").join("dbt");
    #[cfg(windows)]
    let dbt = venv.join("Scripts").join("dbt.exe");
    Ok(dbt)
}

/// Check if uv is available
pub async fn check_uv_available() -> bool {
    Command::new("uv")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Create the venv if it doesn't exist
pub async fn ensure_venv() -> Result<()> {
    let venv_dir = get_venv_dir()?;

    if venv_dir.exists() {
        return Ok(());
    }

    // Create parent directory
    let home = get_refractui_home()?;
    tokio::fs::create_dir_all(&home).await?;

    // Create venv with uv
    let status = Command::new("uv")
        .args(["venv", venv_dir.to_str().unwrap()])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .status()
        .await
        .context("Failed to run uv venv")?;

    if !status.success() {
        anyhow::bail!("uv venv failed");
    }

    Ok(())
}

/// Check if a package is installed in the venv
pub async fn is_package_installed(package: &str) -> Result<bool> {
    let venv_dir = get_venv_dir()?;

    #[cfg(unix)]
    let pip = venv_dir.join("bin").join("pip");
    #[cfg(windows)]
    let pip = venv_dir.join("Scripts").join("pip.exe");

    if !pip.exists() {
        return Ok(false);
    }

    let output = Command::new(&pip)
        .args(["show", package])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await?;

    Ok(output.success())
}

/// Install a package into the venv using uv
pub async fn install_package(package: &str) -> Result<()> {
    let venv_dir = get_venv_dir()?;

    let output = Command::new("uv")
        .args([
            "pip",
            "install",
            "--python",
            venv_dir.join("bin").join("python").to_str().unwrap(),
            package,
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .context("Failed to run uv pip install")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("Failed to install {}: {}", package, stderr);
    }

    Ok(())
}

/// Ensure dbt-core is installed
pub async fn ensure_dbt_core() -> Result<()> {
    if !is_package_installed("dbt-core").await? {
        install_package("dbt-core").await?;
    }
    Ok(())
}

/// Ensure a specific dbt adapter is installed
pub async fn ensure_adapter(adapter_package: &str) -> Result<()> {
    if !is_package_installed(adapter_package).await? {
        install_package(adapter_package).await?;
    }
    Ok(())
}

/// Full setup: ensure venv exists and dbt-core is installed
pub async fn ensure_environment() -> Result<()> {
    ensure_venv().await?;
    ensure_dbt_core().await?;
    Ok(())
}
