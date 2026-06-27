//! Application state types and enums

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pane {
    Explorer,
    Editor,
    Results,
}

/// Tree node types for schema explorer
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeKind {
    Schema,
    TableGroup, // "Tables" or "Views" header
    Table,
    Column,
}

/// A node in the explorer tree
#[derive(Debug, Clone)]
pub struct ExplorerNode {
    pub kind: NodeKind,
    pub name: String,
    pub full_name: String, // Fully qualified name (schema.table.column)
    pub depth: usize,
    pub expanded: bool,
    pub children_count: usize,     // For showing (n) count
    pub data_type: Option<String>, // For columns
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Modal {
    None,
    ConnectionPicker,
    Filter,
    SortPicker,
    ExplorerFilter,
    SaveQuery(String),        // filename input
    LoadQuery,                // file picker
    Help,                     // keybinding help overlay
    CopyOptions,              // delimiter/header options for copy
    CopyColumns,              // column picker for copy
    ExportColumns,            // column picker for CSV export
    LimitPicker,              // query row limit picker
    LimitCustom(String),      // custom limit text input
    CellDetail(usize, usize), // (row_index, col_index) - cell inspector panel
    CancelConfirm,            // confirm query cancellation
    ColumnStats(usize),       // column index - show quick stats for column
    HiddenColumns,            // toggle column visibility
    CommandPalette,           // command palette (Ctrl+p)
    HistoryPicker,            // query history picker (Ctrl+g)
}

/// Available commands in the command palette
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    ToggleResults,
    ToggleExplorer,
    ToggleSplitDirection,
    FormatSql,
    OpenConnectionPicker,
    OpenLimitPicker,
    SaveQuery,
    LoadQuery,
    ExportCsv,
    ShowHistory,
    ShowHelp,
}

impl Command {
    /// All available commands
    pub fn all() -> &'static [Command] {
        &[
            Command::ToggleResults,
            Command::ToggleExplorer,
            Command::ToggleSplitDirection,
            Command::FormatSql,
            Command::OpenConnectionPicker,
            Command::OpenLimitPicker,
            Command::SaveQuery,
            Command::LoadQuery,
            Command::ExportCsv,
            Command::ShowHistory,
            Command::ShowHelp,
        ]
    }

    /// Display name for the command
    pub fn display(&self) -> &'static str {
        match self {
            Command::ToggleResults => "Toggle Results Pane",
            Command::ToggleExplorer => "Toggle Schema Explorer",
            Command::ToggleSplitDirection => "Toggle Split Direction",
            Command::FormatSql => "Format SQL",
            Command::OpenConnectionPicker => "Change Connection",
            Command::OpenLimitPicker => "Set Query Limit",
            Command::SaveQuery => "Save Query to File",
            Command::LoadQuery => "Load Query from File",
            Command::ExportCsv => "Export Results to CSV",
            Command::ShowHistory => "Query History",
            Command::ShowHelp => "Show Keyboard Shortcuts",
        }
    }

    /// Check if command matches filter (case-insensitive)
    pub fn matches(&self, filter: &str) -> bool {
        if filter.is_empty() {
            return true;
        }
        let display = self.display().to_lowercase();
        let filter = filter.to_lowercase();
        display.contains(&filter)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CopyDelimiter {
    Tab,
    Comma,
    Pipe,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueryLimit {
    Limit(usize),
    NoLimit,
}

impl QueryLimit {
    pub fn display(&self) -> String {
        match self {
            QueryLimit::Limit(n) => format!("{}", n),
            QueryLimit::NoLimit => "No limit".to_string(),
        }
    }

    pub fn short_display(&self) -> String {
        match self {
            QueryLimit::Limit(n) if *n >= 1000 => format!("{}k", n / 1000),
            QueryLimit::Limit(n) => format!("{}", n),
            QueryLimit::NoLimit => "All".to_string(),
        }
    }
}

/// What the user is currently dragging
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DragMode {
    #[default]
    None,
    VScrollbar,
    HScrollbar,
    ResizeSplit,
    CellDetailResize,
    ColumnResize(usize), // Column index being resized
}

/// Sort specification for a single column
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SortSpec {
    pub column: usize,
    pub ascending: bool,
}

/// Aggregation type for visualization mode
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AggType {
    #[default]
    Count,
    Sum,
    Avg,
    Min,
    Max,
}

impl AggType {
    pub fn all() -> &'static [AggType] {
        &[
            AggType::Count,
            AggType::Sum,
            AggType::Avg,
            AggType::Min,
            AggType::Max,
        ]
    }

    pub fn display(&self) -> &'static str {
        match self {
            AggType::Count => "COUNT",
            AggType::Sum => "SUM",
            AggType::Avg => "AVG",
            AggType::Min => "MIN",
            AggType::Max => "MAX",
        }
    }

    pub fn needs_value_col(&self) -> bool {
        // COUNT doesn't need a value column (counts rows)
        !matches!(self, AggType::Count)
    }
}

/// Visualization configuration
#[derive(Debug, Clone, Default)]
pub struct VizConfig {
    /// Column to group by (the category/label)
    pub group_col: Option<usize>,
    /// Column to aggregate (for SUM/AVG/MIN/MAX)
    pub value_col: Option<usize>,
    /// Aggregation type
    pub agg_type: AggType,
    /// Which picker is active (0=group, 1=value, 2=agg type)
    pub picker_focus: usize,
}
