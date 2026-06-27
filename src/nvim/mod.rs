use anyhow::Result;
use nvim_rs::{create::tokio as create, Handler, Neovim, UiAttachOptions, Value};
use std::sync::{Arc, Mutex};
use tokio::process::{Child, ChildStdin, Command};
use tokio_util::compat::Compat;

// The writer type that nvim-rs expects
type Writer = Compat<ChildStdin>;

#[derive(Clone)]
pub struct NvimHandler {
    pub screen: Arc<Mutex<Screen>>,
}

impl NvimHandler {
    pub fn new(screen: Arc<Mutex<Screen>>) -> Self {
        Self { screen }
    }
}

#[async_trait::async_trait]
impl Handler for NvimHandler {
    type Writer = Writer;

    async fn handle_notify(&self, name: String, args: Vec<Value>, _neovim: Neovim<Self::Writer>) {
        if name == "redraw" {
            self.handle_redraw(args);
        }
    }
}

impl NvimHandler {
    fn handle_redraw(&self, args: Vec<Value>) {
        let mut screen = self.screen.lock().unwrap();

        for arg in args {
            if let Value::Array(event_batch) = arg {
                if event_batch.is_empty() {
                    continue;
                }

                let event_name = match &event_batch[0] {
                    Value::String(s) => s.as_str().unwrap_or(""),
                    _ => continue,
                };

                // Process each instance of this event
                for event_args in event_batch.iter().skip(1) {
                    if let Value::Array(params) = event_args {
                        self.process_event(&mut screen, event_name, params);
                    }
                }
            }
        }
    }

    fn process_event(&self, screen: &mut Screen, event_name: &str, params: &[Value]) {
        match event_name {
            "grid_resize" => {
                // [grid, width, height]
                if params.len() >= 3 {
                    let width = params[1].as_u64().unwrap_or(80) as usize;
                    let height = params[2].as_u64().unwrap_or(24) as usize;
                    screen.resize(width, height);
                }
            }
            "grid_clear" => {
                // [grid]
                screen.clear();
            }
            "grid_cursor_goto" => {
                // [grid, row, col]
                if params.len() >= 3 {
                    screen.cursor_row = params[1].as_u64().unwrap_or(0) as usize;
                    screen.cursor_col = params[2].as_u64().unwrap_or(0) as usize;
                }
            }
            "grid_line" => {
                // [grid, row, col_start, cells, ...]
                if params.len() >= 4 {
                    let row = params[1].as_u64().unwrap_or(0) as usize;
                    let col_start = params[2].as_u64().unwrap_or(0) as usize;
                    if let Value::Array(cells) = &params[3] {
                        self.process_grid_line(screen, row, col_start, cells);
                    }
                }
            }
            "mode_change" => {
                // [mode_name, mode_idx]
                if let Some(Value::String(mode)) = params.first() {
                    screen.mode = mode.as_str().unwrap_or("normal").to_string();
                }
            }
            "hl_attr_define" => {
                // [id, rgb_attrs, cterm_attrs, info]
                if params.len() >= 2 {
                    let id = params[0].as_u64().unwrap_or(0);
                    if let Value::Map(attrs) = &params[1] {
                        let mut hl = HlAttr::default();
                        for (key, val) in attrs {
                            if let Value::String(k) = key {
                                match k.as_str().unwrap_or("") {
                                    "foreground" => hl.fg = val.as_u64().map(|v| v as u32),
                                    "background" => hl.bg = val.as_u64().map(|v| v as u32),
                                    "bold" => hl.bold = val.as_bool().unwrap_or(false),
                                    "italic" => hl.italic = val.as_bool().unwrap_or(false),
                                    "underline" => hl.underline = val.as_bool().unwrap_or(false),
                                    _ => {}
                                }
                            }
                        }
                        screen.highlights.insert(id, hl);
                    }
                }
            }
            "grid_scroll"
                // [grid, top, bot, left, right, rows, cols]
                // Scroll a region of the grid. rows > 0 means scroll down (content moves up)
                if params.len() >= 6 => {
                    let top = params[1].as_u64().unwrap_or(0) as usize;
                    let bot = params[2].as_u64().unwrap_or(screen.height as u64) as usize;
                    let left = params[3].as_u64().unwrap_or(0) as usize;
                    let right = params[4].as_u64().unwrap_or(screen.width as u64) as usize;
                    let rows = params[5].as_i64().unwrap_or(0);

                    screen.scroll_region(top, bot, left, right, rows);
                }
            _ => {
                // Ignore other events for now
            }
        }
    }

