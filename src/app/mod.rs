//! Application state and logic

mod state;

pub use state::*;

use crate::config;
use crate::executor;
use crate::nvim::Screen;
use crate::profiles::Connection;
use std::sync::{Arc, Mutex};

pub struct App {
    /// Currently focused pane
    pub focus: Pane,
    /// Active database connection
    pub connection: Option<Connection>,
    /// Query results from last execution
    pub results: Vec<Vec<String>>,
    /// Column headers for results
    pub columns: Vec<String>,
    /// Should the app exit
    pub should_quit: bool,
    /// Status message
    pub status: String,
    /// Shared screen state from nvim
    pub screen: Arc<Mutex<Screen>>,
    /// Is a query currently running?
    pub loading: bool,
    /// Has a query been executed this session?
    pub has_run_query: bool,
    /// Results pane visible
    pub results_visible: bool,
    /// Error message from last query
    pub error: Option<String>,
    /// Current modal dialog
    pub modal: Modal,
    /// Available connections from profiles.yml
    pub connections: Vec<Connection>,
    /// Selected index in connection picker
    pub picker_index: usize,
    /// Filter string for connection picker
    pub picker_filter: String,
    /// Filtered connection indices
    pub picker_filtered: Vec<usize>,
    /// Connection pending adapter setup (set by picker, processed by main loop)
    pub pending_connection: Option<Connection>,
    /// Scroll offset for results table
    pub results_scroll: usize,
    /// Horizontal scroll offset for results table
    pub results_hscroll: usize,
    /// Pending 'g' key for gg command
    pub pending_g: bool,
    /// Selection anchor for multi-row select (None = no selection)
    pub selection_anchor: Option<usize>,
    /// Copy delimiter preference
    pub copy_delimiter: CopyDelimiter,
    /// Include header in copy
    pub copy_include_header: bool,
    /// Multi-column sort specifications (in priority order)
    pub sort_specs: Vec<SortSpec>,
    /// Filter string for results
    pub filter: String,
    /// Selected index in sort picker
    pub sort_picker_index: usize,
    /// Filtered row indices (indexes into results)
    pub filtered_indices: Vec<usize>,
    /// Visible rows in results pane (set by UI)
    pub visible_rows: usize,
    /// Max horizontal scroll (set by UI based on content width)
    pub max_hscroll: usize,
    /// Column widths for visible columns in results (set by UI, used for click-to-sort)
    pub col_widths: Vec<usize>,
    /// Mapping from visible column index to original column index (for hidden column support)
    pub visible_cols: Vec<usize>,
    /// Row number column width (set by UI)
    pub row_num_width: usize,
    /// Results pane area (set by UI, used for mouse handling)
    pub results_area: (u16, u16, u16, u16), // (x, y, width, height)
    /// Editor pane area (set by UI, used for mouse handling)
    pub editor_area: (u16, u16, u16, u16),
    /// Split percentage (0-100, how much space editor gets)
    pub split_percent: u16,
    /// Horizontal split (true = editor top/results bottom, false = editor left/results right)
    pub split_horizontal: bool,
    /// Explorer pane visible
    pub explorer_visible: bool,
    /// All explorer nodes (flat list for rendering)
    pub explorer_nodes: Vec<ExplorerNode>,
    /// Currently selected explorer index
    pub explorer_selected: usize,
    /// Explorer scroll offset
    pub explorer_scroll: usize,
    /// Explorer filter string
    pub explorer_filter: String,
    /// Cached schema data
    pub schema_cache: Vec<executor::SchemaInfo>,
    /// Query history
    pub history: config::QueryHistory,
    /// App state (last connection, etc.)
    pub app_state: config::AppState,
    /// List of saved query files (for load modal)
    pub saved_queries: Vec<String>,
    /// Selected index in load query picker
    pub load_query_index: usize,
    /// Which columns to include in export (true = include)
    pub export_columns: Vec<bool>,
    /// Selected index in export column picker
    pub export_picker_index: usize,
    /// Query row limit
    pub query_limit: QueryLimit,
    /// Selected index in limit picker
    pub limit_picker_index: usize,
    /// Loading animation frame
    pub loading_frame: usize,
    /// Current drag mode (what the user is dragging)
    pub drag_mode: DragMode,
    /// Schema is currently being loaded in background
    pub schema_loading: bool,
    /// Cell detail panel width (percentage of results area, 10-80)
    pub cell_detail_width: u16,
    /// Flag to request query cancellation
    pub cancel_requested: bool,
    /// Cancel button area for mouse click detection (x, y, width, height)
    pub cancel_button_area: Option<(u16, u16, u16, u16)>,
    /// Confirm Yes button area
    pub confirm_yes_area: Option<(u16, u16, u16, u16)>,
    /// Confirm No button area
    pub confirm_no_area: Option<(u16, u16, u16, u16)>,
    /// Last click time for double-click detection
    pub last_click_time: std::time::Instant,
    /// Last click position (row, col) for double-click detection
    pub last_click_pos: (u16, u16),
    /// Custom column widths (overrides auto-calculated widths)
    pub custom_col_widths: std::collections::HashMap<usize, usize>,
    /// Hidden columns (indices)
    pub hidden_columns: std::collections::HashSet<usize>,
    /// Index for hidden columns picker
    pub hidden_columns_index: usize,
    /// Command palette filter string
    pub palette_filter: String,
    /// Command palette selected index
    pub palette_index: usize,
    /// Filtered command indices
    pub palette_filtered: Vec<usize>,
    /// History picker filter string
    pub history_picker_filter: String,
    /// History picker selected index (into history_picker_filtered)
    pub history_picker_index: usize,
    /// Filtered history entry indices (into history_entries(), newest first)
    pub history_picker_filtered: Vec<usize>,
    /// Visualization mode - show chart instead of table
    pub viz_mode: bool,
    /// Visualization configuration
    pub viz_config: VizConfig,
    /// Computed aggregated data for viz (label, value pairs)
    pub viz_data: Vec<(String, f64)>,
}

