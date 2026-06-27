use anyhow::{Context, Result};
use std::collections::BTreeMap;
use std::process::Stdio;
use tokio::process::Command;

use crate::profiles::Connection;
use crate::venv;
use crate::QueryLimit;

#[derive(Debug)]
pub struct QueryResult {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

#[derive(Debug, Clone)]
pub struct ColumnInfo {
    pub name: String,
    pub data_type: String,
}

#[derive(Debug, Clone)]
pub struct TableInfo {
    pub name: String,
    pub table_type: String, // "BASE TABLE" or "VIEW"
    pub columns: Vec<ColumnInfo>,
}

#[derive(Debug, Clone)]
pub struct SchemaInfo {
    pub name: String,
    pub tables: Vec<TableInfo>,
}

/// Fetch database schema metadata (schemas, tables, columns)
pub async fn fetch_schema_metadata(connection: &Connection) -> Result<Vec<SchemaInfo>> {
    // Query for tables
    let tables_sql = r#"
        SELECT table_schema, table_name, table_type
        FROM information_schema.tables
        WHERE table_schema NOT IN ('information_schema', 'pg_catalog')
        ORDER BY table_schema, table_name
    "#;

    let tables_result = execute_query(connection, tables_sql, QueryLimit::NoLimit).await?;

    // Query for columns
    let columns_sql = r#"
        SELECT table_schema, table_name, column_name, data_type
        FROM information_schema.columns
        WHERE table_schema NOT IN ('information_schema', 'pg_catalog')
        ORDER BY table_schema, table_name, ordinal_position
    "#;

    let columns_result = execute_query(connection, columns_sql, QueryLimit::NoLimit).await?;

    // Build schema map
    let mut schemas: BTreeMap<String, BTreeMap<String, TableInfo>> = BTreeMap::new();

    // Process tables
    for row in &tables_result.rows {
        if row.len() >= 3 {
            let schema_name = &row[0];
            let table_name = &row[1];
            let table_type = &row[2];

            schemas.entry(schema_name.clone()).or_default().insert(
                table_name.clone(),
                TableInfo {
                    name: table_name.clone(),
                    table_type: table_type.clone(),
                    columns: Vec::new(),
                },
            );
        }
    }

    // Process columns
    for row in &columns_result.rows {
        if row.len() >= 4 {
            let schema_name = &row[0];
            let table_name = &row[1];
            let column_name = &row[2];
            let data_type = &row[3];

            if let Some(schema) = schemas.get_mut(schema_name) {
                if let Some(table) = schema.get_mut(table_name) {
                    table.columns.push(ColumnInfo {
                        name: column_name.clone(),
                        data_type: data_type.clone(),
                    });
                }
            }
        }
    }

    // Convert to Vec<SchemaInfo>
    let result: Vec<SchemaInfo> = schemas
        .into_iter()
        .map(|(name, tables)| SchemaInfo {
            name,
            tables: tables.into_values().collect(),
        })
        .collect();

    Ok(result)
}

/// Ensure stub project exists and is configured for the given connection
pub async fn ensure_stub_project(connection: &Connection) -> Result<()> {
    let stub_dir = venv::get_stub_project_dir()?;

    // Always create/update to ensure profile matches
    tokio::fs::create_dir_all(&stub_dir).await?;

    let dbt_project = format!(
        r#"name: 'refractui_stub'
version: '1.0.0'
profile: '{}'

# Minimal stub project for running ad-hoc queries
# This project is managed by refractui
"#,
        connection.profile
    );

    tokio::fs::write(stub_dir.join("dbt_project.yml"), dbt_project).await?;

    // Create empty models dir (dbt expects it)
    tokio::fs::create_dir_all(stub_dir.join("models")).await?;

    Ok(())
}

/// Ensure the venv and required adapter are installed
pub async fn ensure_adapter(connection: &Connection) -> Result<()> {
    // Check if uv is available
    if !venv::check_uv_available().await {
        anyhow::bail!("uv is not installed. Please install uv: https://docs.astral.sh/uv/");
    }

    // Ensure venv and dbt-core exist
    venv::ensure_environment().await?;

    // Ensure the specific adapter is installed
    let adapter_pkg = connection.adapter_package();
    venv::ensure_adapter(&adapter_pkg).await?;

    Ok(())
}

pub async fn execute_query(
    connection: &Connection,
    sql: &str,
    limit: QueryLimit,
) -> Result<QueryResult> {
    // Ensure stub project is configured for this connection
    ensure_stub_project(connection).await?;

    let stub_dir = venv::get_stub_project_dir()?;
    let dbt_path = venv::get_dbt_path()?;

    // Verify dbt exists
    if !dbt_path.exists() {
        anyhow::bail!(
            "dbt not found at {:?}. Try selecting a connection to install it.",
            dbt_path
        );
    }

    // Remove single-line comments (-- to end of line) before converting to single line
    // Otherwise the -- comment would comment out everything after it
    let sql: String = sql
        .lines()
        .map(|line| {
            if let Some(pos) = line.find("--") {
                line[..pos].trim_end()
            } else {
                line
            }
        })
        .collect::<Vec<_>>()
        .join(" ");

    let sql = sql.trim();

    // Remove everything after the last semicolon (handles trailing comments/statements)
    let sql = if let Some(pos) = sql.rfind(';') {
        sql[..pos].trim().to_string()
    } else {
        sql.to_string()
    };

    // Validate SQL is not empty
    if sql.is_empty() {
        anyhow::bail!("Query is empty after removing comments");
    }

    // Check if query already has a LIMIT/TOP/FETCH clause (case-insensitive)
    // If so, disable dbt's limit injection to avoid conflicts
    let sql_upper = sql.to_uppercase();
    let has_limit = sql_upper.contains(" LIMIT ")
        || sql_upper.contains(" TOP ")
        || sql_upper.contains("SELECT TOP ")
        || sql_upper.contains(" FETCH FIRST ")
        || sql_upper.contains(" FETCH NEXT ")
        || sql_upper.contains(" ROWNUM");

    // Use dbt's --limit flag (it handles adapter-specific syntax)
    // -1 disables limit injection
    let limit_arg = if has_limit {
        "-1".to_string() // User has their own limit, don't inject
    } else {
        match limit {
            QueryLimit::Limit(n) => n.to_string(),
            QueryLimit::NoLimit => "-1".to_string(),
        }
    };

    let profiles_dir = dirs::home_dir()
        .context("Could not find home directory")?
        .join(".dbt");

    let output = Command::new(&dbt_path)
        .arg("show")
        .arg("--inline")
        .arg(&sql)
        .arg("--project-dir")
        .arg(stub_dir.to_str().unwrap())
        .arg("--profiles-dir")
        .arg(profiles_dir.to_str().unwrap())
        .arg("--profile")
        .arg(&connection.profile)
        .arg("--target")
        .arg(&connection.target)
        .arg("--output")
        .arg("json")
        .arg("--threads")
        .arg("1")
        .arg("--limit")
        .arg(&limit_arg)
        .arg("--no-partial-parse")
        .env("DBT_SEND_ANONYMOUS_USAGE_STATS", "false")
        .env("DBT_NO_ANALYTICS", "true")
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .context("Failed to execute dbt show")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        // Combine stderr and stdout for better error context
        let error_msg = if !stderr.is_empty() {
            stderr.to_string()
        } else if !stdout.is_empty() {
            stdout.to_string()
        } else {
            "Unknown error (no output)".to_string()
        };
        anyhow::bail!("{}", error_msg.trim());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_dbt_output(&stdout)
}

fn parse_dbt_output(output: &str) -> Result<QueryResult> {
    // dbt show --output json format: { "show": [{ col: val, ... }, ...] }
    // Log lines may appear before the JSON, so find where JSON starts

    // Strategy 1: Find JSON object after newline (skip log lines)
    if let Some(json_start) = output.find("\n{") {
        let json_str = &output[json_start + 1..];
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(json_str) {
            if let Some(result) = try_parse_json(&json) {
                return Ok(result);
            }
        }
    }

    // Strategy 2: Try line-by-line parsing (fallback)
    for line in output.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('{') {
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(trimmed) {
                if let Some(result) = try_parse_json(&json) {
                    return Ok(result);
                }
            }
        }
    }

