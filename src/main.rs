mod app;
mod config;
mod executor;
mod input;
mod nvim;
mod profiles;
mod ui;
mod venv;

use anyhow::Result;
use crossterm::{
    event::{
        self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
        Event, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
    },
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    ExecutableCommand,
};
use ratatui::prelude::*;
use sqlformat::{format, FormatOptions, QueryParams};
use std::io::stdout;
use std::sync::Arc;

use crate::app::{App, Command, CopyDelimiter, DragMode, Modal, Pane, QueryLimit};
use crate::input::key_to_nvim_input;
use crate::nvim::EmbeddedNvim;

/// Format SQL query with proper indentation and separators between queries
fn format_sql(sql: &str) -> String {
    // First, remove existing --**-- separators so we don't duplicate them
    let sql_clean = sql
        .lines()
        .filter(|line| line.trim() != "--**--")
        .collect::<Vec<_>>()
        .join("\n");

    let options = FormatOptions {
        indent: sqlformat::Indent::Spaces(2),
        uppercase: Some(false),
        lines_between_queries: 1,
        ..Default::default()
    };
    let formatted = format(&sql_clean, &QueryParams::None, &options);

    // Add --**-- separator between queries
    // Look for pattern: semicolon followed by newlines and then a new statement
    let mut result = String::new();
    let mut after_semicolon = false;
    let mut newline_count = 0;

    for c in formatted.chars() {
        if c == ';' {
            result.push(c);
            after_semicolon = true;
            newline_count = 0;
        } else if after_semicolon {
            if c == '\n' {
                result.push(c);
                newline_count += 1;
            } else if !c.is_whitespace() {
                // Found start of new query after semicolon
                if newline_count > 0 {
                    // Insert separator with blank lines above and below
                    result.push_str("\n--**--\n\n");
                }
                result.push(c);
                after_semicolon = false;
                newline_count = 0;
            } else {
                result.push(c);
            }
        } else {
            result.push(c);
        }
    }

    result
}

/// Build completion words from schema metadata for SQL autocomplete
fn build_completion_words(schemas: &[executor::SchemaInfo]) -> Vec<String> {
    let mut words = Vec::new();

    // SQL keywords
    let keywords = [
        "SELECT",
        "FROM",
        "WHERE",
        "AND",
        "OR",
        "NOT",
        "IN",
        "LIKE",
        "BETWEEN",
        "IS",
        "NULL",
        "TRUE",
        "FALSE",
        "AS",
        "ON",
        "JOIN",
        "LEFT",
        "RIGHT",
        "INNER",
        "OUTER",
        "FULL",
        "CROSS",
        "GROUP",
        "BY",
        "ORDER",
        "ASC",
        "DESC",
        "HAVING",
        "LIMIT",
        "OFFSET",
        "UNION",
        "ALL",
        "DISTINCT",
        "CASE",
        "WHEN",
        "THEN",
        "ELSE",
        "END",
        "INSERT",
        "INTO",
        "VALUES",
        "UPDATE",
        "SET",
        "DELETE",
        "CREATE",
        "TABLE",
        "VIEW",
        "INDEX",
        "DROP",
        "ALTER",
        "ADD",
        "WITH",
        "CTE",
        "OVER",
        "PARTITION",
        "ROW_NUMBER",
        "RANK",
        "DENSE_RANK",
        "LAG",
        "LEAD",
        "FIRST_VALUE",
        "LAST_VALUE",
        "SUM",
        "COUNT",
        "AVG",
        "MIN",
        "MAX",
        "COALESCE",
        "NULLIF",
        "CAST",
        "CONVERT",
        "DATE",
        "TIMESTAMP",
        "VARCHAR",
        "INTEGER",
        "BOOLEAN",
        "FLOAT",
        "DECIMAL",
        "TEXT",
    ];
    words.extend(keywords.iter().map(|s| s.to_string()));

    // Add schema names, table names, and column names
    for schema in schemas {
        words.push(schema.name.clone());

        for table in &schema.tables {
            // Add table name
            words.push(table.name.clone());
            // Add fully qualified name
            words.push(format!("{}.{}", schema.name, table.name));

            // Add column names
            for col in &table.columns {
                words.push(col.name.clone());
            }
        }
    }

    // Deduplicate
    words.sort();
    words.dedup();
    words
}