impl App {
    pub fn new(screen: Arc<Mutex<Screen>>, connections: Vec<Connection>) -> Self {
        // Load persisted state
        let history = config::QueryHistory::load().unwrap_or_default();
        let app_state = config::AppState::load().unwrap_or_default();

        Self {
            focus: Pane::Editor,
            connection: None,
            results: Vec::new(),
            columns: Vec::new(),
            should_quit: false,
            status: String::from("No connection"),
            screen,
            loading: false,
            has_run_query: false,
            results_visible: false,
            error: None,
            modal: Modal::None,
            picker_filtered: (0..connections.len()).collect(),
            connections,
            picker_index: 0,
            picker_filter: String::new(),
            pending_connection: None,
            results_scroll: 0,
            results_hscroll: 0,
            pending_g: false,
            selection_anchor: None,
            copy_delimiter: CopyDelimiter::Comma,
            copy_include_header: true,
            sort_specs: Vec::new(),
            filter: String::new(),
            sort_picker_index: 0,
            filtered_indices: Vec::new(),
            visible_rows: 10,
            max_hscroll: 0,
            col_widths: Vec::new(),
            visible_cols: Vec::new(),
            row_num_width: 2,
            results_area: (0, 0, 0, 0),
            editor_area: (0, 0, 0, 0),
            split_percent: 50,
            split_horizontal: true,
            explorer_visible: false,
            explorer_nodes: Vec::new(),
            explorer_selected: 0,
            explorer_scroll: 0,
            explorer_filter: String::new(),
            schema_cache: Vec::new(),
            history,
            app_state,
            saved_queries: Vec::new(),
            load_query_index: 0,
            export_columns: Vec::new(),
            export_picker_index: 0,
            query_limit: QueryLimit::Limit(10000),
            limit_picker_index: 2, // Default to 10000 (index 2)
            loading_frame: 0,
            drag_mode: DragMode::None,
            schema_loading: false,
            cell_detail_width: 35, // Default 35% of results area
            cancel_requested: false,
            cancel_button_area: None,
            confirm_yes_area: None,
            confirm_no_area: None,
            last_click_time: std::time::Instant::now(),
            last_click_pos: (0, 0),
            custom_col_widths: std::collections::HashMap::new(),
            hidden_columns: std::collections::HashSet::new(),
            hidden_columns_index: 0,
            palette_filter: String::new(),
            palette_index: 0,
            palette_filtered: (0..Command::all().len()).collect(),
            history_picker_filter: String::new(),
            history_picker_index: 0,
            history_picker_filtered: Vec::new(),
            viz_mode: false,
            viz_config: VizConfig::default(),
            viz_data: Vec::new(),
        }
    }

    /// Get the connection key for history (profile:target)
    pub fn connection_key(&self) -> Option<String> {
        self.connection
            .as_ref()
            .map(|c| format!("{}:{}", c.profile, c.target))
    }

    /// Add current query to history
    pub fn add_to_history(&mut self, query: &str) {
        if let Some(key) = self.connection_key() {
            self.history.add(&key, query);
            let _ = self.history.save(); // Ignore save errors
        }
    }

    pub fn results_next(&mut self) {
        let max = self.filtered_row_count().saturating_sub(1);
        if self.results_scroll < max {
            self.results_scroll += 1;
        }
    }

    pub fn results_prev(&mut self) {
        self.results_scroll = self.results_scroll.saturating_sub(1);
    }

    pub fn results_page_down(&mut self) {
        let half_page = self.visible_rows / 2;
        let max = self.filtered_row_count().saturating_sub(1);
        self.results_scroll = (self.results_scroll + half_page).min(max);
    }

    pub fn results_page_up(&mut self) {
        let half_page = self.visible_rows / 2;
        self.results_scroll = self.results_scroll.saturating_sub(half_page);
    }

    /// Go to first row (gg)
    pub fn results_first(&mut self) {
        self.results_scroll = 0;
    }

    /// Go to last row (G)
    pub fn results_last(&mut self) {
        self.results_scroll = self.filtered_row_count().saturating_sub(1);
    }

    /// Go to top of visible area (H)
    #[allow(dead_code)]
    pub fn results_high(&mut self) {
        let offset = if self.results_scroll >= self.visible_rows {
            self.results_scroll - self.visible_rows + 1
        } else {
            0
        };
        self.results_scroll = offset;
    }

    /// Go to middle of visible area (M)
    pub fn results_middle(&mut self) {
        let offset = if self.results_scroll >= self.visible_rows {
            self.results_scroll - self.visible_rows + 1
        } else {
            0
        };
        let max = self.filtered_row_count().saturating_sub(1);
        self.results_scroll = (offset + self.visible_rows / 2).min(max);
    }

    /// Go to bottom of visible area (L)
    pub fn results_low(&mut self) {
        let offset = if self.results_scroll >= self.visible_rows {
            self.results_scroll - self.visible_rows + 1
        } else {
            0
        };
        let max = self.filtered_row_count().saturating_sub(1);
        self.results_scroll = (offset + self.visible_rows - 1).min(max);
    }