    // Strategy 3: Try parsing entire output as JSON
    if let Ok(json) = serde_json::from_str::<serde_json::Value>(output) {
        if let Some(result) = try_parse_json(&json) {
            return Ok(result);
        }
    }

    // Strategy 4: Try table parsing as last resort
    parse_table_output(output)
}

fn try_parse_json(json: &serde_json::Value) -> Option<QueryResult> {
    // Try "show" field first (dbt show --output json format)
    if let Some(show) = json.get("show") {
        if let Some(result) = parse_show_array(show) {
            return Some(result);
        }
    }

    // Try "results" field (older dbt format)
    if let Some(results) = json.get("results") {
        if let Some(arr) = results.as_array() {
            if let Some(first) = arr.first() {
                // Check for agate_table or preview
                if let Some(preview) = first.get("agate_table").or(first.get("preview")) {
                    if let Some(result) = parse_show_array(preview) {
                        return Some(result);
                    }
                }
            }
        }
    }

    // Try "data" field
    if let Some(data) = json.get("data") {
        if let Some(result) = parse_show_array(data) {
            return Some(result);
        }
    }

    None
}

fn parse_show_array(data: &serde_json::Value) -> Option<QueryResult> {
    let arr = data.as_array()?;

    if arr.is_empty() {
        return Some(QueryResult {
            columns: vec![],
            rows: vec![],
        });
    }

    // Extract columns from first row's keys
    let first_row = arr.first()?.as_object()?;
    let columns: Vec<String> = first_row.keys().cloned().collect();

    // Extract rows as values in column order
    let rows: Vec<Vec<String>> = arr
        .iter()
        .filter_map(|row| {
            let obj = row.as_object()?;
            Some(
                columns
                    .iter()
                    .map(|col| {
                        obj.get(col)
                            .map(|v| match v {
                                serde_json::Value::String(s) => s.clone(),
                                serde_json::Value::Null => "NULL".to_string(),
                                other => other.to_string(),
                            })
                            .unwrap_or_default()
                    })
                    .collect(),
            )
        })
        .collect();

    Some(QueryResult { columns, rows })
}