#[tokio::main]
async fn main() -> Result<()> {
    // Setup terminal
    enable_raw_mode()?;
    stdout().execute(EnterAlternateScreen)?;
    stdout().execute(EnableMouseCapture)?;
    stdout().execute(EnableBracketedPaste)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;

    // Get terminal size for nvim
    // At startup, results aren't visible, so editor gets full height
    // Layout: status(1) + editor(full) - borders
    let size = terminal.size()?;
    let editor_height = size.height.saturating_sub(4).max(5) as usize; // -1 status, -2 borders, -1 help line
    let editor_width = (size.width.saturating_sub(2)) as usize; // Account for borders

    // Spawn embedded neovim (starts in insert mode by default)
    let nvim = EmbeddedNvim::spawn(editor_width, editor_height).await?;

    // Load available connections
    let connections = profiles::load_profiles()?;

    // Create app state with shared screen and connections
    let mut app = App::new(Arc::clone(&nvim.screen), connections);

    // Load auto-saved editor content from last session
    let autosave_content = config::load_autosave();
    if !autosave_content.is_empty() {
        let _ = nvim.set_buffer_contents(&autosave_content).await;
    }

    // Try to auto-connect to last used connection, or open picker
    if !app.connections.is_empty() {
        if let Some(ref last_conn_key) = app.app_state.last_connection.clone() {
            // Find the last connection
            if let Some(conn) = app
                .connections
                .iter()
                .find(|c| format!("{}:{}", c.profile, c.target) == *last_conn_key)
                .cloned()
            {
                app.pending_connection = Some(conn);
            } else {
                app.open_connection_picker();
            }
        } else {
            app.open_connection_picker();
        }
    }

    // Track pending schema fetch (runs in background)
    let mut pending_schema: Option<
        tokio::sync::oneshot::Receiver<Result<Vec<executor::SchemaInfo>>>,
    > = None;
    // Command queued from the palette; handled after the input match (survives the
    // `continue` used to swallow other keys while a modal is open).
    let mut pending_command: Option<Command> = None;
    // SQL queued for execution (from Ctrl+e or the history picker), handled after
    // the input match so it can run regardless of which path queued it.
    let mut pending_run_query: Option<String> = None;

    // Main loop
    loop {
        // Process pending connection (install adapter if needed)
        if let Some(conn) = app.pending_connection.take() {
            app.status = format!("Setting up {}...", conn.adapter_package());
            app.loading = true;
            terminal.draw(|frame| ui::render(frame, &mut app))?;

            match executor::ensure_adapter(&conn).await {
                Ok(()) => {
                    app.set_connection(conn.clone());
                    app.status = "Connected - loading schema...".to_string();
                    app.schema_loading = true;

                    // Spawn schema fetch in background so UI stays responsive
                    let (tx, rx) = tokio::sync::oneshot::channel();
                    let conn_clone = conn.clone();
                    tokio::spawn(async move {
                        let result = executor::fetch_schema_metadata(&conn_clone).await;
                        let _ = tx.send(result);
                    });
                    pending_schema = Some(rx);
                }
                Err(e) => {
                    app.error = Some(format!("Failed to setup adapter: {}", e));
                    app.status = "Connection failed".to_string();
                }
            }
            app.loading = false;
        }

        // Check for completed schema fetch
        if let Some(ref mut rx) = pending_schema {
            match rx.try_recv() {
                Ok(Ok(schemas)) => {
                    let count = schemas.len();
                    let completion_words = build_completion_words(&schemas);
                    if let Err(e) = nvim.setup_sql_completion(&completion_words).await {
                        app.status =
                            format!("Connected ({} schemas, completion error: {})", count, e);
                    } else {
                        app.status = format!("Connected ({} schemas)", count);
                    }
                    app.set_schema_cache(schemas);
                    app.schema_loading = false;
                    pending_schema = None;
                }
                Ok(Err(e)) => {
                    app.status = format!("Connected (schema error: {})", e);
                    app.schema_loading = false;
                    pending_schema = None;
                }
                Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {
                    // Still loading, continue
                }
                Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                    app.status = "Connected (schema fetch cancelled)".to_string();
                    app.schema_loading = false;
                    pending_schema = None;
                }
            }
        }

        // Draw UI
        terminal.draw(|frame| ui::render(frame, &mut app))?;

        // Handle input with a short poll timeout
        if event::poll(std::time::Duration::from_millis(16))? {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    // Handle modal keys first
                    if app.modal != Modal::None {
                        match app.modal {
                            Modal::ConnectionPicker => match key.code {
                                KeyCode::Esc => app.close_modal(),
                                KeyCode::Enter => app.picker_select(),
                                KeyCode::Down => app.picker_next(),
                                KeyCode::Up => app.picker_prev(),
                                KeyCode::Backspace => app.picker_filter_pop(),
                                KeyCode::Char(c) => {
                                    if c == 'j' && key.modifiers.contains(KeyModifiers::CONTROL) {
                                        app.picker_next();
                                    } else if c == 'k'
                                        && key.modifiers.contains(KeyModifiers::CONTROL)
                                    {
                                        app.picker_prev();
                                    } else {
                                        app.picker_filter_push(c);
                                    }
                                }
                                _ => {}
                            },
                            Modal::Filter => match key.code {
                                KeyCode::Esc | KeyCode::Enter => app.close_modal(),
                                KeyCode::Backspace => app.filter_pop(),
                                KeyCode::Char(c) => app.filter_push(c),
                                _ => {}
                            },
                            Modal::SortPicker => {
                                match key.code {
                                    KeyCode::Esc => app.close_modal(),
                                    KeyCode::Char('j') | KeyCode::Down => app.sort_picker_next(),
                                    KeyCode::Char('k') | KeyCode::Up => app.sort_picker_prev(),
                                    KeyCode::Enter | KeyCode::Char(' ') => app.sort_picker_toggle(),
                                    KeyCode::Char('a') => {
                                        // Set ascending
                                        let col = app.sort_picker_index;
                                        if let Some(spec) =
                                            app.sort_specs.iter_mut().find(|s| s.column == col)
                                        {
                                            spec.ascending = true;
                                            app.apply_filter_and_sort();
                                        }
                                    }
                                    KeyCode::Char('d') => {
                                        // Set descending
                                        let col = app.sort_picker_index;
                                        if let Some(spec) =
                                            app.sort_specs.iter_mut().find(|s| s.column == col)
                                        {
                                            spec.ascending = false;
                                            app.apply_filter_and_sort();
                                        }
                                    }
                                    KeyCode::Char('c') => app.sort_clear(),
                                    _ => {}
                                }
                            }
                            Modal::ExplorerFilter => match key.code {
                                KeyCode::Esc | KeyCode::Enter => app.close_modal(),
                                KeyCode::Backspace => app.explorer_filter_pop(),
                                KeyCode::Char(c) => app.explorer_filter_push(c),
                                _ => {}
                            },
                            Modal::SaveQuery(ref mut filename) => match key.code {
                                KeyCode::Esc => app.close_modal(),
                                KeyCode::Enter => {
                                    if !filename.is_empty() {
                                        let sql = nvim.get_buffer_contents().await?;
                                        match config::save_query(filename, &sql) {
                                            Ok(path) => {
                                                app.status = format!("Saved: {}", path.display());
                                            }
                                            Err(e) => {
                                                app.status = format!("Save failed: {}", e);
                                            }
                                        }
                                    }
                                    app.close_modal();
                                }
                                KeyCode::Backspace => {
                                    filename.pop();
                                }
                                KeyCode::Char(c) => {
                                    filename.push(c);
                                }
                                _ => {}
                            },
                            Modal::LoadQuery => {
                                match key.code {
                                    KeyCode::Esc => app.close_modal(),
                                    KeyCode::Char('j') | KeyCode::Down => {
                                        if !app.saved_queries.is_empty() {
                                            app.load_query_index = (app.load_query_index + 1)
                                                % app.saved_queries.len();
                                        }
                                    }
                                    KeyCode::Char('k') | KeyCode::Up => {
                                        if !app.saved_queries.is_empty() {
                                            app.load_query_index = app
                                                .load_query_index
                                                .checked_sub(1)
                                                .unwrap_or(app.saved_queries.len() - 1);
                                        }
                                    }
                                    KeyCode::Enter => {
                                        if let Some(filename) =
                                            app.saved_queries.get(app.load_query_index)
                                        {
                                            match config::load_query(filename) {
                                                Ok(content) => {
                                                    nvim.set_buffer_contents(&content).await?;
                                                    app.status = format!("Loaded: {}", filename);
                                                }
                                                Err(e) => {
                                                    app.status = format!("Load failed: {}", e);
                                                }
                                            }
                                        }
                                        app.close_modal();
                                    }
                                    KeyCode::Char('d') | KeyCode::Delete => {
                                        // Delete selected query file
                                        if let Some(filename) =
                                            app.saved_queries.get(app.load_query_index).cloned()
                                        {
                                            match config::delete_query(&filename) {
                                                Ok(()) => {
                                                    app.status = format!("Deleted: {}", filename);
                                                    // Refresh list
                                                    app.saved_queries =
                                                        config::list_saved_queries()
                                                            .unwrap_or_default();
                                                    // Adjust index if needed
                                                    if !app.saved_queries.is_empty() {
                                                        app.load_query_index = app
                                                            .load_query_index
                                                            .min(app.saved_queries.len() - 1);
                                                    } else {
                                                        app.load_query_index = 0;
                                                        app.close_modal();
                                                    }
                                                }
                                                Err(e) => {
                                                    app.status = format!("Delete failed: {}", e);
                                                }
                                            }
                                        }
                                    }
                                    _ => {}
                                }
                            }
                            Modal::Help => {
                                // Any key closes help
                                app.close_modal();
                            }
                            Modal::CopyColumns => {
                                match key.code {
                                    KeyCode::Esc => app.close_modal(),
                                    KeyCode::Char('j') | KeyCode::Down => {
                                        if !app.columns.is_empty() {
                                            app.export_picker_index =
                                                (app.export_picker_index + 1) % app.columns.len();
                                        }
                                    }
                                    KeyCode::Char('k') | KeyCode::Up => {
                                        if !app.columns.is_empty() {
                                            app.export_picker_index = app
                                                .export_picker_index
                                                .checked_sub(1)
                                                .unwrap_or(app.columns.len() - 1);
                                        }
                                    }
                                    KeyCode::Char(' ') => {
                                        if let Some(selected) =
                                            app.export_columns.get_mut(app.export_picker_index)
                                        {
                                            *selected = !*selected;
                                        }
                                    }
                                    KeyCode::Char('a') => {
                                        // Toggle all
                                        let all_selected = app.export_columns.iter().all(|&x| x);
                                        for col in &mut app.export_columns {
                                            *col = !all_selected;
                                        }
                                    }
                                    KeyCode::Enter => {
                                        if !app.export_columns.iter().any(|&x| x) {
                                            app.status = "Select at least one column".to_string();
                                        } else {
                                            // Proceed to copy options
                                            app.modal = Modal::CopyOptions;
                                        }
                                    }
                                    _ => {}
                                }
                            }
                            Modal::CopyOptions => {
                                match key.code {
                                    KeyCode::Esc => app.close_modal(),
                                    KeyCode::Char('1') | KeyCode::Char('c') => {
                                        app.copy_delimiter = CopyDelimiter::Comma;
                                    }
                                    KeyCode::Char('2') | KeyCode::Char('t') => {
                                        app.copy_delimiter = CopyDelimiter::Tab;
                                    }
                                    KeyCode::Char('3') | KeyCode::Char('p') => {
                                        app.copy_delimiter = CopyDelimiter::Pipe;
                                    }
                                    KeyCode::Char('h') => {
                                        app.copy_include_header = !app.copy_include_header;
                                    }
                                    KeyCode::Enter | KeyCode::Char('y') => {
                                        // Copy with selected columns and options
                                        let selected_indices: Vec<usize> = app
                                            .export_columns
                                            .iter()
                                            .enumerate()
                                            .filter_map(
                                                |(i, &selected)| {
                                                    if selected {
                                                        Some(i)
                                                    } else {
                                                        None
                                                    }
                                                },
                                            )
                                            .collect();

                                        let rows = app.get_selected_rows();
                                        let filtered_rows: Vec<Vec<String>> = rows
                                            .iter()
                                            .map(|r| {
                                                selected_indices
                                                    .iter()
                                                    .filter_map(|&i| r.get(i).cloned())
                                                    .collect()
                                            })
                                            .collect();

                                        let filtered_cols: Vec<String> = selected_indices
                                            .iter()
                                            .filter_map(|&i| app.columns.get(i).cloned())
                                            .collect();

                                        let text = app.format_rows_for_copy_with_cols(
                                            &filtered_rows,
                                            &filtered_cols,
                                            app.copy_include_header,
                                            app.copy_delimiter,
                                        );
                                        match arboard::Clipboard::new() {
                                            Ok(mut clipboard) => {
                                                if clipboard.set_text(&text).is_ok() {
                                                    let count = rows.len();
                                                    let col_count = selected_indices.len();
                                                    app.status = format!(
                                                        "{} row{} x {} col{} copied",
                                                        count,
                                                        if count == 1 { "" } else { "s" },
                                                        col_count,
                                                        if col_count == 1 { "" } else { "s" }
                                                    );
                                                    app.clear_selection();
                                                } else {
                                                    app.status = "Failed to copy".to_string();
                                                }
                                            }
                                            Err(_) => {
                                                app.status = "Clipboard not available".to_string();
                                            }
                                        }
                                        app.close_modal();
                                    }
                                    _ => {}
                                }
                            }
                            Modal::ExportColumns => {
                                match key.code {
                                    KeyCode::Esc => app.close_modal(),
                                    KeyCode::Char('j') | KeyCode::Down => {
                                        if !app.columns.is_empty() {
                                            app.export_picker_index =
                                                (app.export_picker_index + 1) % app.columns.len();
                                        }
                                    }
                                    KeyCode::Char('k') | KeyCode::Up => {
                                        if !app.columns.is_empty() {
                                            app.export_picker_index = app
                                                .export_picker_index
                                                .checked_sub(1)
                                                .unwrap_or(app.columns.len() - 1);
                                        }
                                    }
                                    KeyCode::Char(' ') => {
                                        // Toggle current column
                                        if let Some(selected) =
                                            app.export_columns.get_mut(app.export_picker_index)
                                        {
                                            *selected = !*selected;
                                        }
                                    }
                                    KeyCode::Char('a') => {
                                        // Toggle all
                                        let all_selected = app.export_columns.iter().all(|&x| x);
                                        for col in &mut app.export_columns {
                                            *col = !all_selected;
                                        }
                                    }
                                    KeyCode::Enter => {
                                        // Check at least one column is selected
                                        if !app.export_columns.iter().any(|&x| x) {
                                            app.status = "Select at least one column".to_string();
                                        } else {
                                            app.close_modal();

                                            // Temporarily leave TUI for native dialog
                                            disable_raw_mode()?;
                                            stdout().execute(LeaveAlternateScreen)?;
                                            stdout().execute(DisableMouseCapture)?;
                                            stdout().execute(DisableBracketedPaste)?;

                                            // Default filename with timestamp
                                            let timestamp =
                                                chrono::Local::now().format("%Y%m%d_%H%M%S");
                                            let default_name = format!("export_{}.csv", timestamp);

                                            // Show native save dialog
                                            let file_path = rfd::FileDialog::new()
                                                .set_file_name(&default_name)
                                                .add_filter("CSV", &["csv"])
                                                .save_file();

                                            // Re-enter TUI
                                            enable_raw_mode()?;
                                            stdout().execute(EnterAlternateScreen)?;
                                            stdout().execute(EnableMouseCapture)?;
                                            stdout().execute(EnableBracketedPaste)?;
                                            terminal.clear()?;

                                            if let Some(path) = file_path {
                                                // Filter columns based on selection
                                                let selected_indices: Vec<usize> = app
                                                    .export_columns
                                                    .iter()
                                                    .enumerate()
                                                    .filter_map(|(i, &selected)| {
                                                        if selected {
                                                            Some(i)
                                                        } else {
                                                            None
                                                        }
                                                    })
                                                    .collect();

                                                let export_columns: Vec<String> = selected_indices
                                                    .iter()
                                                    .filter_map(|&i| app.columns.get(i).cloned())
                                                    .collect();

                                                let rows: Vec<Vec<String>> = app
                                                    .get_display_rows()
                                                    .iter()
                                                    .map(|r| {
                                                        selected_indices
                                                            .iter()
                                                            .filter_map(|&i| r.get(i).cloned())
                                                            .collect()
                                                    })
                                                    .collect();

                                                match config::export_csv_to_path(
                                                    &export_columns,
                                                    &rows,
                                                    &path,
                                                ) {
                                                    Ok(()) => {
                                                        let filename = path
                                                            .file_name()
                                                            .map(|f| {
                                                                f.to_string_lossy().to_string()
                                                            })
                                                            .unwrap_or_default();
                                                        let col_count = selected_indices.len();
                                                        app.status = format!(
                                                            "Exported {} rows x {} cols to {}",
                                                            rows.len(),
                                                            col_count,
                                                            filename
                                                        );
                                                        #[cfg(target_os = "macos")]
                                                        {
                                                            let _ =
                                                                std::process::Command::new("open")
                                                                    .arg("-R")
                                                                    .arg(&path)
                                                                    .spawn();
                                                        }
                                                    }
                                                    Err(e) => {
                                                        app.status =
                                                            format!("Export failed: {}", e);
                                                    }
                                                }
                                            } else {
                                                app.status = "Export cancelled".to_string();
                                            }
                                        }
                                    }
                                    _ => {}
                                }
                            }
                            Modal::LimitPicker => {
                                const LIMIT_OPTIONS: [QueryLimit; 5] = [
                                    QueryLimit::Limit(100),
                                    QueryLimit::Limit(1000),
                                    QueryLimit::Limit(10000),
                                    QueryLimit::Limit(100000),
                                    QueryLimit::NoLimit,
                                ];
                                match key.code {
                                    KeyCode::Esc => app.close_modal(),
                                    KeyCode::Char('j') | KeyCode::Down => {
                                        app.limit_picker_index =
                                            (app.limit_picker_index + 1) % LIMIT_OPTIONS.len();
                                    }
                                    KeyCode::Char('k') | KeyCode::Up => {
                                        app.limit_picker_index = app
                                            .limit_picker_index
                                            .checked_sub(1)
                                            .unwrap_or(LIMIT_OPTIONS.len() - 1);
                                    }
                                    KeyCode::Enter | KeyCode::Char(' ') => {
                                        app.query_limit = LIMIT_OPTIONS[app.limit_picker_index];
                                        app.status =
                                            format!("Limit: {}", app.query_limit.display());
                                        app.close_modal();
                                    }
                                    KeyCode::Char('1') => {
                                        app.query_limit = QueryLimit::Limit(100);
                                        app.status =
                                            format!("Limit: {}", app.query_limit.display());
                                        app.close_modal();
                                    }
                                    KeyCode::Char('2') => {
                                        app.query_limit = QueryLimit::Limit(1000);
                                        app.status =
                                            format!("Limit: {}", app.query_limit.display());
                                        app.close_modal();
                                    }
                                    KeyCode::Char('3') => {
                                        app.query_limit = QueryLimit::Limit(10000);
                                        app.status =
                                            format!("Limit: {}", app.query_limit.display());
                                        app.close_modal();
                                    }
                                    KeyCode::Char('4') => {
                                        app.query_limit = QueryLimit::Limit(100000);
                                        app.status =
                                            format!("Limit: {}", app.query_limit.display());
                                        app.close_modal();
                                    }
                                    KeyCode::Char('5') | KeyCode::Char('a') => {
                                        app.query_limit = QueryLimit::NoLimit;
                                        app.status =
                                            format!("Limit: {}", app.query_limit.display());
                                        app.close_modal();
                                    }
                                    KeyCode::Char('c') => {
                                        // Custom limit entry
                                        app.modal = Modal::LimitCustom(String::new());
                                    }
                                    _ => {}
                                }
                            }
                            Modal::LimitCustom(ref mut input) => match key.code {
                                KeyCode::Esc => app.modal = Modal::LimitPicker,
                                KeyCode::Enter => {
                                    if let Ok(n) = input.parse::<usize>() {
                                        if n > 0 {
                                            app.query_limit = QueryLimit::Limit(n);
                                            app.status =
                                                format!("Limit: {}", app.query_limit.display());
                                            app.close_modal();
                                        } else {
                                            app.status = "Limit must be > 0".to_string();
                                        }
                                    } else {
                                        app.status = "Invalid number".to_string();
                                    }
                                }
                                KeyCode::Backspace => {
                                    input.pop();
                                }
                                KeyCode::Char(c) if c.is_ascii_digit()
                                    && input.len() < 10 => {
                                        input.push(c);
                                    }
                                _ => {}
                            },
                            Modal::CellDetail(row_idx, col_idx) => {
                                match key.code {
                                    KeyCode::Esc | KeyCode::Char('q') => app.close_modal(),
                                    KeyCode::Char('y') => {
                                        // Copy cell value to clipboard
                                        let display_rows = app.get_display_rows();
                                        if let Some(row) = display_rows.get(row_idx) {
                                            if let Some(cell) = row.get(col_idx) {
                                                if let Ok(mut ctx) = arboard::Clipboard::new() {
                                                    let _ = ctx.set_text(cell.clone());
                                                    app.status = "Copied to clipboard".to_string();
                                                }
                                            }
                                        }
                                        app.close_modal();
                                    }
                                    KeyCode::Left | KeyCode::Char('h') => {
                                        // Move to previous column
                                        if col_idx > 0 {
                                            app.modal = Modal::CellDetail(row_idx, col_idx - 1);
                                        }
                                    }
                                    KeyCode::Right | KeyCode::Char('l') => {
                                        // Move to next column
                                        if col_idx + 1 < app.columns.len() {
                                            app.modal = Modal::CellDetail(row_idx, col_idx + 1);
                                        }
                                    }
                                    KeyCode::Up | KeyCode::Char('k') => {
                                        // Move to previous row
                                        if row_idx > 0 {
                                            app.modal = Modal::CellDetail(row_idx - 1, col_idx);
                                        }
                                    }
                                    KeyCode::Down | KeyCode::Char('j') => {
                                        // Move to next row
                                        let total = app.filtered_row_count();
                                        if row_idx + 1 < total {
                                            app.modal = Modal::CellDetail(row_idx + 1, col_idx);
                                        }
                                    }
                                    _ => {}
                                }
                            }
                            Modal::CancelConfirm => match key.code {
                                KeyCode::Char('y') | KeyCode::Char('Y') => {
                                    app.cancel_requested = true;
                                    app.close_modal();
                                }
                                KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                                    app.close_modal();
                                }
                                _ => {}
                            },
                            Modal::ColumnStats(col_idx) => {
                                match key.code {
                                    KeyCode::Esc | KeyCode::Char('q') => app.close_modal(),
                                    KeyCode::Char('h') => {
                                        // Hide this column
                                        app.hidden_columns.insert(col_idx);
                                        app.status = format!(
                                            "Hidden column {} (Ctrl+H to manage, {} hidden)",
                                            col_idx,
                                            app.hidden_columns.len()
                                        );
                                        app.modal = Modal::None;
                                    }
                                    _ => {}
                                }
                            }
                            Modal::HiddenColumns => {
                                match key.code {
                                    KeyCode::Esc | KeyCode::Char('q') => app.close_modal(),
                                    KeyCode::Up | KeyCode::Char('k') => {
                                        if app.hidden_columns_index > 0 {
                                            app.hidden_columns_index -= 1;
                                        }
                                    }
                                    KeyCode::Down | KeyCode::Char('j') => {
                                        if app.hidden_columns_index + 1 < app.columns.len() {
                                            app.hidden_columns_index += 1;
                                        }
                                    }
                                    KeyCode::Enter | KeyCode::Char(' ') => {
                                        // Toggle visibility
                                        let idx = app.hidden_columns_index;
                                        if app.hidden_columns.contains(&idx) {
                                            app.hidden_columns.remove(&idx);
                                        } else {
                                            app.hidden_columns.insert(idx);
                                        }
                                    }
                                    KeyCode::Char('a') => {
                                        // Show all
                                        app.hidden_columns.clear();
                                    }
                                    _ => {}
                                }
                            }
                            Modal::CommandPalette => {
                                match key.code {
                                    KeyCode::Esc => app.close_modal(),
                                    KeyCode::Up | KeyCode::Char('k')
                                        if key.modifiers.contains(KeyModifiers::CONTROL) =>
                                    {
                                        app.palette_prev();
                                    }
                                    KeyCode::Down | KeyCode::Char('j')
                                        if key.modifiers.contains(KeyModifiers::CONTROL) =>
                                    {
                                        app.palette_next();
                                    }
                                    KeyCode::Up => app.palette_prev(),
                                    KeyCode::Down => app.palette_next(),
                                    KeyCode::Enter => {
                                        // Execute selected command
                                        if let Some(cmd) = app.palette_selected_command() {
                                            app.close_modal();
                                            pending_command = Some(cmd);
                                        }
                                    }
                                    KeyCode::Backspace => app.palette_filter_pop(),
                                    KeyCode::Char(c) => app.palette_filter_push(c),
                                    _ => {}
                                }
                            }
                            Modal::HistoryPicker => {
                                match key.code {
                                    KeyCode::Esc => app.close_modal(),
                                    KeyCode::Up | KeyCode::Char('k')
                                        if key.modifiers.contains(KeyModifiers::CONTROL) =>
                                    {
                                        app.history_picker_prev();
                                    }
                                    KeyCode::Down | KeyCode::Char('j')
                                        if key.modifiers.contains(KeyModifiers::CONTROL) =>
                                    {
                                        app.history_picker_next();
                                    }
                                    KeyCode::Up => app.history_picker_prev(),
                                    KeyCode::Down => app.history_picker_next(),
                                    KeyCode::Enter => {
                                        // Run the selected query without touching the editor
                                        if let Some(query) = app.history_picker_selected() {
                                            app.close_modal();
                                            pending_run_query = Some(query);
                                        }
                                    }
                                    KeyCode::Char('o')
                                        if key.modifiers.contains(KeyModifiers::CONTROL) =>
                                    {
                                        // Load into the editor, replacing the buffer
                                        if let Some(query) = app.history_picker_selected() {
                                            nvim.set_buffer_contents(&query).await?;
                                            app.close_modal();
                                            app.focus = Pane::Editor;
                                            app.status = "Loaded query into editor".to_string();
                                        }
                                    }
                                    KeyCode::Char('a')
                                        if key.modifiers.contains(KeyModifiers::CONTROL) =>
                                    {
                                        // Append to the editor buffer (separated as a new statement)
                                        if let Some(query) = app.history_picker_selected() {
                                            let current = nvim.get_buffer_contents().await?;
                                            let combined = if current.trim().is_empty() {
                                                query
                                            } else {
                                                format!("{}\n\n--**--\n\n{}", current.trim_end(), query)
                                            };
                                            nvim.set_buffer_contents(&combined).await?;
                                            app.close_modal();
                                            app.focus = Pane::Editor;
                                            app.status = "Appended query to editor".to_string();
                                        }
                                    }
                                    KeyCode::Backspace => app.history_picker_filter_pop(),
                                    KeyCode::Char(c) => app.history_picker_filter_push(c),
                                    _ => {}
                                }
                            }
                            Modal::None => {}
                        }
                        continue; // Don't process other keys when modal is open
                    }

                    // Global keybindings that work regardless of focus
                    if key.modifiers.contains(KeyModifiers::CONTROL) {
                        match key.code {
                            KeyCode::Char('q') => {
                                app.should_quit = true;
                            }
                            KeyCode::Char('c') => {
                                // Open connection picker
                                app.open_connection_picker();
                            }
                            KeyCode::Char('e') | KeyCode::Enter => {
                                // Execute query with Ctrl+e or Ctrl+Enter.
                                // Get SQL: visual selection > query at cursor > full buffer,
                                // then defer execution to the pending_run_query handler.
                                let sql = match nvim.get_visual_selection().await? {
                                    Some(selection) if !selection.trim().is_empty() => selection,
                                    _ => {
                                        let query = nvim.get_query_at_cursor().await?;
                                        if !query.is_empty() {
                                            query
                                        } else {
                                            nvim.get_buffer_contents().await?
                                        }
                                    }
                                };
                                pending_run_query = Some(sql);
                            }
                            KeyCode::Char('d') => {
                                if app.focus == Pane::Results {
                                    app.results_page_down();
                                } else {
                                    let input = key_to_nvim_input(&key);
                                    nvim.send_input(&input).await?;
                                }
                            }
                            KeyCode::Char('u') => {
                                if app.focus == Pane::Results {
                                    app.results_page_up();
                                } else {
                                    let input = key_to_nvim_input(&key);
                                    nvim.send_input(&input).await?;
                                }
                            }
                            KeyCode::Char('b') => {
                                // Toggle schema explorer
                                let was_visible = app.explorer_visible;
                                if app.explorer_visible {
                                    // Just hide it
                                    app.explorer_visible = false;
                                    if app.focus == Pane::Explorer {
                                        app.focus = Pane::Editor;
                                    }
                                } else if let Some(_conn) = app.connection.clone() {
                                    // Show explorer if schema is loaded or loading
                                    if app.schema_cache.is_empty() && !app.schema_loading {
                                        // Schema failed to load on connect, show message
                                        app.status =
                                            "Schema not loaded - reconnect to retry".to_string();
                                    } else if app.schema_loading {
                                        // Still loading, show explorer anyway (will populate when done)
                                        app.explorer_visible = true;
                                        app.focus = Pane::Explorer;
                                        app.status = "Schema loading...".to_string();
                                    } else {
                                        app.explorer_visible = true;
                                        app.focus = Pane::Explorer;
                                    }
                                } else {
                                    app.status = "No connection selected".to_string();
                                }
                                // If visibility changed, resize nvim and reset results scroll
                                if was_visible != app.explorer_visible {
                                    app.results_hscroll = 0;
                                    let size = terminal.size()?;
                                    let content_width = if app.explorer_visible {
                                        size.width.saturating_sub(35) // Explorer takes 35 cols
                                    } else {
                                        size.width
                                    };
                                    let content_height = size.height.saturating_sub(2);
                                    let editor_height = if app.results_visible {
                                        ((content_height as u32 * app.split_percent as u32) / 100)
                                            as u16
                                    } else {
                                        content_height
                                    };
                                    let new_height =
                                        editor_height.saturating_sub(2).max(5) as usize;
                                    let new_width = content_width.saturating_sub(2) as usize;
                                    nvim.resize(new_width, new_height).await?;
                                }
                            }
                            KeyCode::Char('f') => {
                                // Format SQL in editor
                                let sql = nvim.get_buffer_contents().await?;
                                let formatted = format_sql(&sql);
                                nvim.set_buffer_contents(&formatted).await?;
                                app.status = "SQL formatted".to_string();
                            }
                            KeyCode::Char('r') => {
                                // Toggle results pane
                                app.toggle_results();
                                // Resize nvim to match new layout
                                let size = terminal.size()?;
                                let content_height = size.height.saturating_sub(2); // -1 status, -1 help
                                let editor_height = if app.results_visible {
                                    // Split view: editor gets split_percent of content area
                                    ((content_height as u32 * app.split_percent as u32) / 100)
                                        as u16
                                } else {
                                    // Full view: editor gets all content area
                                    content_height
                                };
                                let new_height = editor_height.saturating_sub(2).max(5) as usize;
                                let new_width = size.width.saturating_sub(2) as usize;
                                nvim.resize(new_width, new_height).await?;
                            }
                            KeyCode::Char('t') => {
                                // Toggle split direction (horizontal/vertical)
                                app.toggle_split_direction();
                            }
                            KeyCode::Char('p') => {
                                // Open command palette (Ctrl+p; F1 also works)
                                app.open_command_palette();
                            }
                            KeyCode::Char('g') => {
                                // Open query history picker
                                app.open_history_picker();
                            }
                            KeyCode::Char('s') => {
                                // Save query to file
                                app.modal = Modal::SaveQuery(String::new());
                            }
                            KeyCode::Char('o') => {
                                // Load query from file
                                app.saved_queries =
                                    config::list_saved_queries().unwrap_or_default();
                                app.load_query_index = 0;
                                app.modal = Modal::LoadQuery;
                            }
                            KeyCode::Char('x') => {
                                // Export results to CSV - open column picker first
                                if app.columns.is_empty() {
                                    app.status = "No results to export".to_string();
                                } else {
                                    // Initialize all columns as selected
                                    app.export_columns = vec![true; app.columns.len()];
                                    app.export_picker_index = 0;
                                    app.modal = Modal::ExportColumns;
                                }
                            }
                            KeyCode::Char('l') => {
                                // Open limit picker (Ctrl+l)
                                app.modal = Modal::LimitPicker;
                            }
                            _ => {
                                // Forward other Ctrl keys to nvim when editor focused
                                if app.focus == Pane::Editor {
                                    let input = key_to_nvim_input(&key);
                                    nvim.send_input(&input).await?;
                                }
                            }
                        }
                    } else {
                        match key.code {
                            KeyCode::F(1) => {
                                // Open command palette (alias for Ctrl+k)
                                app.open_command_palette();
                            }
                            // In viz mode, Tab cycles the pickers instead of switching panes
                            KeyCode::Tab if !(app.viz_mode && app.focus == Pane::Results) => {
                                app.toggle_focus();
                            }
                            KeyCode::Char('?') => {
                                app.modal = Modal::Help;
                            }
                            _ => {
                                if app.focus == Pane::Explorer {
                                    // Handle explorer navigation
                                    match key.code {
                                        KeyCode::Char('j') | KeyCode::Down => app.explorer_next(),
                                        KeyCode::Char('k') | KeyCode::Up => app.explorer_prev(),
                                        KeyCode::Enter | KeyCode::Char(' ') => {
                                            app.explorer_toggle_node()
                                        }
                                        KeyCode::Char('l') | KeyCode::Right => {
                                            // Expand or move to child
                                            if let Some(node) =
                                                app.explorer_nodes.get(app.explorer_selected)
                                            {
                                                if node.children_count > 0 && !node.expanded {
                                                    app.explorer_toggle_node();
                                                }
                                            }
                                        }
                                        KeyCode::Char('h') | KeyCode::Left => {
                                            // Collapse or move to parent
                                            if let Some(node) =
                                                app.explorer_nodes.get(app.explorer_selected)
                                            {
                                                if node.expanded {
                                                    app.explorer_toggle_node();
                                                }
                                            }
                                        }
                                        KeyCode::Char('y') => {
                                            // Yank full name to status (would need clipboard)
                                            if let Some(name) = app.get_selected_explorer_name() {
                                                app.status = format!("Copied: {}", name);
                                            }
                                        }
                                        KeyCode::Char('i') => {
                                            // Insert into editor
                                            if let Some(name) = app.get_selected_explorer_name() {
                                                nvim.send_input(&format!("a{}", name)).await?;
                                                app.focus = Pane::Editor;
                                            }
                                        }
                                        KeyCode::Char('/') => {
                                            app.modal = Modal::ExplorerFilter;
                                        }
                                        KeyCode::Esc => {
                                            app.explorer_filter_clear();
                                        }
                                        KeyCode::Char('q') => {
                                            app.explorer_visible = false;
                                            app.focus = Pane::Editor;
                                        }
                                        KeyCode::Char('r') => {
                                            // Refresh schema
                                            if let Some(conn) = app.connection.clone() {
                                                app.status = "Refreshing schema...".to_string();
                                                terminal
                                                    .draw(|frame| ui::render(frame, &mut app))?;
                                                match executor::fetch_schema_metadata(&conn).await {
                                                    Ok(schemas) => {
                                                        let count = schemas.len();
                                                        app.set_schema_cache(schemas);
                                                        app.status =
                                                            format!("{} schemas refreshed", count);
                                                    }
                                                    Err(e) => {
                                                        app.status =
                                                            format!("Refresh error: {}", e);
                                                    }
                                                }
                                            }
                                        }
                                        _ => {}
                                    }
                                } else if app.focus == Pane::Editor {
                                    // Forward to nvim when editor focused
                                    let input = key_to_nvim_input(&key);
                                    nvim.send_input(&input).await?;
                                } else if app.focus == Pane::Results {
                                    // Handle pending 'g' for gg command
                                    if app.pending_g {
                                        app.pending_g = false;
                                        if let KeyCode::Char('g') = key.code {
                                            app.results_first();
                                            continue;
                                        }
                                        // Not 'g', fall through to process this key normally
                                    }

                                    // Visualization mode has different keybindings
                                    if app.viz_mode {
                                        match key.code {
                                            KeyCode::Char('v')
                                            | KeyCode::Char('V')
                                            | KeyCode::Esc => {
                                                app.toggle_viz_mode();
                                                app.status = "Table view".to_string();
                                            }
                                            KeyCode::Tab => {
                                                app.viz_picker_next();
                                            }
                                            KeyCode::BackTab => {
                                                app.viz_picker_prev();
                                            }
                                            KeyCode::Char('a') => {
                                                app.cycle_viz_agg_type();
                                            }
                                            KeyCode::Char('j') | KeyCode::Down => {
                                                // Move to next column in current picker
                                                let num_cols = app.columns.len();
                                                if num_cols > 0 {
                                                    match app.viz_config.picker_focus {
                                                        0 => {
                                                            // Group column picker
                                                            let current = app
                                                                .viz_config
                                                                .group_col
                                                                .unwrap_or(0);
                                                            app.set_viz_group_col(
                                                                (current + 1) % num_cols,
                                                            );
                                                        }
                                                        1 => {
                                                            // Value column picker
                                                            let current = app
                                                                .viz_config
                                                                .value_col
                                                                .unwrap_or(0);
                                                            app.set_viz_value_col(Some(
                                                                (current + 1) % num_cols,
                                                            ));
                                                        }
                                                        2 => {
                                                            // Agg type picker
                                                            app.cycle_viz_agg_type();
                                                        }
                                                        _ => {}
                                                    }
                                                }
                                            }
                                            KeyCode::Char('k') | KeyCode::Up => {
                                                // Move to previous column in current picker
                                                let num_cols = app.columns.len();
                                                if num_cols > 0 {
                                                    match app.viz_config.picker_focus {
                                                        0 => {
                                                            let current = app
                                                                .viz_config
                                                                .group_col
                                                                .unwrap_or(0);
                                                            let prev = if current == 0 {
                                                                num_cols - 1
                                                            } else {
                                                                current - 1
                                                            };
                                                            app.set_viz_group_col(prev);
                                                        }
                                                        1 => {
                                                            let current = app
                                                                .viz_config
                                                                .value_col
                                                                .unwrap_or(0);
                                                            let prev = if current == 0 {
                                                                num_cols - 1
                                                            } else {
                                                                current - 1
                                                            };
                                                            app.set_viz_value_col(Some(prev));
                                                        }
                                                        2 => {
                                                            // Cycle agg backwards (just cycle forward for simplicity)
                                                            app.cycle_viz_agg_type();
                                                        }
                                                        _ => {}
                                                    }
                                                }
                                            }
                                            _ => {}
                                        }
                                        continue;
                                    }

                                    // Handle results navigation (table mode)
                                    match key.code {
                                        KeyCode::Char('j') | KeyCode::Down => app.results_next(),
                                        KeyCode::Char('k') | KeyCode::Up => app.results_prev(),
                                        KeyCode::Char('h') | KeyCode::Left => {
                                            app.results_scroll_left()
                                        }
                                        KeyCode::Char('l') | KeyCode::Right => {
                                            app.results_scroll_right()
                                        }
                                        KeyCode::Char('G') => app.results_last(),
                                        KeyCode::Char('g') => {
                                            // Start gg sequence
                                            app.pending_g = true;
                                        }
                                        KeyCode::Char('H') => {
                                            // Open hidden columns modal
                                            if !app.columns.is_empty() {
                                                app.hidden_columns_index = 0;
                                                app.modal = Modal::HiddenColumns;
                                            }
                                        }
                                        KeyCode::Char('M') => app.results_middle(),
                                        KeyCode::Char('L') => app.results_low(),
                                        KeyCode::Char('0') => app.results_scroll_start(),
                                        KeyCode::Char('$') => app.results_scroll_end(),
                                        KeyCode::Char('s') => app.open_sort_picker(),
                                        KeyCode::Char('S') => app.sort_clear(),
                                        KeyCode::Char('/') => {
                                            // Enter filter mode
                                            app.modal = Modal::Filter;
                                        }
                                        KeyCode::Char('V') => {
                                            // Toggle visualization mode
                                            app.toggle_viz_mode();
                                            if app.viz_mode {
                                                app.status = "Viz mode: Tab to switch, j/k to change, a for agg type".to_string();
                                            }
                                        }
                                        KeyCode::Char('v') => {
                                            // Toggle visual selection
                                            app.toggle_selection();
                                            if app.selection_anchor.is_some() {
                                                app.status = "Visual selection started".to_string();
                                            } else {
                                                app.status = "Selection cleared".to_string();
                                            }
                                        }
                                        KeyCode::Char('y') => {
                                            // Open column picker for copy
                                            app.export_columns = vec![true; app.columns.len()];
                                            app.export_picker_index = 0;
                                            app.modal = Modal::CopyColumns;
                                        }
                                        KeyCode::Char('Y') => {
                                            // Quick copy with current settings
                                            let rows = app.get_selected_rows();
                                            let text = app.format_rows_for_copy(
                                                &rows,
                                                app.copy_include_header,
                                                app.copy_delimiter,
                                            );
                                            match arboard::Clipboard::new() {
                                                Ok(mut clipboard) => {
                                                    if clipboard.set_text(&text).is_ok() {
                                                        let count = rows.len();
                                                        app.status = format!(
                                                            "{} row{} copied",
                                                            count,
                                                            if count == 1 { "" } else { "s" }
                                                        );
                                                        app.clear_selection();
                                                    } else {
                                                        app.status = "Failed to copy".to_string();
                                                    }
                                                }
                                                Err(_) => {
                                                    app.status =
                                                        "Clipboard not available".to_string();
                                                }
                                            }
                                        }
                                        KeyCode::Esc => {
                                            if app.selection_anchor.is_some() {
                                                app.clear_selection();
                                                app.status = "Selection cleared".to_string();
                                            } else {
                                                app.filter_clear();
                                            }
                                        }
                                        _ => {}
                                    }
                                }
                            }
                        }
                    }
                }
                Event::Mouse(mouse) => {
                    // Handle click on status bar (row 0) - limit indicator
                    if mouse.row == 0 && mouse.kind == MouseEventKind::Down(MouseButton::Left) {
                        // Calculate approximate position of limit indicator
                        // Mode: ~8 chars, Connection: varies, Limit starts after
                        let mode_len = 8u16;
                        let conn_len = app
                            .connection
                            .as_ref()
                            .map(|c| c.profile.len() + c.target.len() + 4)
                            .unwrap_or(16) as u16;
                        let limit_start = mode_len + conn_len;
                        let limit_end = limit_start + 8; // " [10k] " is about 8 chars

                        if mouse.column >= limit_start && mouse.column < limit_end {
                            app.modal = Modal::LimitPicker;
                        }
                    }

                    // Handle mouse events for explorer
                    if app.explorer_visible {
                        // Explorer is in left 35 columns (with 1px border)
                        let explorer_width = 35u16;
                        let explorer_start_y = 1u16; // After status bar
                        let explorer_inner_x = 1u16; // After left border
                        let explorer_inner_y = explorer_start_y + 1; // After top border

                        if mouse.column < explorer_width && mouse.row >= explorer_inner_y {
                            match mouse.kind {
                                MouseEventKind::Down(MouseButton::Left) => {
                                    // Calculate which node was clicked
                                    let clicked_row = (mouse.row - explorer_inner_y) as usize;
                                    let target_idx = app.explorer_scroll + clicked_row;
                                    if target_idx < app.explorer_nodes.len() {
                                        app.explorer_selected = target_idx;
                                        app.focus = Pane::Explorer;
                                    }
                                }
                                MouseEventKind::Up(MouseButton::Left) => {
                                    // Double-click logic could go here
                                    // For now, toggle on click if on expand icon
                                    let clicked_row = (mouse.row - explorer_inner_y) as usize;
                                    let target_idx = app.explorer_scroll + clicked_row;
                                    if target_idx == app.explorer_selected {
                                        if let Some(node) = app.explorer_nodes.get(target_idx) {
                                            // Check if click was on the icon area
                                            let icon_col =
                                                (node.depth * 2) as u16 + explorer_inner_x;
                                            if mouse.column >= icon_col
                                                && mouse.column < icon_col + 2
                                                && node.children_count > 0
                                            {
                                                app.explorer_toggle_node();
                                            }
                                        }
                                    }
                                }
                                MouseEventKind::ScrollUp => {
                                    if app.focus == Pane::Explorer {
                                        app.explorer_prev();
                                        app.explorer_prev();
                                        app.explorer_prev();
                                    }
                                }
                                MouseEventKind::ScrollDown
                                    if app.focus == Pane::Explorer => {
                                        app.explorer_next();
                                        app.explorer_next();
                                        app.explorer_next();
                                    }
                                _ => {}
                            }
                        }
                    }

                    // Handle mouse events for results pane (using stored area from render)
                    let (res_x, res_y, res_w, res_h) = app.results_area;
                    let res_end_x = res_x + res_w;
                    let res_end_y = res_y + res_h;

                    // Handle cell detail panel resize (if panel is open)
                    if let Modal::CellDetail(_, _) = app.modal {
                        // Calculate panel left edge position
                        let panel_width =
                            ((res_w as u32 * app.cell_detail_width as u32) / 100) as u16;
                        let panel_width = panel_width.max(20).min(res_w.saturating_sub(10));
                        let panel_left_x = res_x + res_w.saturating_sub(panel_width);

                        // Check for click on panel left border (within 2 pixels for easier grabbing)
                        if let MouseEventKind::Down(MouseButton::Left) = mouse.kind {
                            if mouse.column >= panel_left_x.saturating_sub(1)
                                && mouse.column <= panel_left_x + 1
                                && mouse.row >= res_y
                                && mouse.row < res_end_y
                            {
                                app.drag_mode = DragMode::CellDetailResize;
                            }
                        }

                        // Handle drag for cell detail resize
                        if let MouseEventKind::Drag(MouseButton::Left) = mouse.kind {
                            if app.drag_mode == DragMode::CellDetailResize {
                                // Calculate new width percentage based on mouse position
                                let mouse_offset = res_end_x.saturating_sub(mouse.column);
                                let new_percent =
                                    ((mouse_offset as u32 * 100) / res_w as u32) as u16;
                                app.cell_detail_width = new_percent.clamp(15, 80);
                            }
                        }
                    }

                    if mouse.column >= res_x
                        && mouse.column < res_end_x
                        && mouse.row >= res_y
                        && mouse.row < res_end_y
                    {
                        match mouse.kind {
                            MouseEventKind::Down(MouseButton::Left) => {
                                app.focus = Pane::Results;
                                let inner_x = res_x + 1; // After left border
                                let inner_y = res_y + 1; // After top border
                                let header_y = inner_y; // Header is first row inside border
                                let scrollbar_x = res_end_x - 2; // Vertical scrollbar column
                                let hscrollbar_y = res_end_y - 2; // Horizontal scrollbar row

                                // Check if click is on horizontal scrollbar
                                if mouse.row == hscrollbar_y && app.max_hscroll > 0 {
                                    app.drag_mode = DragMode::HScrollbar;
                                    let scrollbar_width = res_w.saturating_sub(3) as usize; // -2 borders, -1 corner
                                    let click_offset =
                                        mouse.column.saturating_sub(inner_x) as usize;
                                    let total_width: usize =
                                        app.col_widths.iter().map(|w| w + 1).sum();
                                    if let Some(target) =
                                        (click_offset * total_width).checked_div(scrollbar_width)
                                    {
                                        app.results_hscroll = target.min(app.max_hscroll);
                                    }
                                }
                                // Check if click is on vertical scrollbar area
                                else if mouse.column >= scrollbar_x
                                    && app.filtered_row_count() > app.visible_rows
                                {
                                    app.drag_mode = DragMode::VScrollbar;
                                    // Calculate scroll position from click
                                    let scrollbar_height = res_h.saturating_sub(4) as usize; // -2 borders, -1 header, -1 hscroll
                                    let click_offset =
                                        mouse.row.saturating_sub(header_y + 1) as usize;
                                    let total_rows = app.filtered_row_count();
                                    if let Some(target) =
                                        (click_offset * total_rows).checked_div(scrollbar_height)
                                    {
                                        app.results_scroll =
                                            target.min(total_rows.saturating_sub(1));
                                    }
                                }
                                // Check if click is on header row (for sorting or column resize)
                                else if mouse.row == header_y && !app.col_widths.is_empty() {
                                    // Calculate click position relative to content
                                    let click_x = (mouse.column.saturating_sub(inner_x) as usize)
                                        + app.results_hscroll;

                                    // First check if near a column border (for resize)
                                    let mut col_end_pos = app.row_num_width + 1; // Start after row number column
                                    let mut resize_col: Option<usize> = None;
                                    for (col_idx, &width) in app.col_widths.iter().enumerate() {
                                        col_end_pos += width + 1; // +1 for separator
                                                                  // Check if click is within 2 chars of column border
                                        if click_x >= col_end_pos.saturating_sub(2)
                                            && click_x <= col_end_pos + 1
                                        {
                                            resize_col = Some(col_idx);
                                            break;
                                        }
                                    }

                                    if let Some(vis_col_idx) = resize_col {
                                        // Start column resize - map to original column index
                                        let orig_col_idx = app
                                            .visible_cols
                                            .get(vis_col_idx)
                                            .copied()
                                            .unwrap_or(vis_col_idx);
                                        app.drag_mode = DragMode::ColumnResize(orig_col_idx);
                                    } else {
                                        // Regular click - sort by column
                                        let mut col_start = 0usize;
                                        for (vis_col_idx, &width) in
                                            app.col_widths.iter().enumerate()
                                        {
                                            let col_end = col_start + width + 1; // +1 for space
                                            if click_x >= col_start && click_x < col_end {
                                                // Map visible column index to original column index
                                                let orig_col_idx = app
                                                    .visible_cols
                                                    .get(vis_col_idx)
                                                    .copied()
                                                    .unwrap_or(vis_col_idx);
                                                app.sort_by_column(orig_col_idx);
                                                break;
                                            }
                                            col_start = col_end;
                                        }
                                    }
                                } else if mouse.row > header_y {
                                    // Click on data row - select it and detect column for cell detail
                                    let clicked_row = (mouse.row - header_y - 1) as usize;
                                    let row_offset = if app.results_scroll >= app.visible_rows {
                                        app.results_scroll - app.visible_rows + 1
                                    } else {
                                        0
                                    };
                                    let target_idx = row_offset + clicked_row;
                                    if target_idx < app.filtered_row_count() {
                                        app.results_scroll = target_idx;

                                        // Calculate which column was clicked
                                        let click_x = (mouse.column.saturating_sub(inner_x)
                                            as usize)
                                            + app.results_hscroll;
                                        // Skip row number column + separator
                                        if click_x > app.row_num_width {
                                            let content_x = click_x - app.row_num_width - 1; // -1 for separator
                                            let mut col_start = 0usize;
                                            for (vis_col_idx, &width) in
                                                app.col_widths.iter().enumerate()
                                            {
                                                let col_end = col_start + width + 2; // +2 for space + separator
                                                if content_x >= col_start && content_x < col_end {
                                                    // Map visible column index to original column index
                                                    let orig_col_idx = app
                                                        .visible_cols
                                                        .get(vis_col_idx)
                                                        .copied()
                                                        .unwrap_or(vis_col_idx);

                                                    // Check for double-click (same cell within 500ms)
                                                    let now = std::time::Instant::now();
                                                    let is_double_click = now
                                                        .duration_since(app.last_click_time)
                                                        .as_millis()
                                                        < 500
                                                        && app.last_click_pos
                                                            == (mouse.row, mouse.column);

                                                    if is_double_click {
                                                        // Double-click: copy cell value to clipboard
                                                        let display_rows = app.get_display_rows();
                                                        if let Some(row) =
                                                            display_rows.get(target_idx)
                                                        {
                                                            if let Some(cell) =
                                                                row.get(orig_col_idx)
                                                            {
                                                                if let Ok(mut ctx) =
                                                                    arboard::Clipboard::new()
                                                                {
                                                                    let _ =
                                                                        ctx.set_text(cell.clone());
                                                                    app.status =
                                                                        "Copied to clipboard"
                                                                            .to_string();
                                                                }
                                                            }
                                                        }
                                                    } else {
                                                        // Single click: open cell detail panel
                                                        app.modal = Modal::CellDetail(
                                                            target_idx,
                                                            orig_col_idx,
                                                        );
                                                    }

                                                    app.last_click_time = now;
                                                    app.last_click_pos = (mouse.row, mouse.column);
                                                    break;
                                                }
                                                col_start = col_end;
                                            }
                                        }
                                    }
                                }
                            }
                            MouseEventKind::Drag(MouseButton::Left) => {
                                let inner_x = res_x + 1;
                                let inner_y = res_y + 1;
                                let header_y = inner_y;

                                // Only process scrollbar drags if we started on a scrollbar
                                if app.drag_mode == DragMode::HScrollbar && app.max_hscroll > 0 {
                                    let scrollbar_width = res_w.saturating_sub(3) as usize;
                                    let click_offset =
                                        mouse.column.saturating_sub(inner_x) as usize;
                                    let total_width: usize =
                                        app.col_widths.iter().map(|w| w + 1).sum();
                                    if let Some(target) =
                                        (click_offset * total_width).checked_div(scrollbar_width)
                                    {
                                        app.results_hscroll = target.min(app.max_hscroll);
                                    }
                                } else if app.drag_mode == DragMode::VScrollbar
                                    && app.filtered_row_count() > app.visible_rows
                                {
                                    let scrollbar_height = res_h.saturating_sub(4) as usize;
                                    let click_offset =
                                        mouse.row.saturating_sub(header_y + 1) as usize;
                                    let total_rows = app.filtered_row_count();
                                    if let Some(target) =
                                        (click_offset * total_rows).checked_div(scrollbar_height)
                                    {
                                        app.results_scroll =
                                            target.min(total_rows.saturating_sub(1));
                                    }
                                } else if let DragMode::ColumnResize(col_idx) = app.drag_mode {
                                    // Calculate new column width based on mouse position
                                    let click_x = (mouse.column.saturating_sub(inner_x) as usize)
                                        + app.results_hscroll;

                                    // Find where this column starts
                                    let mut col_start = app.row_num_width + 1;
                                    for i in 0..col_idx {
                                        col_start +=
                                            app.col_widths.get(i).copied().unwrap_or(10) + 1;
                                    }

                                    // New width is distance from column start to mouse
                                    let new_width = click_x.saturating_sub(col_start).max(4); // Min width 4
                                    app.custom_col_widths.insert(col_idx, new_width);
                                }
                            }
                            MouseEventKind::Up(MouseButton::Left) => {
                                // Reset drag mode when mouse is released
                                app.drag_mode = DragMode::None;
                            }
                            MouseEventKind::ScrollUp => {
                                app.results_prev();
                                app.results_prev();
                                app.results_prev();
                            }
                            MouseEventKind::ScrollDown => {
                                app.results_next();
                                app.results_next();
                                app.results_next();
                            }
                            MouseEventKind::ScrollLeft => {
                                app.results_scroll_left();
                            }
                            MouseEventKind::ScrollRight => {
                                app.results_scroll_right();
                            }
                            MouseEventKind::Down(MouseButton::Right) => {
                                // Right-click on header for column stats
                                let inner_x = res_x + 1;
                                let inner_y = res_y + 1;
                                let header_y = inner_y;

                                if mouse.row == header_y && !app.col_widths.is_empty() {
                                    let click_x = (mouse.column.saturating_sub(inner_x) as usize)
                                        + app.results_hscroll;
                                    let mut col_start = 0usize;
                                    for (vis_col_idx, &width) in app.col_widths.iter().enumerate() {
                                        let col_end = col_start + width + 1;
                                        if click_x >= col_start && click_x < col_end {
                                            // Map visible column index to original column index
                                            let orig_col_idx = app
                                                .visible_cols
                                                .get(vis_col_idx)
                                                .copied()
                                                .unwrap_or(vis_col_idx);
                                            app.modal = Modal::ColumnStats(orig_col_idx);
                                            break;
                                        }
                                        col_start = col_end;
                                    }
                                }
                            }
                            _ => {}
                        }
                    }

                    // Handle mouse events for editor pane (click to focus)
                    let (ed_x, ed_y, ed_w, ed_h) = app.editor_area;
                    let ed_end_x = ed_x + ed_w;
                    let ed_end_y = ed_y + ed_h;

                    if mouse.column >= ed_x
                        && mouse.column < ed_end_x
                        && mouse.row >= ed_y
                        && mouse.row < ed_end_y
                    {
                        if let MouseEventKind::Down(MouseButton::Left) = mouse.kind {
                            app.focus = Pane::Editor;
                        }
                    }

                    // Handle split border clicking and dragging for resize
                    if app.results_visible {
                        let (res_x, res_y, res_w, res_h) = app.results_area;
                        let (ed_x, ed_y, ed_w, ed_h) = app.editor_area;

                        // Detect click on split border
                        if let MouseEventKind::Down(MouseButton::Left) = mouse.kind {
                            let on_border = if app.split_horizontal {
                                // Horizontal split: border is between editor bottom and results top
                                let border_y = ed_y + ed_h;
                                mouse.row == border_y || mouse.row == res_y.saturating_sub(1)
                            } else {
                                // Vertical split: border is between editor right and results left
                                let border_x = ed_x + ed_w;
                                mouse.column == border_x || mouse.column == res_x.saturating_sub(1)
                            };
                            if on_border {
                                app.drag_mode = DragMode::ResizeSplit;
                            }
                        }

                        // Only handle resize drag if we started on the split border
                        if let MouseEventKind::Drag(MouseButton::Left) = mouse.kind {
                            if app.drag_mode == DragMode::ResizeSplit {
                                let old_percent = app.split_percent;
                                if app.split_horizontal {
                                    // Horizontal split: calculate based on vertical position
                                    let content_start = ed_y;
                                    let content_end = res_y + res_h;
                                    let total_height = content_end.saturating_sub(content_start);
                                    if total_height > 0 {
                                        let mouse_offset = mouse.row.saturating_sub(content_start);
                                        let new_percent =
                                            ((mouse_offset as f32 / total_height as f32) * 100.0)
                                                as u16;
                                        app.split_percent = new_percent.clamp(20, 80);
                                    }
                                } else {
                                    // Vertical split: calculate based on horizontal position
                                    let content_start = ed_x;
                                    let content_end = res_x + res_w;
                                    let total_width = content_end.saturating_sub(content_start);
                                    if total_width > 0 {
                                        let mouse_offset =
                                            mouse.column.saturating_sub(content_start);
                                        let new_percent =
                                            ((mouse_offset as f32 / total_width as f32) * 100.0)
                                                as u16;
                                        app.split_percent = new_percent.clamp(20, 80);
                                    }
                                }
                                // Resize nvim if split percent changed
                                if app.split_percent != old_percent {
                                    let size = terminal.size()?;
                                    let content_height = size.height.saturating_sub(2);
                                    let editor_height =
                                        ((content_height as u32 * app.split_percent as u32) / 100)
                                            as u16;
                                    let new_height =
                                        editor_height.saturating_sub(2).max(5) as usize;
                                    let new_width = size.width.saturating_sub(2) as usize;
                                    nvim.resize(new_width, new_height).await?;
                                }
                            }
                        }

                        // Reset drag mode on mouse up
                        if let MouseEventKind::Up(MouseButton::Left) = mouse.kind {
                            app.drag_mode = DragMode::None;
                        }
                    }
                }
                Event::Resize(width, height) => {
                    // Recalculate nvim size based on new terminal size and layout
                    // Account for explorer panel if visible
                    let content_width = if app.explorer_visible {
                        width.saturating_sub(35) // Explorer takes 35 cols
                    } else {
                        width
                    };
                    let content_height = height.saturating_sub(2); // -1 status, -1 help

                    let (editor_width, editor_height) = if app.results_visible {
                        if app.split_horizontal {
                            // Horizontal split: editor gets split_percent of height
                            let h =
                                ((content_height as u32 * app.split_percent as u32) / 100) as u16;
                            (content_width, h)
                        } else {
                            // Vertical split: editor gets split_percent of width
                            let w =
                                ((content_width as u32 * app.split_percent as u32) / 100) as u16;
                            (w, content_height)
                        }
                    } else {
                        // Full view: editor gets all content area
                        (content_width, content_height)
                    };

                    let new_height = editor_height.saturating_sub(2).max(5) as usize;
                    let new_width = editor_width.saturating_sub(2).max(10) as usize;
                    nvim.resize(new_width, new_height).await?;
                }
                Event::Paste(content)
                    // Handle bracketed paste - use nvim's paste API
                    if app.focus == Pane::Editor && app.modal == Modal::None => {
                        // Use nvim's paste API which handles modes correctly
                        nvim.paste(&content).await?;
                    }
                _ => {}
            }
        }

        // Run a query queued from Ctrl+e or the history picker
        if let Some(sql) = pending_run_query.take() {
            if let Some(conn) = app.connection.clone() {
                app.loading = true;
                app.loading_frame = 0;
                app.error = None;
                let was_visible = app.results_visible;
                app.results_visible = true; // Show results pane with loading
                app.status = "Running query...".to_string();

                // Resize nvim if results pane just became visible
                if !was_visible {
                    let size = terminal.size()?;
                    let content_height = size.height.saturating_sub(2);
                    let editor_height =
                        ((content_height as u32 * app.split_percent as u32) / 100) as u16;
                    let new_height = editor_height.saturating_sub(2).max(5) as usize;
                    let new_width = size.width.saturating_sub(2) as usize;
                    nvim.resize(new_width, new_height).await?;
                }

                // Spawn query execution in background
                let limit = app.query_limit;
                let conn_clone = conn.clone();
                let sql_clone = sql.clone();
                let (tx, mut rx) = tokio::sync::oneshot::channel();

                tokio::spawn(async move {
                    let start = std::time::Instant::now();
                    let result = executor::execute_query(&conn_clone, &sql_clone, limit).await;
                    let elapsed = start.elapsed();
                    let _ = tx.send((result, elapsed));
                });

                // Animate while waiting for query, but still handle input
                let mut interval = tokio::time::interval(std::time::Duration::from_millis(100));
                loop {
                    tokio::select! {
                        result = &mut rx => {
                            match result {
                                Ok((Ok(query_result), elapsed)) => {
                                    app.add_to_history(&sql);
                                    app.has_run_query = true;
                                    app.results_visible = true;
                                    let row_count = query_result.rows.len();
                                    app.set_results(query_result.columns, query_result.rows);
                                    let time_str = if elapsed.as_secs() > 0 {
                                        format!("{:.2}s", elapsed.as_secs_f64())
                                    } else {
                                        format!("{}ms", elapsed.as_millis())
                                    };
                                    let hit_limit = match limit {
                                        QueryLimit::Limit(n) => row_count >= n,
                                        QueryLimit::NoLimit => false,
                                    };
                                    app.status = if hit_limit {
                                        format!("{} rows (limit: {}) in {}", row_count, limit.short_display(), time_str)
                                    } else {
                                        format!("{} rows in {}", row_count, time_str)
                                    };
                                    app.focus = Pane::Results;
                                }
                                Ok((Err(e), _)) => {
                                    app.has_run_query = true;
                                    app.results_visible = true;
                                    app.error = Some(e.to_string());
                                    app.status = "Query failed".to_string();
                                    app.focus = Pane::Results;
                                }
                                Err(_) => {
                                    app.error = Some("Query task failed".to_string());
                                    app.status = "Query failed".to_string();
                                }
                            }
                            app.loading = false;
                            break;
                        }
                        _ = interval.tick() => {
                            app.loading_frame = (app.loading_frame + 1) % 10;
                            terminal.draw(|frame| ui::render(frame, &mut app))?;
                        }
                        // Handle input while query is running
                        _ = async {
                            if event::poll(std::time::Duration::from_millis(0)).unwrap_or(false) {
                                match event::read() {
                                    Ok(Event::Key(key)) if key.kind == KeyEventKind::Press => {
                                        // Send input to nvim (allow editing while query runs)
                                        let input = key_to_nvim_input(&key);
                                        let _ = nvim.send_input(&input).await;
                                    }
                                    Ok(Event::Mouse(mouse)) => {
                                        if let MouseEventKind::Down(MouseButton::Left) = mouse.kind {
                                            // Check for cancel button click
                                            if let Some((bx, by, bw, bh)) = app.cancel_button_area {
                                                if mouse.column >= bx && mouse.column < bx + bw
                                                    && mouse.row >= by && mouse.row < by + bh
                                                {
                                                    app.modal = Modal::CancelConfirm;
                                                }
                                            }
                                            // Check for Yes/No button clicks in CancelConfirm modal
                                            if app.modal == Modal::CancelConfirm {
                                                if let Some((bx, by, bw, bh)) = app.confirm_yes_area {
                                                    if mouse.column >= bx && mouse.column < bx + bw
                                                        && mouse.row >= by && mouse.row < by + bh
                                                    {
                                                        app.cancel_requested = true;
                                                        app.modal = Modal::None;
                                                    }
                                                }
                                                if let Some((bx, by, bw, bh)) = app.confirm_no_area {
                                                    if mouse.column >= bx && mouse.column < bx + bw
                                                        && mouse.row >= by && mouse.row < by + bh
                                                    {
                                                        app.modal = Modal::None;
                                                    }
                                                }
                                            }
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        } => {
                            // Check if cancellation was requested
                            if app.cancel_requested {
                                app.loading = false;
                                app.cancel_requested = false;
                                app.status = "Query cancelled".to_string();
                                break;
                            }
                        }
                    }
                }
            } else {
                app.status = "No connection selected".to_string();
            }
        }

        // Handle pending command from palette
        if let Some(cmd) = pending_command.take() {
            match cmd {
                Command::ToggleResults => {
                    app.toggle_results();
                    let size = terminal.size()?;
                    let content_height = size.height.saturating_sub(2);
                    let editor_height = if app.results_visible {
                        ((content_height as u32 * app.split_percent as u32) / 100) as u16
                    } else {
                        content_height
                    };
                    let new_height = editor_height.saturating_sub(2).max(5) as usize;
                    let new_width = size.width.saturating_sub(2) as usize;
                    nvim.resize(new_width, new_height).await?;
                }
                Command::ToggleExplorer => {
                    if app.explorer_visible {
                        app.explorer_visible = false;
                        if app.focus == Pane::Explorer {
                            app.focus = Pane::Editor;
                        }
                    } else if app.connection.is_some() {
                        app.explorer_visible = true;
                        app.focus = Pane::Explorer;
                    } else {
                        app.status = "No connection selected".to_string();
                    }
                }
                Command::ToggleSplitDirection => {
                    app.toggle_split_direction();
                }
                Command::FormatSql => {
                    let sql = nvim.get_buffer_contents().await?;
                    let formatted = format_sql(&sql);
                    nvim.set_buffer_contents(&formatted).await?;
                    app.status = "SQL formatted".to_string();
                }
                Command::OpenConnectionPicker => {
                    app.open_connection_picker();
                }
                Command::OpenLimitPicker => {
                    app.modal = Modal::LimitPicker;
                }
                Command::SaveQuery => {
                    app.modal = Modal::SaveQuery(String::new());
                }
                Command::LoadQuery => {
                    app.saved_queries = config::list_saved_queries().unwrap_or_default();
                    app.load_query_index = 0;
                    app.modal = Modal::LoadQuery;
                }
                Command::ExportCsv => {
                    if app.columns.is_empty() {
                        app.status = "No results to export".to_string();
                    } else {
                        app.export_columns = vec![true; app.columns.len()];
                        app.export_picker_index = 0;
                        app.modal = Modal::ExportColumns;
                    }
                }
                Command::ShowHistory => {
                    app.open_history_picker();
                }
                Command::ShowHelp => {
                    app.modal = Modal::Help;
                }
            }
        }

        if app.should_quit {
            break;
        }
    }

    // Auto-save editor content for next session
    if let Ok(content) = nvim.get_buffer_contents().await {
        let _ = config::save_autosave(&content);
    }

    // Clean shutdown of nvim
    let _ = nvim.quit().await;

    // Cleanup terminal
    disable_raw_mode()?;
    stdout().execute(DisableBracketedPaste)?;
    stdout().execute(DisableMouseCapture)?;
    stdout().execute(LeaveAlternateScreen)?;

    Ok(())
}