    /// Scroll horizontally to start (0)
    pub fn results_scroll_start(&mut self) {
        self.results_hscroll = 0;
    }

    /// Scroll horizontally to end ($)
    pub fn results_scroll_end(&mut self) {
        self.results_hscroll = self.max_hscroll;
    }

    pub fn results_scroll_left(&mut self) {
        self.results_hscroll = self.results_hscroll.saturating_sub(10);
    }

    pub fn results_scroll_right(&mut self) {
        self.results_hscroll = (self.results_hscroll + 10).min(self.max_hscroll);
    }

    // Sort picker methods
    pub fn open_sort_picker(&mut self) {
        if !self.columns.is_empty() {
            self.modal = Modal::SortPicker;
            self.sort_picker_index = 0;
        }
    }

    pub fn sort_picker_next(&mut self) {
        if !self.columns.is_empty() {
            self.sort_picker_index = (self.sort_picker_index + 1) % self.columns.len();
        }
    }

    pub fn sort_picker_prev(&mut self) {
        if !self.columns.is_empty() {
            self.sort_picker_index = self
                .sort_picker_index
                .checked_sub(1)
                .unwrap_or(self.columns.len() - 1);
        }
    }

    /// Toggle column in sort order (add if not present, remove if present)
    pub fn sort_picker_toggle(&mut self) {
        let col = self.sort_picker_index;
        if let Some(pos) = self.sort_specs.iter().position(|s| s.column == col) {
            // Already in sort order - remove it
            self.sort_specs.remove(pos);
        } else {
            // Add to sort order (ascending by default)
            self.sort_specs.push(SortSpec {
                column: col,
                ascending: true,
            });
        }
        self.apply_filter_and_sort();
    }

    /// Toggle direction of column at current index (if in sort order)
    #[allow(dead_code)]
    pub fn sort_picker_toggle_direction(&mut self) {
        let col = self.sort_picker_index;
        if let Some(spec) = self.sort_specs.iter_mut().find(|s| s.column == col) {
            spec.ascending = !spec.ascending;
            self.apply_filter_and_sort();
        }
    }

    /// Clear all sort specifications
    pub fn sort_clear(&mut self) {
        self.sort_specs.clear();
        self.apply_filter_and_sort();
    }

    /// Get sort priority for a column (1-indexed, None if not sorted)
    pub fn get_sort_priority(&self, col: usize) -> Option<(usize, bool)> {
        self.sort_specs
            .iter()
            .position(|s| s.column == col)
            .map(|pos| (pos + 1, self.sort_specs[pos].ascending))
    }

    #[allow(dead_code)]
    pub fn set_filter(&mut self, filter: String) {
        self.filter = filter;
        self.apply_filter_and_sort();
        self.results_scroll = 0;
    }

    pub fn filter_push(&mut self, c: char) {
        self.filter.push(c);
        self.apply_filter_and_sort();
        self.results_scroll = 0;
    }

    pub fn filter_pop(&mut self) {
        self.filter.pop();
        self.apply_filter_and_sort();
        self.results_scroll = 0;
    }

    pub fn filter_clear(&mut self) {
        self.filter.clear();
        self.apply_filter_and_sort();
        self.results_scroll = 0;
    }

    pub fn apply_filter_and_sort(&mut self) {
        // First filter
        let filter_lower = self.filter.to_lowercase();
        self.filtered_indices = if self.filter.is_empty() {
            (0..self.results.len()).collect()
        } else {
            self.results
                .iter()
                .enumerate()
                .filter(|(_, row)| {
                    row.iter()
                        .any(|cell| cell.to_lowercase().contains(&filter_lower))
                })
                .map(|(i, _)| i)
                .collect()
        };

        // Then sort by multiple columns
        if !self.sort_specs.is_empty() {
            let results = &self.results;
            let sort_specs = &self.sort_specs;
            self.filtered_indices.sort_by(|&a, &b| {
                for spec in sort_specs {
                    let va = results
                        .get(a)
                        .and_then(|r| r.get(spec.column))
                        .map(|s| s.as_str())
                        .unwrap_or("");
                    let vb = results
                        .get(b)
                        .and_then(|r| r.get(spec.column))
                        .map(|s| s.as_str())
                        .unwrap_or("");
                    // Try numeric comparison first
                    let cmp = match (va.parse::<f64>(), vb.parse::<f64>()) {
                        (Ok(na), Ok(nb)) => {
                            na.partial_cmp(&nb).unwrap_or(std::cmp::Ordering::Equal)
                        }
                        _ => va.cmp(vb),
                    };
                    let cmp = if spec.ascending { cmp } else { cmp.reverse() };
                    if cmp != std::cmp::Ordering::Equal {
                        return cmp;
                    }
                }
                std::cmp::Ordering::Equal
            });
        }
    }

    pub fn filtered_row_count(&self) -> usize {
        if self.filter.is_empty() && self.sort_specs.is_empty() {
            self.results.len()
        } else {
            self.filtered_indices.len()
        }
    }

    pub fn get_display_rows(&self) -> Vec<&Vec<String>> {
        if self.filter.is_empty() && self.sort_specs.is_empty() {
            self.results.iter().collect()
        } else {
            self.filtered_indices
                .iter()
                .filter_map(|&i| self.results.get(i))
                .collect()
        }
    }

    /// Get the currently selected row
    #[allow(dead_code)]
    pub fn get_current_row(&self) -> Option<Vec<String>> {
        let display_rows = self.get_display_rows();
        display_rows.get(self.results_scroll).map(|r| (*r).clone())
    }