    fn process_grid_line(
        &self,
        screen: &mut Screen,
        row: usize,
        col_start: usize,
        cells: &[Value],
    ) {
        if row >= screen.height {
            return;
        }

        let mut col = col_start;
        let mut last_hl_id = 0u64;

        for cell_data in cells {
            if let Value::Array(cell_arr) = cell_data {
                // Cell format: [text, hl_id?, repeat?]
                let text = match cell_arr.first() {
                    Some(Value::String(s)) => s.as_str().unwrap_or(" ").to_string(),
                    _ => " ".to_string(),
                };

                // Highlight ID (optional, defaults to last used)
                if let Some(Value::Integer(hl)) = cell_arr.get(1) {
                    last_hl_id = hl.as_u64().unwrap_or(0);
                }

                // Repeat count (optional, defaults to 1)
                let repeat = cell_arr.get(2).and_then(|v| v.as_u64()).unwrap_or(1) as usize;

                // Write cells
                for _ in 0..repeat {
                    if col < screen.width {
                        screen.cells[row][col] = Cell {
                            char: text.clone(),
                            hl_id: last_hl_id,
                        };
                        col += 1;
                    }
                }
            }
        }
    }
}

pub struct EmbeddedNvim {
    pub nvim: Neovim<Writer>,
    pub screen: Arc<Mutex<Screen>>,
    _io_handle: tokio::task::JoinHandle<Result<(), Box<nvim_rs::error::LoopError>>>,
    _child: Child,
}