fn parse_table_output(output: &str) -> Result<QueryResult> {
    // Parse ASCII table output from dbt show (non-JSON mode)
    // Format:
    // | col1 | col2 |
    // | ---- | ---- |
    // | val1 | val2 |

    let lines: Vec<&str> = output
        .lines()
        .filter(|l| l.trim().starts_with('|'))
        .collect();

    if lines.len() < 2 {
        return Ok(QueryResult {
            columns: vec![],
            rows: vec![],
        });
    }

    let parse_row = |line: &str| -> Vec<String> {
        line.split('|')
            .filter(|s| !s.is_empty())
            .map(|s| s.trim().to_string())
            .collect()
    };

    let columns = parse_row(lines[0]);

    // Skip header and separator line
    let rows: Vec<Vec<String>> = lines
        .iter()
        .skip(2)
        .map(|line| parse_row(line))
        .filter(|row| !row.is_empty() && !row[0].chars().all(|c| c == '-'))
        .collect();

    Ok(QueryResult { columns, rows })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_table_output() {
        let output = r#"
| id | name    | active |
| -- | ------- | ------ |
| 1  | Alice   | true   |
| 2  | Bob     | false  |
"#;

        let result = parse_table_output(output).unwrap();
        assert_eq!(result.columns, vec!["id", "name", "active"]);
        assert_eq!(result.rows.len(), 2);
        assert_eq!(result.rows[0], vec!["1", "Alice", "true"]);
    }
}