    /// Get the selected row range (start, end) inclusive
    pub fn get_selection_range(&self) -> (usize, usize) {
        match self.selection_anchor {
            Some(anchor) => {
                let start = anchor.min(self.results_scroll);
                let end = anchor.max(self.results_scroll);
                (start, end)
            }
            None => (self.results_scroll, self.results_scroll),
        }
    }

    /// Toggle visual selection mode
    pub fn toggle_selection(&mut self) {
        if self.selection_anchor.is_some() {
            self.selection_anchor = None;
        } else {
            self.selection_anchor = Some(self.results_scroll);
        }
    }

    /// Clear selection
    pub fn clear_selection(&mut self) {
        self.selection_anchor = None;
    }

    /// Get selected rows for copying
    pub fn get_selected_rows(&self) -> Vec<Vec<String>> {
        let display_rows = self.get_display_rows();
        let (start, end) = self.get_selection_range();
        display_rows[start..=end.min(display_rows.len().saturating_sub(1))]
            .iter()
            .map(|r| (*r).clone())
            .collect()
    }

    /// Format rows for clipboard with current delimiter/header settings
    pub fn format_rows_for_copy(
        &self,
        rows: &[Vec<String>],
        include_header: bool,
        delimiter: CopyDelimiter,
    ) -> String {
        let delim = match delimiter {
            CopyDelimiter::Tab => "\t",
            CopyDelimiter::Comma => ",",
            CopyDelimiter::Pipe => "|",
        };

        let mut result = String::new();

        if include_header && !self.columns.is_empty() {
            result.push_str(&self.columns.join(delim));
            result.push('\n');
        }

        for row in rows {
            result.push_str(&row.join(delim));
            result.push('\n');
        }

        result
    }

    /// Format rows for clipboard with explicit columns
    pub fn format_rows_for_copy_with_cols(
        &self,
        rows: &[Vec<String>],
        columns: &[String],
        include_header: bool,
        delimiter: CopyDelimiter,
    ) -> String {
        let delim = match delimiter {
            CopyDelimiter::Tab => "\t",
            CopyDelimiter::Comma => ",",
            CopyDelimiter::Pipe => "|",
        };

        let mut result = String::new();

        if include_header && !columns.is_empty() {
            result.push_str(&columns.join(delim));
            result.push('\n');
        }

        for row in rows {
            result.push_str(&row.join(delim));
            result.push('\n');
        }

        result
    }

    pub fn open_connection_picker(&mut self) {
        if !self.connections.is_empty() {
            self.modal = Modal::ConnectionPicker;
            self.picker_filter.clear();
            self.picker_filtered = (0..self.connections.len()).collect();
            self.picker_index = 0;
            // Pre-select current connection if any
            if let Some(ref conn) = self.connection {
                if let Some(idx) = self.connections.iter().position(|c| c == conn) {
                    if let Some(filtered_idx) = self.picker_filtered.iter().position(|&i| i == idx)
                    {
                        self.picker_index = filtered_idx;
                    }
                }
            }
        }
    }

    pub fn close_modal(&mut self) {
        self.modal = Modal::None;
    }

    /// Open command palette
    pub fn open_command_palette(&mut self) {
        self.modal = Modal::CommandPalette;
        self.palette_filter.clear();
        self.palette_filtered = (0..Command::all().len()).collect();
        self.palette_index = 0;
    }

    /// Navigate down in command palette
    pub fn palette_next(&mut self) {
        if !self.palette_filtered.is_empty() {
            self.palette_index = (self.palette_index + 1) % self.palette_filtered.len();
        }
    }

    /// Navigate up in command palette
    pub fn palette_prev(&mut self) {
        if !self.palette_filtered.is_empty() {
            self.palette_index = self
                .palette_index
                .checked_sub(1)
                .unwrap_or(self.palette_filtered.len() - 1);
        }
    }

    /// Get currently selected command
    pub fn palette_selected_command(&self) -> Option<Command> {
        self.palette_filtered
            .get(self.palette_index)
            .and_then(|&idx| Command::all().get(idx).copied())
    }

    /// Add character to palette filter
    pub fn palette_filter_push(&mut self, c: char) {
        self.palette_filter.push(c);
        self.apply_palette_filter();
    }

    /// Remove character from palette filter
    pub fn palette_filter_pop(&mut self) {
        self.palette_filter.pop();
        self.apply_palette_filter();
    }

    /// Apply filter to command list
    fn apply_palette_filter(&mut self) {
        self.palette_filtered = Command::all()
            .iter()
            .enumerate()
            .filter(|(_, cmd)| cmd.matches(&self.palette_filter))
            .map(|(i, _)| i)
            .collect();
        // Reset selection if current is out of bounds
        if self.palette_index >= self.palette_filtered.len() {
            self.palette_index = 0;
        }
    }

    /// Query-history entries for the current connection, newest first.
    pub fn history_entries(&self) -> Vec<String> {
        match self.connection_key() {
            Some(key) => self.history.get(&key).iter().rev().cloned().collect(),
            None => Vec::new(),
        }
    }

    /// Open the query-history picker.
    pub fn open_history_picker(&mut self) {
        self.history_picker_filter.clear();
        self.history_picker_index = 0;
        self.apply_history_filter();
        self.modal = Modal::HistoryPicker;
    }

    pub fn history_picker_next(&mut self) {
        if !self.history_picker_filtered.is_empty() {
            self.history_picker_index =
                (self.history_picker_index + 1) % self.history_picker_filtered.len();
        }
    }