impl EmbeddedNvim {
    pub async fn spawn(width: usize, height: usize) -> Result<Self> {
        // Create shared screen state
        let screen = Arc::new(Mutex::new(Screen::new(width, height)));
        let handler = NvimHandler::new(Arc::clone(&screen));

        let mut cmd = Command::new("nvim");
        cmd.args(["--embed"]); // No --headless since we're attaching a UI

        let (nvim, io_handle, child) = create::new_child_cmd(&mut cmd, handler).await?;

        // Attach UI with specified dimensions
        let mut opts = UiAttachOptions::new();
        opts.set_rgb(true);
        opts.set_linegrid_external(true);

        nvim.ui_attach(width as i64, height as i64, &opts).await?;

        // Create a scratch buffer for SQL editing
        nvim.command("enew").await?; // New empty buffer
        nvim.command("file query.sql").await?; // Give it a name for statusline
        nvim.command("setlocal buftype=nofile").await?; // Not associated with a file
        nvim.command("setlocal bufhidden=hide").await?; // Keep buffer when hidden
        nvim.command("setlocal noswapfile").await?;
        nvim.command("syntax enable").await?; // Enable syntax highlighting
        nvim.command("setlocal filetype=sql").await?;
        nvim.command("set number").await?;
        nvim.command("set termguicolors").await?; // Enable true color
        nvim.command("set laststatus=0").await?; // Hide statusline (we have our own)
        nvim.command("set cmdheight=1").await?; // Cmdline space
        nvim.command("set signcolumn=yes:1").await?; // Enable sign column for query border

        // Set up query region highlighting with left border
        nvim.exec_lua(r#"
            -- Create namespace for query highlighting
            _G.query_ns = vim.api.nvim_create_namespace('current_query')

            -- Define highlight group for the border
            vim.api.nvim_set_hl(0, 'QueryBorder', { fg = '#7aa2f7' })

            -- Function to find and highlight current query with left border
            function _G.highlight_current_query()
                local bufnr = vim.api.nvim_get_current_buf()
                -- Clear previous highlights
                vim.api.nvim_buf_clear_namespace(bufnr, _G.query_ns, 0, -1)

                local lines = vim.api.nvim_buf_get_lines(bufnr, 0, -1, false)
                if #lines == 0 then return end

                local content = table.concat(lines, '\n')
                local cursor = vim.api.nvim_win_get_cursor(0)
                local cursor_line = cursor[1]  -- 1-indexed
                local cursor_col = cursor[2]   -- 0-indexed

                -- Find query boundaries by looking at lines directly
                -- This is simpler and avoids offset calculation bugs

                -- Find start line: scan backwards from cursor for line ending with ;
                local start_line = 1
                for i = cursor_line - 1, 1, -1 do
                    if lines[i] and lines[i]:match(';%s*$') then
                        start_line = i + 1  -- Start on line after the semicolon
                        break
                    end
                end

                -- Skip empty/whitespace-only lines at start
                while start_line <= #lines and lines[start_line]:match('^%s*$') do
                    start_line = start_line + 1
                end

                -- Find end line: scan forwards from cursor for line ending with ;
                local end_line = #lines
                for i = cursor_line, #lines do
                    if lines[i] and lines[i]:match(';%s*$') then
                        end_line = i
                        break
                    end
                end

                -- Add left border sign to each line in the query
                for line = start_line, end_line do
                    if line <= #lines then
                        vim.api.nvim_buf_set_extmark(bufnr, _G.query_ns, line - 1, 0, {
                            sign_text = '▎',
                            sign_hl_group = 'QueryBorder',
                        })
                    end
                end
            end

            -- Set up autocmd to update highlight on cursor move
            vim.api.nvim_create_autocmd({'CursorMoved', 'CursorMovedI', 'TextChanged', 'TextChangedI'}, {
                pattern = '*.sql',
                callback = function()
                    _G.highlight_current_query()
                end
            })

            -- Initial highlight
            vim.defer_fn(function()
                _G.highlight_current_query()
            end, 100)
        "#, vec![]).await?;

        Ok(Self {
            nvim,
            screen,
            _io_handle: io_handle,
            _child: child,
        })
    }

    pub async fn get_buffer_contents(&self) -> Result<String> {
        let buf = self.nvim.get_current_buf().await?;
        let lines = buf.get_lines(0, -1, false).await?;
        Ok(lines.join("\n"))
    }

    pub async fn set_buffer_contents(&self, content: &str) -> Result<()> {
        let buf = self.nvim.get_current_buf().await?;
        let lines: Vec<String> = content.lines().map(|s| s.to_string()).collect();
        buf.set_lines(0, -1, false, lines).await?;
        Ok(())
    }

    /// Get the query at cursor position (delimited by semicolons)
    pub async fn get_query_at_cursor(&self) -> Result<String> {
        let buf = self.nvim.get_current_buf().await?;
        let lines = buf.get_lines(0, -1, false).await?;

        // Get cursor position (1-indexed line, 0-indexed col)
        let cursor = self.nvim.get_current_win().await?.get_cursor().await?;
        let cursor_line = cursor.0 as usize; // 1-indexed

        // Find start line: scan backwards for line ending with ;
        let mut start_line = 0; // 0-indexed
        for i in (0..cursor_line.saturating_sub(1)).rev() {
            if lines[i].trim_end().ends_with(';') {
                start_line = i + 1; // Start on line after the semicolon
                break;
            }
        }

        // Skip empty/whitespace-only lines at start
        while start_line < lines.len() && lines[start_line].trim().is_empty() {
            start_line += 1;
        }

        // Find end line: scan forwards for line ending with ;
        let mut end_line = lines.len().saturating_sub(1); // 0-indexed, default to last line
        for (i, line) in lines.iter().enumerate().skip(cursor_line.saturating_sub(1)) {
            if line.trim_end().ends_with(';') {
                end_line = i;
                break;
            }
        }

        // Build query from start_line to end_line (inclusive)
        let query_lines: Vec<&str> = lines[start_line..=end_line]
            .iter()
            .map(|s| s.as_str())
            .collect();
        let query = query_lines.join("\n").trim().to_string();

        // Remove trailing semicolon for dbt (it adds its own)
        let query = query.trim_end_matches(';').trim().to_string();

        Ok(query)
    }

    /// Get visual selection if in visual mode, otherwise None
    pub async fn get_visual_selection(&self) -> Result<Option<String>> {
        // Check current mode
        let mode = self.nvim.get_mode().await?;
        let mode_str = mode
            .iter()
            .find(|(k, _)| k.as_str().map(|s| s == "mode").unwrap_or(false))
            .and_then(|(_, v)| v.as_str())
            .unwrap_or("");

        // Only return selection if currently in visual mode (v, V, or Ctrl-V)
        // Don't use previous visual marks ('< and '>) as they persist and would
        // cause old selections to be used even when cursor is elsewhere
        if mode_str.starts_with('v') || mode_str.starts_with('V') || mode_str == "\x16" {
            // Yank selection to register z, get it, then restore
            self.nvim.command("normal! \"zy").await?;
            let selection = self
                .nvim
                .call_function("getreg", vec![Value::from("z")])
                .await?
                .as_str()
                .unwrap_or("")
                .to_string();

            if !selection.is_empty() {
                return Ok(Some(selection));
            }
        }

        Ok(None)
    }

    pub async fn send_input(&self, input: &str) -> Result<()> {
        self.nvim.input(input).await?;
        Ok(())
    }

    /// Paste text using nvim's paste API (handles modes correctly)
    pub async fn paste(&self, text: &str) -> Result<()> {
        // Use nvim_paste which handles paste correctly regardless of mode
        // The -1 phase means single-shot paste (not chunked)
        self.nvim.paste(text, true, -1).await?;
        Ok(())
    }

    pub async fn resize(&self, width: usize, height: usize) -> Result<()> {
        // Resize nvim UI
        self.nvim.ui_try_resize(width as i64, height as i64).await?;
        // Resize our screen buffer
        self.screen.lock().unwrap().resize(width, height);
        Ok(())
    }

    pub async fn quit(&self) -> Result<()> {
        self.nvim.command("qa!").await?;
        Ok(())
    }

    /// Set up SQL completion with the given words (tables, columns, keywords)
    pub async fn setup_sql_completion(&self, words: &[String]) -> Result<()> {
        // Escape words for Lua string
        let words_lua: Vec<String> = words
            .iter()
            .map(|w| format!("\"{}\"", w.replace('\\', "\\\\").replace('"', "\\\"")))
            .collect();
        let words_list = words_lua.join(", ");

        // Define completion function in Lua
        let lua_code = format!(
            r#"
            -- Store completion words globally
            _G.sql_completion_words = {{ {words_list} }}

            -- Omnifunc for SQL completion
            function _G.sql_omnifunc(findstart, base)
                if findstart == 1 then
                    -- Find start of word
                    local line = vim.fn.getline('.')
                    local col = vim.fn.col('.') - 1
                    while col > 0 and line:sub(col, col):match('[%w_.]') do
                        col = col - 1
                    end
                    return col
                else
                    -- Find matches
                    local matches = {{}}
                    local base_lower = base:lower()
                    for _, word in ipairs(_G.sql_completion_words) do
                        if word:lower():find(base_lower, 1, true) == 1 then
                            table.insert(matches, word)
                        end
                    end
                    -- Sort: exact case match first, then alphabetical
                    table.sort(matches, function(a, b)
                        local a_exact = a:find(base, 1, true) == 1
                        local b_exact = b:find(base, 1, true) == 1
                        if a_exact and not b_exact then return true end
                        if b_exact and not a_exact then return false end
                        return a:lower() < b:lower()
                    end)
                    return matches
                end
            end
        "#
        );

        self.nvim.exec_lua(&lua_code, vec![]).await?;

        // Set omnifunc for current buffer
        self.nvim
            .command("setlocal omnifunc=v:lua.sql_omnifunc")
            .await?;

        // Set up completion options
        self.nvim
            .command("set completeopt=menuone,noselect,preview")
            .await?;

        // Optional: trigger completion on . for schema.table
        self.nvim
            .command(r#"inoremap <buffer> . .<C-x><C-o>"#)
            .await?;

        Ok(())
    }
}

// Grid cell for rendering
#[derive(Debug, Clone, Default)]
pub struct Cell {
    pub char: String,
    pub hl_id: u64,
}

// Highlight attributes from nvim
#[derive(Debug, Clone, Default)]
pub struct HlAttr {
    pub fg: Option<u32>,
    pub bg: Option<u32>,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
}

// Screen state from neovim redraw events
#[derive(Debug, Clone)]
pub struct Screen {
    pub width: usize,
    pub height: usize,
    pub cells: Vec<Vec<Cell>>,
    pub cursor_row: usize,
    pub cursor_col: usize,
    pub mode: String,
    pub highlights: std::collections::HashMap<u64, HlAttr>,
}

impl Default for Screen {
    fn default() -> Self {
        Self::new(80, 24)
    }
}

impl Screen {
    pub fn new(width: usize, height: usize) -> Self {
        let cells = vec![vec![Cell::default(); width]; height];
        Self {
            width,
            height,
            cells,
            cursor_row: 0,
            cursor_col: 0,
            mode: String::from("normal"),
            highlights: std::collections::HashMap::new(),
        }
    }

    pub fn resize(&mut self, width: usize, height: usize) {
        // Resize while preserving existing content where possible
        // Nvim will send redraw events to update the content after resize

        // Adjust height
        if height > self.height {
            // Add new rows
            for _ in self.height..height {
                self.cells.push(vec![Cell::default(); self.width]);
            }
        } else if height < self.height {
            // Truncate rows
            self.cells.truncate(height);
        }

        // Adjust width for all rows
        if width != self.width {
            for row in &mut self.cells {
                if width > self.width {
                    // Extend with empty cells
                    row.resize(width, Cell::default());
                } else {
                    // Truncate
                    row.truncate(width);
                }
            }
        }

        self.width = width;
        self.height = height;
    }

    pub fn clear(&mut self) {
        for row in &mut self.cells {
            for cell in row {
                cell.char = String::from(" ");
                cell.hl_id = 0;
            }
        }
    }

    /// Scroll a region of the screen
    /// rows > 0: scroll down (content moves up, new blank lines at bottom)
    /// rows < 0: scroll up (content moves down, new blank lines at top)
    pub fn scroll_region(&mut self, top: usize, bot: usize, left: usize, right: usize, rows: i64) {
        let bot = bot.min(self.height);
        let right = right.min(self.width);

        if rows > 0 {
            // Scroll down: content moves up
            let shift = rows as usize;
            for row_idx in top..bot.saturating_sub(shift) {
                for col_idx in left..right {
                    if row_idx + shift < bot {
                        self.cells[row_idx][col_idx] = self.cells[row_idx + shift][col_idx].clone();
                    }
                }
            }
            // Clear the newly exposed rows at the bottom
            for row_idx in bot.saturating_sub(shift)..bot {
                for col_idx in left..right {
                    self.cells[row_idx][col_idx] = Cell::default();
                }
            }
        } else if rows < 0 {
            // Scroll up: content moves down
            let shift = (-rows) as usize;
            for row_idx in (top + shift..bot).rev() {
                for col_idx in left..right {
                    if row_idx >= shift {
                        self.cells[row_idx][col_idx] = self.cells[row_idx - shift][col_idx].clone();
                    }
                }
            }
            // Clear the newly exposed rows at the top
            for row_idx in top..top + shift.min(bot - top) {
                for col_idx in left..right {
                    self.cells[row_idx][col_idx] = Cell::default();
                }
            }
        }
    }

    /// Get all lines as strings (for rendering)
    #[allow(dead_code)]
    pub fn lines(&self) -> Vec<String> {
        self.cells
            .iter()
            .map(|row| row.iter().map(|c| c.char.as_str()).collect())
            .collect()
    }
}