    pub fn history_picker_prev(&mut self) {
        if !self.history_picker_filtered.is_empty() {
            self.history_picker_index = self
                .history_picker_index
                .checked_sub(1)
                .unwrap_or(self.history_picker_filtered.len() - 1);
        }
    }

    pub fn history_picker_filter_push(&mut self, c: char) {
        self.history_picker_filter.push(c);
        self.apply_history_filter();
    }

    pub fn history_picker_filter_pop(&mut self) {
        self.history_picker_filter.pop();
        self.apply_history_filter();
    }

    /// Recompute the filtered history list from the current filter string.
    fn apply_history_filter(&mut self) {
        let entries = self.history_entries();
        let filter = self.history_picker_filter.to_lowercase();
        self.history_picker_filtered = entries
            .iter()
            .enumerate()
            .filter(|(_, q)| filter.is_empty() || q.to_lowercase().contains(&filter))
            .map(|(i, _)| i)
            .collect();
        if self.history_picker_index >= self.history_picker_filtered.len() {
            self.history_picker_index = 0;
        }
    }

    /// The query currently selected in the history picker.
    pub fn history_picker_selected(&self) -> Option<String> {
        let entries = self.history_entries();
        self.history_picker_filtered
            .get(self.history_picker_index)
            .and_then(|&i| entries.get(i).cloned())
    }

    pub fn picker_next(&mut self) {
        if !self.picker_filtered.is_empty() {
            self.picker_index = (self.picker_index + 1) % self.picker_filtered.len();
        }
    }

    pub fn picker_prev(&mut self) {
        if !self.picker_filtered.is_empty() {
            self.picker_index = self
                .picker_index
                .checked_sub(1)
                .unwrap_or(self.picker_filtered.len() - 1);
        }
    }

    pub fn picker_select(&mut self) {
        if let Some(&conn_idx) = self.picker_filtered.get(self.picker_index) {
            if let Some(conn) = self.connections.get(conn_idx) {
                self.pending_connection = Some(conn.clone());
            }
        }
        self.close_modal();
    }

    pub fn picker_filter_push(&mut self, c: char) {
        self.picker_filter.push(c);
        self.apply_picker_filter();
    }

    pub fn picker_filter_pop(&mut self) {
        self.picker_filter.pop();
        self.apply_picker_filter();
    }

    fn apply_picker_filter(&mut self) {
        let filter_lower = self.picker_filter.to_lowercase();
        self.picker_filtered = if self.picker_filter.is_empty() {
            (0..self.connections.len()).collect()
        } else {
            self.connections
                .iter()
                .enumerate()
                .filter(|(_, conn)| {
                    conn.profile.to_lowercase().contains(&filter_lower)
                        || conn.target.to_lowercase().contains(&filter_lower)
                        || conn.adapter.to_lowercase().contains(&filter_lower)
                })
                .map(|(i, _)| i)
                .collect()
        };
        // Reset selection to first match
        self.picker_index = 0;
    }

    pub fn get_filtered_connections(&self) -> Vec<&Connection> {
        self.picker_filtered
            .iter()
            .filter_map(|&i| self.connections.get(i))
            .collect()
    }

    pub fn set_connection(&mut self, conn: Connection) {
        self.status = format!("Connected: {}:{}", conn.profile, conn.target);
        // Save last connection
        self.app_state.last_connection = Some(format!("{}:{}", conn.profile, conn.target));
        let _ = self.app_state.save();
        self.connection = Some(conn);
    }

    pub fn set_results(&mut self, columns: Vec<String>, rows: Vec<Vec<String>>) {
        self.columns = columns;
        self.filtered_indices = (0..rows.len()).collect();
        self.results = rows;
        self.results_scroll = 0;
        self.results_hscroll = 0;
        self.sort_specs.clear();
        self.filter.clear();
    }

    pub fn toggle_focus(&mut self) {
        self.focus = match self.focus {
            Pane::Explorer => Pane::Editor,
            Pane::Editor => {
                if self.results_visible {
                    Pane::Results
                } else if self.explorer_visible {
                    Pane::Explorer
                } else {
                    Pane::Editor
                }
            }
            Pane::Results => {
                if self.explorer_visible {
                    Pane::Explorer
                } else {
                    Pane::Editor
                }
            }
        };
    }

    pub fn toggle_results(&mut self) {
        if self.has_run_query {
            self.results_visible = !self.results_visible;
            if !self.results_visible && self.focus == Pane::Results {
                self.focus = Pane::Editor;
            }
        }
    }

    pub fn toggle_split_direction(&mut self) {
        self.split_horizontal = !self.split_horizontal;
    }

    // Explorer methods
    #[allow(dead_code)]
    pub fn toggle_explorer(&mut self) {
        self.explorer_visible = !self.explorer_visible;
        if self.explorer_visible && self.explorer_nodes.is_empty() {
            self.rebuild_explorer_nodes();
        }
        // Reset horizontal scroll since pane width changed
        self.results_hscroll = 0;
    }

    pub fn set_schema_cache(&mut self, schemas: Vec<executor::SchemaInfo>) {
        self.schema_cache = schemas;
        self.rebuild_explorer_nodes();
    }

    pub fn rebuild_explorer_nodes(&mut self) {
        self.explorer_nodes.clear();
        let filter_lower = self.explorer_filter.to_lowercase();

        for schema in &self.schema_cache {
            // Schema node
            let schema_matches = filter_lower.is_empty()
                || schema.name.to_lowercase().contains(&filter_lower)
                || schema.tables.iter().any(|t| {
                    t.name.to_lowercase().contains(&filter_lower)
                        || t.columns
                            .iter()
                            .any(|c| c.name.to_lowercase().contains(&filter_lower))
                });

            if !schema_matches {
                continue;
            }

            let schema_expanded = self
                .explorer_nodes
                .iter()
                .find(|n| n.kind == NodeKind::Schema && n.name == schema.name)
                .map(|n| n.expanded)
                .unwrap_or(true); // Default expanded

            self.explorer_nodes.push(ExplorerNode {
                kind: NodeKind::Schema,
                name: schema.name.clone(),
                full_name: schema.name.clone(),
                depth: 0,
                expanded: schema_expanded,
                children_count: schema.tables.len(),
                data_type: None,
            });

            if !schema_expanded {
                continue;
            }

            // Group tables and views
            let tables: Vec<_> = schema
                .tables
                .iter()
                .filter(|t| t.table_type == "BASE TABLE")
                .collect();
            let views: Vec<_> = schema
                .tables
                .iter()
                .filter(|t| t.table_type != "BASE TABLE")
                .collect();

            // Tables group
            if !tables.is_empty() {
                let group_expanded = true;
                self.explorer_nodes.push(ExplorerNode {
                    kind: NodeKind::TableGroup,
                    name: "Tables".to_string(),
                    full_name: format!("{}.Tables", schema.name),
                    depth: 1,
                    expanded: group_expanded,
                    children_count: tables.len(),
                    data_type: None,
                });

                if group_expanded {
                    for table in &tables {
                        let table_matches = filter_lower.is_empty()
                            || table.name.to_lowercase().contains(&filter_lower)
                            || table
                                .columns
                                .iter()
                                .any(|c| c.name.to_lowercase().contains(&filter_lower));

                        if !table_matches {
                            continue;
                        }

                        let table_expanded = false; // Tables collapsed by default
                        self.explorer_nodes.push(ExplorerNode {
                            kind: NodeKind::Table,
                            name: table.name.clone(),
                            full_name: format!("{}.{}", schema.name, table.name),
                            depth: 2,
                            expanded: table_expanded,
                            children_count: table.columns.len(),
                            data_type: None,
                        });

                        if table_expanded {
                            for col in &table.columns {
                                self.explorer_nodes.push(ExplorerNode {
                                    kind: NodeKind::Column,
                                    name: col.name.clone(),
                                    full_name: format!(
                                        "{}.{}.{}",
                                        schema.name, table.name, col.name
                                    ),
                                    depth: 3,
                                    expanded: false,
                                    children_count: 0,
                                    data_type: Some(col.data_type.clone()),
                                });
                            }
                        }
                    }
                }
            }

            // Views group
            if !views.is_empty() {
                let group_expanded = true;
                self.explorer_nodes.push(ExplorerNode {
                    kind: NodeKind::TableGroup,
                    name: "Views".to_string(),
                    full_name: format!("{}.Views", schema.name),
                    depth: 1,
                    expanded: group_expanded,
                    children_count: views.len(),
                    data_type: None,
                });

                if group_expanded {
                    for view in &views {
                        let view_expanded = false;
                        self.explorer_nodes.push(ExplorerNode {
                            kind: NodeKind::Table,
                            name: view.name.clone(),
                            full_name: format!("{}.{}", schema.name, view.name),
                            depth: 2,
                            expanded: view_expanded,
                            children_count: view.columns.len(),
                            data_type: None,
                        });

                        if view_expanded {
                            for col in &view.columns {
                                self.explorer_nodes.push(ExplorerNode {
                                    kind: NodeKind::Column,
                                    name: col.name.clone(),
                                    full_name: format!(
                                        "{}.{}.{}",
                                        schema.name, view.name, col.name
                                    ),
                                    depth: 3,
                                    expanded: false,
                                    children_count: 0,
                                    data_type: Some(col.data_type.clone()),
                                });
                            }
                        }
                    }
                }
            }
        }

        // Clamp selection
        if self.explorer_selected >= self.explorer_nodes.len() {
            self.explorer_selected = self.explorer_nodes.len().saturating_sub(1);
        }
    }

    pub fn explorer_toggle_node(&mut self) {
        if let Some(node) = self.explorer_nodes.get_mut(self.explorer_selected) {
            if node.children_count > 0 {
                node.expanded = !node.expanded;
                // Rebuild to show/hide children
                let schemas = self.schema_cache.clone();
                self.rebuild_explorer_from_state(&schemas);
            }
        }
    }

    fn rebuild_explorer_from_state(&mut self, schemas: &[executor::SchemaInfo]) {
        // Store current expanded state
        let expanded_state: std::collections::HashMap<String, bool> = self
            .explorer_nodes
            .iter()
            .map(|n| (n.full_name.clone(), n.expanded))
            .collect();

        self.explorer_nodes.clear();
        let filter_lower = self.explorer_filter.to_lowercase();

        for schema in schemas {
            // Check which tables match the filter
            let matching_tables: Vec<_> = schema
                .tables
                .iter()
                .filter(|t| {
                    filter_lower.is_empty()
                        || schema.name.to_lowercase().contains(&filter_lower)
                        || t.name.to_lowercase().contains(&filter_lower)
                        || t.columns
                            .iter()
                            .any(|c| c.name.to_lowercase().contains(&filter_lower))
                })
                .collect();

            if matching_tables.is_empty() {
                continue;
            }

            let schema_key = schema.name.clone();
            let schema_expanded = *expanded_state.get(&schema_key).unwrap_or(&true);

            self.explorer_nodes.push(ExplorerNode {
                kind: NodeKind::Schema,
                name: schema.name.clone(),
                full_name: schema_key.clone(),
                depth: 0,
                expanded: schema_expanded,
                children_count: matching_tables.len(),
                data_type: None,
            });

            if !schema_expanded {
                continue;
            }

            let tables: Vec<_> = matching_tables
                .iter()
                .filter(|t| t.table_type == "BASE TABLE")
                .cloned()
                .collect();
            let views: Vec<_> = matching_tables
                .iter()
                .filter(|t| t.table_type != "BASE TABLE")
                .cloned()
                .collect();

            if !tables.is_empty() {
                let group_key = format!("{}.Tables", schema.name);
                let group_expanded = *expanded_state.get(&group_key).unwrap_or(&true);

                self.explorer_nodes.push(ExplorerNode {
                    kind: NodeKind::TableGroup,
                    name: "Tables".to_string(),
                    full_name: group_key,
                    depth: 1,
                    expanded: group_expanded,
                    children_count: tables.len(),
                    data_type: None,
                });

                if group_expanded {
                    for table in &tables {
                        // Filter columns if filter is active
                        let matching_cols: Vec<_> = table
                            .columns
                            .iter()
                            .filter(|c| {
                                filter_lower.is_empty()
                                    || schema.name.to_lowercase().contains(&filter_lower)
                                    || table.name.to_lowercase().contains(&filter_lower)
                                    || c.name.to_lowercase().contains(&filter_lower)
                            })
                            .collect();

                        let table_key = format!("{}.{}", schema.name, table.name);
                        let table_expanded = *expanded_state.get(&table_key).unwrap_or(&false);

                        self.explorer_nodes.push(ExplorerNode {
                            kind: NodeKind::Table,
                            name: table.name.clone(),
                            full_name: table_key,
                            depth: 2,
                            expanded: table_expanded,
                            children_count: matching_cols.len(),
                            data_type: None,
                        });

                        if table_expanded {
                            for col in &matching_cols {
                                self.explorer_nodes.push(ExplorerNode {
                                    kind: NodeKind::Column,
                                    name: col.name.clone(),
                                    full_name: format!(
                                        "{}.{}.{}",
                                        schema.name, table.name, col.name
                                    ),
                                    depth: 3,
                                    expanded: false,
                                    children_count: 0,
                                    data_type: Some(col.data_type.clone()),
                                });
                            }
                        }
                    }
                }
            }

            if !views.is_empty() {
                let group_key = format!("{}.Views", schema.name);
                let group_expanded = *expanded_state.get(&group_key).unwrap_or(&true);

                self.explorer_nodes.push(ExplorerNode {
                    kind: NodeKind::TableGroup,
                    name: "Views".to_string(),
                    full_name: group_key,
                    depth: 1,
                    expanded: group_expanded,
                    children_count: views.len(),
                    data_type: None,
                });

                if group_expanded {
                    for view in &views {
                        let matching_cols: Vec<_> = view
                            .columns
                            .iter()
                            .filter(|c| {
                                filter_lower.is_empty()
                                    || schema.name.to_lowercase().contains(&filter_lower)
                                    || view.name.to_lowercase().contains(&filter_lower)
                                    || c.name.to_lowercase().contains(&filter_lower)
                            })
                            .collect();

                        let view_key = format!("{}.{}", schema.name, view.name);
                        let view_expanded = *expanded_state.get(&view_key).unwrap_or(&false);

                        self.explorer_nodes.push(ExplorerNode {
                            kind: NodeKind::Table,
                            name: view.name.clone(),
                            full_name: view_key,
                            depth: 2,
                            expanded: view_expanded,
                            children_count: matching_cols.len(),
                            data_type: None,
                        });

                        if view_expanded {
                            for col in &matching_cols {
                                self.explorer_nodes.push(ExplorerNode {
                                    kind: NodeKind::Column,
                                    name: col.name.clone(),
                                    full_name: format!(
                                        "{}.{}.{}",
                                        schema.name, view.name, col.name
                                    ),
                                    depth: 3,
                                    expanded: false,
                                    children_count: 0,
                                    data_type: Some(col.data_type.clone()),
                                });
                            }
                        }
                    }
                }
            }
        }
    }

    pub fn explorer_next(&mut self) {
        if self.explorer_selected < self.explorer_nodes.len().saturating_sub(1) {
            self.explorer_selected += 1;
        }
    }

    pub fn explorer_prev(&mut self) {
        self.explorer_selected = self.explorer_selected.saturating_sub(1);
    }

    pub fn explorer_filter_push(&mut self, c: char) {
        self.explorer_filter.push(c);
        let schemas = self.schema_cache.clone();
        self.rebuild_explorer_from_state(&schemas);
    }

    pub fn explorer_filter_pop(&mut self) {
        self.explorer_filter.pop();
        let schemas = self.schema_cache.clone();
        self.rebuild_explorer_from_state(&schemas);
    }

    pub fn explorer_filter_clear(&mut self) {
        self.explorer_filter.clear();
        let schemas = self.schema_cache.clone();
        self.rebuild_explorer_from_state(&schemas);
    }

    pub fn get_selected_explorer_name(&self) -> Option<String> {
        self.explorer_nodes
            .get(self.explorer_selected)
            .map(|n| n.full_name.clone())
    }

    /// Toggle sort on a specific column (used by click-to-sort)
    pub fn sort_by_column(&mut self, col: usize) {
        if col >= self.columns.len() {
            return;
        }

        if let Some(pos) = self.sort_specs.iter().position(|s| s.column == col) {
            // Column is already being sorted - toggle direction or remove
            if self.sort_specs[pos].ascending {
                // Currently ascending -> switch to descending
                self.sort_specs[pos].ascending = false;
            } else {
                // Currently descending -> remove sort
                self.sort_specs.remove(pos);
            }
        } else {
            // Add new sort (ascending first, replace existing single-column sort)
            self.sort_specs.clear();
            self.sort_specs.push(SortSpec {
                column: col,
                ascending: true,
            });
        }
        self.apply_filter_and_sort();
    }

    // Visualization mode methods

    pub fn toggle_viz_mode(&mut self) {
        self.viz_mode = !self.viz_mode;
        if self.viz_mode {
            // Auto-select first column as group by if not set
            if self.viz_config.group_col.is_none() && !self.columns.is_empty() {
                self.viz_config.group_col = Some(0);
            }
            self.ensure_viz_value_col();
            self.compute_viz_data();
        }
    }

    /// Find the first column that looks numeric (optionally skipping `skip`).
    /// A column counts as numeric if it has at least one non-empty value and
    /// every non-empty/non-NULL value parses as a number (sampled).
    fn first_numeric_col(&self, skip: Option<usize>) -> Option<usize> {
        (0..self.columns.len())
            .filter(|c| Some(*c) != skip)
            .find(|&c| {
                let mut saw_value = false;
                for row in self.results.iter().take(50) {
                    if let Some(v) = row.get(c) {
                        let t = v.trim();
                        if t.is_empty() || t.eq_ignore_ascii_case("null") {
                            continue;
                        }
                        if t.parse::<f64>().is_ok() {
                            saw_value = true;
                        } else {
                            return false;
                        }
                    }
                }
                saw_value
            })
    }

    /// When the current aggregation needs a value column but none is selected,
    /// auto-pick the first numeric column (preferring one that isn't the group column).
    fn ensure_viz_value_col(&mut self) {
        if self.viz_config.agg_type.needs_value_col() && self.viz_config.value_col.is_none() {
            let skip = self.viz_config.group_col;
            if let Some(c) = self
                .first_numeric_col(skip)
                .or_else(|| self.first_numeric_col(None))
            {
                self.viz_config.value_col = Some(c);
            }
        }
    }

    pub fn set_viz_group_col(&mut self, col: usize) {
        if col < self.columns.len() {
            self.viz_config.group_col = Some(col);
            self.compute_viz_data();
        }
    }

    pub fn set_viz_value_col(&mut self, col: Option<usize>) {
        if col.map(|c| c < self.columns.len()).unwrap_or(true) {
            self.viz_config.value_col = col;
            self.compute_viz_data();
        }
    }

    pub fn cycle_viz_agg_type(&mut self) {
        let all = AggType::all();
        let current_idx = all
            .iter()
            .position(|a| *a == self.viz_config.agg_type)
            .unwrap_or(0);
        let next_idx = (current_idx + 1) % all.len();
        self.viz_config.agg_type = all[next_idx];
        self.ensure_viz_value_col();
        self.compute_viz_data();
    }

    pub fn viz_picker_next(&mut self) {
        self.viz_config.picker_focus = (self.viz_config.picker_focus + 1) % 3;
    }

    pub fn viz_picker_prev(&mut self) {
        self.viz_config.picker_focus = self.viz_config.picker_focus.checked_sub(1).unwrap_or(2);
    }

    /// Compute aggregated data for visualization
    pub fn compute_viz_data(&mut self) {
        use std::collections::HashMap;

        self.viz_data.clear();

        let group_col = match self.viz_config.group_col {
            Some(c) if c < self.columns.len() => c,
            _ => return,
        };

        // Use filtered indices if filter is active
        let row_indices: Vec<usize> = if !self.filter.is_empty() {
            self.filtered_indices.clone()
        } else {
            (0..self.results.len()).collect()
        };

        let mut groups: HashMap<String, Vec<f64>> = HashMap::new();

        for &row_idx in &row_indices {
            if let Some(row) = self.results.get(row_idx) {
                let label = row.get(group_col).cloned().unwrap_or_default();

                let value = if self.viz_config.agg_type.needs_value_col() {
                    // For SUM/AVG/MIN/MAX, parse value from value column
                    self.viz_config
                        .value_col
                        .and_then(|vc| row.get(vc))
                        .and_then(|v| v.parse::<f64>().ok())
                        .unwrap_or(0.0)
                } else {
                    // For COUNT, just count 1 per row
                    1.0
                };

                groups.entry(label).or_default().push(value);
            }
        }

        // Aggregate each group
        self.viz_data = groups
            .into_iter()
            .map(|(label, values)| {
                let agg_value = match self.viz_config.agg_type {
                    AggType::Count => values.len() as f64,
                    AggType::Sum => values.iter().sum(),
                    AggType::Avg => {
                        if values.is_empty() {
                            0.0
                        } else {
                            values.iter().sum::<f64>() / values.len() as f64
                        }
                    }
                    AggType::Min => values.iter().cloned().fold(f64::INFINITY, f64::min),
                    AggType::Max => values.iter().cloned().fold(f64::NEG_INFINITY, f64::max),
                };
                (label, agg_value)
            })
            .collect();

        // Sort by value descending
        self.viz_data
            .sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    }
}
