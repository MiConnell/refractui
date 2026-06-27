use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{
        Block, Borders, Clear, List, ListItem, Paragraph, Scrollbar, ScrollbarOrientation,
        ScrollbarState,
    },
    Frame,
};

use crate::app::{App, CopyDelimiter, Modal, NodeKind, Pane, QueryLimit};

pub fn render(frame: &mut Frame, app: &mut App) {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // Status bar
            Constraint::Min(10),   // Main area
            Constraint::Length(1), // Help line
        ])
        .split(frame.area());

    render_status_bar(frame, app, vertical[0]);
    render_help(frame, app, vertical[2]);

    // Main area: explorer (optional) | editor | results (optional)
    let main_area = vertical[1];

    // Determine content area (after explorer if visible)
    let content_area = if app.explorer_visible {
        let horizontal = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(35), // Explorer
                Constraint::Min(40),    // Editor/Results
            ])
            .split(main_area);

        render_explorer(frame, app, horizontal[0]);
        horizontal[1]
    } else {
        main_area
    };

    // Split content area between editor and results (if visible)
    if app.results_visible {
        let direction = if app.split_horizontal {
            Direction::Vertical
        } else {
            Direction::Horizontal
        };

        let chunks = Layout::default()
            .direction(direction)
            .constraints([
                Constraint::Percentage(app.split_percent),
                Constraint::Percentage(100 - app.split_percent),
            ])
            .split(content_area);

        render_editor(frame, app, chunks[0]);
        render_results(frame, app, chunks[1]);
    } else {
        // Editor takes full area
        render_editor(frame, app, content_area);
    }

    // Render modal on top if active
    if app.modal != Modal::None {
        match &app.modal {
            Modal::ConnectionPicker => render_connection_picker(frame, app),
            Modal::Filter => render_filter_input(frame, app),
            Modal::SortPicker => render_sort_picker(frame, app),
            Modal::ExplorerFilter => render_explorer_filter(frame, app),
            Modal::SaveQuery(filename) => render_save_query(frame, filename),
            Modal::LoadQuery => render_load_query(frame, app),
            Modal::Help => render_help_overlay(frame),
            Modal::CopyOptions => render_copy_options(frame, app),
            Modal::CopyColumns => render_copy_columns(frame, app),
            Modal::ExportColumns => render_export_columns(frame, app),
            Modal::LimitPicker => render_limit_picker(frame, app),
            Modal::LimitCustom(input) => render_limit_custom(frame, input),
            Modal::CellDetail(row_idx, col_idx) => {
                render_cell_detail(frame, app, *row_idx, *col_idx)
            }
            Modal::CancelConfirm => render_cancel_confirm(frame, app),
            Modal::ColumnStats(col_idx) => render_column_stats(frame, app, *col_idx),
            Modal::HiddenColumns => render_hidden_columns(frame, app),
            Modal::CommandPalette => render_command_palette(frame, app),
            Modal::HistoryPicker => render_history_picker(frame, app),
            Modal::None => {}
        }
    }
}

fn render_filter_input(frame: &mut Frame, app: &App) {
    // Render filter input at bottom of screen
    let area = frame.area();
    let input_area = Rect {
        x: 0,
        y: area.height.saturating_sub(3),
        width: area.width,
        height: 3,
    };

    // Clear the area
    frame.render_widget(Clear, input_area);

    let input_text = format!("/{}", app.filter);
    let block = Block::default()
        .title(" Filter (Enter/Esc to close) ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    let paragraph = Paragraph::new(input_text)
        .block(block)
        .style(Style::default().fg(Color::White));

    frame.render_widget(paragraph, input_area);

    // Position cursor at end of input
    frame.set_cursor_position((
        input_area.x + 1 + app.filter.len() as u16 + 1, // +1 for border, +1 for /
        input_area.y + 1,                               // +1 for border
    ));
}

fn render_explorer_filter(frame: &mut Frame, app: &App) {
    // Render explorer filter input at bottom of explorer area
    let area = frame.area();
    let input_area = Rect {
        x: 0,
        y: area.height.saturating_sub(3),
        width: 35.min(area.width), // Match explorer width
        height: 3,
    };

    frame.render_widget(Clear, input_area);

    let input_text = format!("/{}", app.explorer_filter);
    let block = Block::default()
        .title(" Schema Filter ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    let paragraph = Paragraph::new(input_text)
        .block(block)
        .style(Style::default().fg(Color::White));

    frame.render_widget(paragraph, input_area);

    frame.set_cursor_position((
        input_area.x + 1 + app.explorer_filter.len() as u16 + 1,
        input_area.y + 1,
    ));
}

fn render_explorer(frame: &mut Frame, app: &mut App, area: Rect) {
    let focused = app.focus == Pane::Explorer;
    let border_style = if focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let title = if app.explorer_filter.is_empty() {
        format!(" Schema ({}) ", app.explorer_nodes.len())
    } else {
        format!(" Schema [/{}] ", app.explorer_filter)
    };

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(border_style);

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if app.explorer_nodes.is_empty() {
        let msg = if app.schema_cache.is_empty() {
            "Press Ctrl+b to load schema"
        } else {
            "No matches"
        };
        let p = Paragraph::new(msg).style(Style::default().fg(Color::DarkGray));
        frame.render_widget(p, inner);
        return;
    }

    // Calculate visible range
    let visible_height = inner.height as usize;

    // Adjust scroll to keep selection visible
    if app.explorer_selected < app.explorer_scroll {
        app.explorer_scroll = app.explorer_selected;
    } else if app.explorer_selected >= app.explorer_scroll + visible_height {
        app.explorer_scroll = app.explorer_selected - visible_height + 1;
    }

    // Render nodes
    let mut lines: Vec<Line> = Vec::new();
    for (i, node) in app
        .explorer_nodes
        .iter()
        .enumerate()
        .skip(app.explorer_scroll)
        .take(visible_height)
    {
        let indent = "  ".repeat(node.depth);
        let icon = match node.kind {
            NodeKind::Schema => {
                if node.expanded {
                    "▼ "
                } else {
                    "▶ "
                }
            }
            NodeKind::TableGroup => {
                if node.expanded {
                    "▼ "
                } else {
                    "▶ "
                }
            }
            NodeKind::Table => {
                if node.expanded {
                    "▼ "
                } else {
                    "▶ "
                }
            }
            NodeKind::Column => "  ",
        };

        let name_style = match node.kind {
            NodeKind::Schema => Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
            NodeKind::TableGroup => Style::default().fg(Color::Cyan),
            NodeKind::Table => Style::default().fg(Color::White),
            NodeKind::Column => Style::default().fg(Color::Rgb(180, 180, 200)), // Brighter for readability
        };

        let count_str = if node.children_count > 0 && !node.expanded {
            format!(" ({})", node.children_count)
        } else {
            String::new()
        };

        let type_str = node
            .data_type
            .as_ref()
            .map(|t| format!(" {}", t))
            .unwrap_or_default();

        let is_selected = i == app.explorer_selected;
        let line_style = if is_selected {
            Style::default().bg(Color::DarkGray)
        } else {
            Style::default()
        };

        let line = Line::from(vec![
            Span::styled(indent, line_style),
            Span::styled(icon, line_style),
            Span::styled(&node.name, name_style.patch(line_style)),
            Span::styled(
                count_str,
                Style::default().fg(Color::DarkGray).patch(line_style),
            ),
            Span::styled(type_str, Style::default().fg(Color::Blue).patch(line_style)),
        ]);
        lines.push(line);
    }

    let p = Paragraph::new(lines);
    frame.render_widget(p, inner);
}

fn render_status_bar(frame: &mut Frame, app: &App, area: Rect) {
    // Get mode from screen
    let mode = {
        let screen = app.screen.lock().unwrap();
        screen.mode.clone()
    };

    let (mode_text, mode_color) = match mode.as_str() {
        "normal" => ("NORMAL", Color::Blue),
        "insert" => ("INSERT", Color::Green),
        "visual" => ("VISUAL", Color::Magenta),
        "replace" => ("REPLACE", Color::Red),
        "command" => ("COMMAND", Color::Yellow),
        _ => (&mode[..], Color::Gray),
    };

    let conn_info = app
        .connection
        .as_ref()
        .map(|c| format!(" {}:{} ", c.profile, c.target))
        .unwrap_or_else(|| " No connection ".to_string());

    let limit_info = format!(" [{}] ", app.query_limit.short_display());

    let status = Line::from(vec![
        Span::styled(
            format!(" {} ", mode_text),
            Style::default()
                .bg(mode_color)
                .fg(Color::Black)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(conn_info, Style::default().fg(Color::Cyan)),
        Span::styled(limit_info, Style::default().fg(Color::Yellow)),
        Span::styled(" ", Style::default()),
        Span::styled(&app.status, Style::default().fg(Color::DarkGray)),
    ]);

    frame.render_widget(Paragraph::new(status), area);
}

fn render_editor(frame: &mut Frame, app: &mut App, area: Rect) {
    // Store area for mouse handling
    app.editor_area = (area.x, area.y, area.width, area.height);

    let focused = app.focus == Pane::Editor;

    let border_style = if focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(border_style);

    // Get inner area for content
    let inner = block.inner(area);

    // Render the block first
    frame.render_widget(block, area);

    // Get screen content from nvim with syntax highlighting
    let screen = app.screen.lock().unwrap();
    let lines: Vec<Line> = screen
        .cells
        .iter()
        .take(inner.height as usize)
        .map(|row| {
            let spans: Vec<Span> = row
                .iter()
                .take(inner.width as usize)
                .map(|cell| {
                    let mut style = Style::default();
                    if let Some(hl) = screen.highlights.get(&cell.hl_id) {
                        if let Some(fg) = hl.fg {
                            style = style.fg(Color::Rgb(
                                ((fg >> 16) & 0xFF) as u8,
                                ((fg >> 8) & 0xFF) as u8,
                                (fg & 0xFF) as u8,
                            ));
                        }
                        if let Some(bg) = hl.bg {
                            style = style.bg(Color::Rgb(
                                ((bg >> 16) & 0xFF) as u8,
                                ((bg >> 8) & 0xFF) as u8,
                                (bg & 0xFF) as u8,
                            ));
                        }
                        if hl.bold {
                            style = style.add_modifier(Modifier::BOLD);
                        }
                        if hl.italic {
                            style = style.add_modifier(Modifier::ITALIC);
                        }
                        if hl.underline {
                            style = style.add_modifier(Modifier::UNDERLINED);
                        }
                    }
                    Span::styled(cell.char.clone(), style)
                })
                .collect();
            Line::from(spans)
        })
        .collect();

    // Render nvim content
    let content = Paragraph::new(lines);
    frame.render_widget(content, inner);

    // Render cursor if editor is focused
    if focused
        && screen.cursor_row < inner.height as usize
        && screen.cursor_col < inner.width as usize
    {
        frame.set_cursor_position((
            inner.x + screen.cursor_col as u16,
            inner.y + screen.cursor_row as u16,
        ));
    }
}

fn render_results(frame: &mut Frame, app: &mut App, area: Rect) {
    // Store area for mouse handling
    app.results_area = (area.x, area.y, area.width, area.height);

    let focused = app.focus == Pane::Results;

    let border_style = if focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    // Build title with row/column count and filter info
    let row_count = app.results.len();
    let col_count = app.columns.len();
    let title = if app.filter.is_empty() {
        format!(" Results ({} rows x {} cols) ", row_count, col_count)
    } else {
        format!(
            " Results ({}/{} rows x {} cols) [filter: {}] ",
            app.filtered_row_count(),
            row_count,
            col_count,
            app.filter
        )
    };

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(border_style);

    // Show loading state with animation and cancel button
    if app.loading {
        let spinner = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
        let frame_char = spinner[app.loading_frame % spinner.len()];
        let inner = block.inner(area);
        frame.render_widget(block, area);

        // Center the loading message vertically
        let v_center = inner.y + inner.height / 2;

        // Loading spinner line
        let spinner_line = Line::from(vec![
            Span::styled(
                format!("{} ", frame_char),
                Style::default().fg(Color::Yellow),
            ),
            Span::styled("Running query...", Style::default().fg(Color::Yellow)),
        ]);
        let spinner_para = Paragraph::new(spinner_line).alignment(Alignment::Center);
        let spinner_area = Rect {
            x: inner.x,
            y: v_center.saturating_sub(2),
            width: inner.width,
            height: 1,
        };
        frame.render_widget(spinner_para, spinner_area);

        // Cancel button - centered box
        let button_text = " ✕ Cancel ";
        let button_width = button_text.len() as u16 + 2; // +2 for border
        let button_x = inner.x + (inner.width.saturating_sub(button_width)) / 2;
        let button_y = v_center + 1;
        let button_area = Rect {
            x: button_x,
            y: button_y,
            width: button_width,
            height: 3,
        };

        // Store button area for click detection
        app.cancel_button_area = Some((
            button_area.x,
            button_area.y,
            button_area.width,
            button_area.height,
        ));

        let cancel_btn = Paragraph::new(button_text)
            .alignment(Alignment::Center)
            .style(Style::default().fg(Color::White))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Red)),
            );
        frame.render_widget(cancel_btn, button_area);

        return;
    } else {
        app.cancel_button_area = None;
    }

    // Show error if present
    if let Some(ref error) = app.error {
        let error_text = format!("Error: {}", error);
        let content = Paragraph::new(error_text)
            .block(block)
            .style(Style::default().fg(Color::Red))
            .wrap(ratatui::widgets::Wrap { trim: false });
        frame.render_widget(content, area);
        return;
    }

    if app.columns.is_empty() {
        let msg = if app.has_run_query {
            "Query returned 0 rows"
        } else {
            "No results yet. Press Ctrl+e to execute query."
        };
        let content = Paragraph::new(msg)
            .block(block)
            .style(Style::default().fg(Color::DarkGray));
        frame.render_widget(content, area);
        return;
    }

    // Visualization mode - render bar chart instead of table
    if app.viz_mode {
        render_viz_chart(frame, app, area, block);
        return;
    }

    // Get inner area (leave 1 col for scrollbar)
    let block_inner = block.inner(area);
    let inner = Rect {
        x: block_inner.x,
        y: block_inner.y,
        width: block_inner.width.saturating_sub(1), // Reserve space for scrollbar
        height: block_inner.height,
    };
    // Calculate visible rows (inner height minus header row)
    let visible_rows = inner.height as usize;
    let header_lines = 1;
    let data_rows_to_show = visible_rows.saturating_sub(header_lines);
    app.visible_rows = data_rows_to_show;

    // Get filtered/sorted rows
    let display_rows = app.get_display_rows();
    let total_rows = display_rows.len();

    // Calculate row number width based on total rows
    let row_num_width = total_rows.to_string().len().max(2);

    // Get visible columns (not hidden)
    let visible_cols: Vec<usize> = (0..app.columns.len())
        .filter(|i| !app.hidden_columns.contains(i))
        .collect();

    // Calculate column widths based on content (or use custom widths if set) - only for visible columns
    let col_widths: Vec<usize> = visible_cols
        .iter()
        .map(|&i| {
            let name = &app.columns[i];
            // Use custom width if set, otherwise calculate automatically
            if let Some(&custom_width) = app.custom_col_widths.get(&i) {
                custom_width.max(4) // Minimum width of 4
            } else {
                let header_width = name.len() + 4; // +4 for sort indicator
                let data_width = display_rows
                    .iter()
                    .take(100)
                    .filter_map(|row| row.get(i))
                    .map(|cell| cell.len())
                    .max()
                    .unwrap_or(0);
                header_width.max(data_width).max(4)
            }
        })
        .collect();

    // Calculate total content width and max horizontal scroll (include row number column and separators)
    let total_width: usize = row_num_width + 1 + col_widths.iter().map(|w| w + 2).sum::<usize>(); // +2 for separator + space
    let visible_width = inner.width as usize;
    let max_hscroll = total_width.saturating_sub(visible_width);

    // Render block
    frame.render_widget(block, area);

    let grid_color = Color::Rgb(70, 70, 85); // More visible grid

    // Calculate column x positions (accounting for separators)
    let mut col_x_positions: Vec<usize> = Vec::new();
    let mut x = row_num_width + 1; // After row number and first separator
    for width in &col_widths {
        col_x_positions.push(x);
        x += width + 2; // column width + space + separator
    }

    // Calculate row scroll offset
    let row_offset = if app.results_scroll >= visible_rows {
        app.results_scroll - visible_rows + 1
    } else {
        0
    };

    // Alternating row colors for zebra striping
    let band_color = Color::Rgb(30, 32, 40); // Subtle alternate row background

    // Build vertical grid lines for each row
    let build_vgrid = || -> String {
        let mut line_chars: Vec<char> = vec![' '; total_width + 10];
        if row_num_width < line_chars.len() {
            line_chars[row_num_width] = '│';
        }
        let mut x = row_num_width + 1;
        for (i, width) in col_widths.iter().enumerate() {
            x += width + 1;
            if i < col_widths.len() - 1 && x < line_chars.len() {
                line_chars[x] = '│';
            }
            x += 1;
        }
        line_chars.into_iter().collect()
    };

    let vgrid = build_vgrid();

    // Build grid background with alternating colors
    let mut grid_lines: Vec<Line> = Vec::new();

    // Header row
    let header_grid: String = vgrid
        .chars()
        .skip(app.results_hscroll)
        .take(inner.width as usize)
        .collect();
    grid_lines.push(Line::styled(header_grid, Style::default().fg(grid_color)));

    // Data rows with alternating backgrounds
    for i in 0..data_rows_to_show {
        let row_idx = row_offset + i;
        let is_banded = row_idx % 2 == 1;
        let scrolled: String = vgrid
            .chars()
            .skip(app.results_hscroll)
            .take(inner.width as usize)
            .collect();

        let style = if is_banded {
            Style::default().fg(grid_color).bg(band_color)
        } else {
            Style::default().fg(grid_color)
        };
        grid_lines.push(Line::styled(scrolled, style));
    }

    // Render grid background
    let grid = Paragraph::new(grid_lines);
    frame.render_widget(grid, inner);

    // Now render content on top of grid
    // Header row
    let header_y = inner.y;

    // Row number header
    let row_num_x = inner.x
        + (row_num_width as u16)
            .saturating_sub(1)
            .saturating_sub(app.results_hscroll as u16);
    if row_num_x >= inner.x && row_num_x < inner.x + inner.width {
        frame.render_widget(
            Paragraph::new("#").style(
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Rect {
                x: row_num_x,
                y: header_y,
                width: 1,
                height: 1,
            },
        );
    }

    // Column headers (only visible columns)
    for (vis_col_idx, &orig_col_idx) in visible_cols.iter().enumerate() {
        let col_x = col_x_positions.get(vis_col_idx).copied().unwrap_or(0);
        if col_x < app.results_hscroll {
            continue;
        }
        let screen_x = inner.x + (col_x - app.results_hscroll) as u16;
        if screen_x >= inner.x + inner.width {
            break;
        }

        let name = &app.columns[orig_col_idx];
        let indicator = if let Some((priority, ascending)) = app.get_sort_priority(orig_col_idx) {
            let arrow = if ascending { "▲" } else { "▼" };
            if app.sort_specs.len() > 1 {
                format!("{}{}", priority, arrow)
            } else {
                arrow.to_string()
            }
        } else {
            String::new()
        };

        let col_width = col_widths.get(vis_col_idx).copied().unwrap_or(10);
        let available_width = (inner.x + inner.width).saturating_sub(screen_x) as usize;
        let display_width = col_width.min(available_width);

        let text = format!("{}{}", name, indicator);
        let truncated: String = text.chars().take(display_width).collect();

        frame.render_widget(
            Paragraph::new(truncated).style(
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Rect {
                x: screen_x,
                y: header_y,
                width: display_width as u16,
                height: 1,
            },
        );
    }

    // Data rows (1 line per row, tight layout)
    for (vis_idx, (display_idx, row)) in display_rows
        .iter()
        .enumerate()
        .skip(row_offset)
        .take(data_rows_to_show)
        .enumerate()
    {
        let row_y = inner.y + header_lines as u16 + vis_idx as u16;
        if row_y >= inner.y + inner.height {
            break;
        }

        // Determine row style
        let (sel_start, sel_end) = app.get_selection_range();
        let is_selected =
            app.selection_anchor.is_some() && display_idx >= sel_start && display_idx <= sel_end;
        let is_cursor = display_idx == app.results_scroll;
        let is_banded = display_idx % 2 == 1;

        // Determine background color
        let bg_color = if is_cursor {
            Some(Color::Rgb(60, 60, 75))
        } else if is_selected {
            Some(Color::Rgb(50, 50, 80))
        } else if is_banded {
            Some(band_color)
        } else {
            None
        };

        // Draw background and grid if needed
        if let Some(bg) = bg_color {
            let bg_line: String = " ".repeat(inner.width as usize);
            frame.render_widget(
                Paragraph::new(bg_line).style(Style::default().bg(bg)),
                Rect {
                    x: inner.x,
                    y: row_y,
                    width: inner.width,
                    height: 1,
                },
            );
            // Redraw grid lines on top of background
            let scrolled: String = vgrid
                .chars()
                .skip(app.results_hscroll)
                .take(inner.width as usize)
                .collect();
            frame.render_widget(
                Paragraph::new(scrolled).style(Style::default().fg(grid_color).bg(bg)),
                Rect {
                    x: inner.x,
                    y: row_y,
                    width: inner.width,
                    height: 1,
                },
            );
        }

        let text_style = Style::default().fg(Color::White);
        let null_style = Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::ITALIC);

        // Row number
        let row_num_text = format!("{:>width$}", display_idx + 1, width = row_num_width);
        if app.results_hscroll < row_num_width {
            let start = app.results_hscroll;
            let text: String = row_num_text.chars().skip(start).collect();
            frame.render_widget(
                Paragraph::new(text).style(Style::default().fg(Color::DarkGray)),
                Rect {
                    x: inner.x,
                    y: row_y,
                    width: (row_num_width - start) as u16,
                    height: 1,
                },
            );
        }

        // Cell content (only visible columns)
        for (vis_col_idx, &orig_col_idx) in visible_cols.iter().enumerate() {
            let cell = row.get(orig_col_idx).map(|s| s.as_str()).unwrap_or("");
            // Check if cell is NULL (empty or literal "NULL")
            let is_null = cell.is_empty() || cell == "NULL" || cell == "null";
            let (display_text, style) = if is_null {
                ("NULL".to_string(), null_style)
            } else {
                (cell.to_string(), text_style)
            };

            let col_x = col_x_positions.get(vis_col_idx).copied().unwrap_or(0);
            if col_x < app.results_hscroll {
                // Partially visible or fully scrolled
                let col_width = col_widths.get(vis_col_idx).copied().unwrap_or(10);
                if col_x + col_width > app.results_hscroll {
                    // Partially visible
                    let skip = app.results_hscroll - col_x;
                    let text: String = display_text
                        .chars()
                        .skip(skip)
                        .take(col_width - skip)
                        .collect();
                    frame.render_widget(
                        Paragraph::new(text).style(style),
                        Rect {
                            x: inner.x,
                            y: row_y,
                            width: (col_width - skip) as u16,
                            height: 1,
                        },
                    );
                }
                continue;
            }
            let screen_x = inner.x + (col_x - app.results_hscroll) as u16;
            if screen_x >= inner.x + inner.width {
                break;
            }

            let col_width = col_widths.get(vis_col_idx).copied().unwrap_or(10);
            let available_width = (inner.x + inner.width).saturating_sub(screen_x) as usize;
            let display_width = col_width.min(available_width);

            let truncated: String = display_text.chars().take(display_width).collect();
            frame.render_widget(
                Paragraph::new(truncated).style(style),
                Rect {
                    x: screen_x,
                    y: row_y,
                    width: display_width as u16,
                    height: 1,
                },
            );
        }
    }

    // Render vertical scrollbar if needed
    if total_rows > visible_rows {
        let scrollbar_area = Rect {
            x: block_inner.x + block_inner.width.saturating_sub(1),
            y: block_inner.y + 1, // Start after header
            width: 1,
            height: block_inner.height.saturating_sub(2), // Exclude header and bottom row for hscroll
        };

        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(Some("↑"))
            .end_symbol(Some("↓"))
            .track_symbol(Some("│"))
            .thumb_symbol("█");

        // Use max scrollable range for better thumb sizing
        let scroll_range = total_rows.saturating_sub(visible_rows);
        let mut scrollbar_state =
            ScrollbarState::new(scroll_range.max(1)).position(app.results_scroll.min(scroll_range));

        frame.render_stateful_widget(scrollbar, scrollbar_area, &mut scrollbar_state);
    }

    // Render horizontal scrollbar if needed
    if max_hscroll > 0 {
        let hscrollbar_area = Rect {
            x: block_inner.x,
            y: block_inner.y + block_inner.height.saturating_sub(1),
            width: block_inner.width.saturating_sub(1), // Leave space for corner
            height: 1,
        };

        let hscrollbar = Scrollbar::new(ScrollbarOrientation::HorizontalBottom)
            .begin_symbol(Some("←"))
            .end_symbol(Some("→"))
            .track_symbol(Some("─"))
            .thumb_symbol("━");

        // Use max scrollable range for better thumb sizing
        let mut hscrollbar_state =
            ScrollbarState::new(max_hscroll.max(1)).position(app.results_hscroll.min(max_hscroll));

        frame.render_stateful_widget(hscrollbar, hscrollbar_area, &mut hscrollbar_state);
    }

    // Update state (after display_rows borrow is released)
    app.max_hscroll = max_hscroll;
    app.col_widths = col_widths;
    app.visible_cols = visible_cols;
    app.row_num_width = row_num_width;
}

fn render_viz_chart(frame: &mut Frame, app: &App, area: Rect, block: Block) {
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if app.viz_data.is_empty() {
        let msg = Paragraph::new("No data to visualize. Select a group-by column.")
            .style(Style::default().fg(Color::DarkGray))
            .alignment(Alignment::Center);
        frame.render_widget(msg, inner);
        return;
    }

    // Layout: header (config) + chart area
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(5)])
        .split(inner);

    // Render config header
    let group_col_name = app
        .viz_config
        .group_col
        .and_then(|i| app.columns.get(i))
        .map(|s| s.as_str())
        .unwrap_or("(none)");

    let value_col_name = if app.viz_config.agg_type.needs_value_col() {
        app.viz_config
            .value_col
            .and_then(|i| app.columns.get(i))
            .map(|s| s.as_str())
            .unwrap_or("(none)")
    } else {
        "*"
    };

    let agg_display = app.viz_config.agg_type.display();

    // Column pickers - clickable
    let group_style = if app.viz_config.picker_focus == 0 {
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Cyan)
    };
    let value_style = if app.viz_config.picker_focus == 1 {
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Cyan)
    };
    let agg_style = if app.viz_config.picker_focus == 2 {
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Cyan)
    };

    let config_line = Line::from(vec![
        Span::raw(" Group: "),
        Span::styled(format!("[{}]", group_col_name), group_style),
        Span::raw("  Agg: "),
        Span::styled(format!("[{}]", agg_display), agg_style),
        Span::raw("("),
        Span::styled(value_col_name, value_style),
        Span::raw(")"),
        Span::styled(
            "  Tab:cycle  j/k:col  a:agg  v:exit",
            Style::default().fg(Color::DarkGray),
        ),
    ]);
    let config_para = Paragraph::new(config_line);
    frame.render_widget(config_para, chunks[0]);

    // Render bar chart
    let chart_area = chunks[1];
    let max_value = app.viz_data.iter().map(|(_, v)| *v).fold(0.0_f64, f64::max);
    if max_value == 0.0 {
        return;
    }

    // Calculate label width (max label length)
    let label_width = app
        .viz_data
        .iter()
        .map(|(label, _)| label.len())
        .max()
        .unwrap_or(10)
        .min(30); // Cap at 30 chars

    // Space for value display
    let value_width = 12;

    // Available width for bar
    let bar_width = chart_area.width as usize - label_width - value_width - 4;

    // Render bars (limited by height)
    let max_bars = chart_area.height as usize;
    let bars_to_show = app.viz_data.len().min(max_bars);

    for (i, (label, value)) in app.viz_data.iter().take(bars_to_show).enumerate() {
        let y = chart_area.y + i as u16;
        if y >= chart_area.y + chart_area.height {
            break;
        }

        // Truncate label if needed
        let label_display: String = if label.len() > label_width {
            format!("{}…", &label[..label_width - 1])
        } else {
            format!("{:width$}", label, width = label_width)
        };

        // Calculate bar length
        let bar_len = ((value / max_value) * bar_width as f64) as usize;

        // Unicode bar characters for smooth gradients
        let bar: String = "█".repeat(bar_len);

        // Format value
        let value_display = if *value == value.floor() {
            format!("{:.0}", value)
        } else {
            format!("{:.2}", value)
        };

        // Color based on position (gradient from bright to dim)
        let bar_color = match i {
            0 => Color::Green,
            1 => Color::Cyan,
            2 => Color::Blue,
            _ => Color::DarkGray,
        };

        let line = Line::from(vec![
            Span::styled(label_display, Style::default().fg(Color::White)),
            Span::raw(" "),
            Span::styled(bar, Style::default().fg(bar_color)),
            Span::raw(" "),
            Span::styled(value_display, Style::default().fg(Color::Yellow)),
        ]);

        let bar_area = Rect {
            x: chart_area.x,
            y,
            width: chart_area.width,
            height: 1,
        };
        frame.render_widget(Paragraph::new(line), bar_area);
    }

    // Show "and N more..." if truncated
    if app.viz_data.len() > bars_to_show {
        let more = app.viz_data.len() - bars_to_show;
        let msg = format!("... and {} more", more);
        let y = chart_area.y + bars_to_show as u16;
        if y < chart_area.y + chart_area.height {
            let more_para = Paragraph::new(msg).style(Style::default().fg(Color::DarkGray));
            let more_area = Rect {
                x: chart_area.x,
                y,
                width: chart_area.width,
                height: 1,
            };
            frame.render_widget(more_para, more_area);
        }
    }
}

fn render_help(frame: &mut Frame, app: &App, area: Rect) {
    // Get current vim mode
    let mode = {
        let screen = app.screen.lock().unwrap();
        screen.mode.clone()
    };
    let is_insert = mode == "insert";

    let help = if app.focus == Pane::Results {
        Line::from(vec![
            Span::styled(" j/k ", Style::default().bg(Color::DarkGray)),
            Span::raw(" nav "),
            Span::styled(" v ", Style::default().bg(Color::DarkGray)),
            Span::raw(" select "),
            Span::styled(" y ", Style::default().bg(Color::DarkGray)),
            Span::raw(" copy "),
            Span::styled(" s ", Style::default().bg(Color::DarkGray)),
            Span::raw(" sort "),
            Span::styled(" / ", Style::default().bg(Color::DarkGray)),
            Span::raw(" filter "),
        ])
    } else if app.focus == Pane::Editor && !is_insert {
        // Show vim help when in non-insert mode
        Line::from(vec![
            Span::styled(" i ", Style::default().bg(Color::Yellow).fg(Color::Black)),
            Span::raw(" to type "),
            Span::styled(" C-e ", Style::default().bg(Color::DarkGray)),
            Span::raw(" run "),
            Span::styled(" : ", Style::default().bg(Color::DarkGray)),
            Span::raw(" vim cmd "),
            Span::styled(" ? ", Style::default().bg(Color::DarkGray)),
            Span::raw(" help "),
        ])
    } else {
        Line::from(vec![
            Span::styled(" C-e ", Style::default().bg(Color::DarkGray)),
            Span::raw(" run "),
            Span::styled(" C-r ", Style::default().bg(Color::DarkGray)),
            Span::raw(" results "),
            Span::styled(" C-b ", Style::default().bg(Color::DarkGray)),
            Span::raw(" explorer "),
            Span::styled(" C-f ", Style::default().bg(Color::DarkGray)),
            Span::raw(" format "),
            Span::styled(" ? ", Style::default().bg(Color::DarkGray)),
            Span::raw(" help "),
        ])
    };

    frame.render_widget(Paragraph::new(help), area);
}

fn render_connection_picker(frame: &mut Frame, app: &App) {
    // Use full screen
    let area = frame.area();

    // Split into filter input and list
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Filter input
            Constraint::Min(5),    // Connection list
        ])
        .split(area);

    // Clear the area
    frame.render_widget(Clear, area);

    // Render filter input
    let filter_block = Block::default()
        .title(" Filter (type to search, ↑/↓ or Ctrl-j/k to navigate) ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    let filter_text = if app.picker_filter.is_empty() {
        Span::styled("Type to filter...", Style::default().fg(Color::DarkGray))
    } else {
        Span::raw(&app.picker_filter)
    };

    let filter_para = Paragraph::new(filter_text).block(filter_block);
    frame.render_widget(filter_para, chunks[0]);

    // Build list items from filtered connections
    let filtered_conns = app.get_filtered_connections();
    let items: Vec<ListItem> = filtered_conns
        .iter()
        .enumerate()
        .map(|(i, conn)| {
            let style = if i == app.picker_index {
                Style::default()
                    .bg(Color::Blue)
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };

            let marker = if Some(*conn) == app.connection.as_ref() {
                " ● "
            } else {
                "   "
            };

            ListItem::new(format!("{}{}", marker, conn)).style(style)
        })
        .collect();

    let title = format!(
        " Connections ({}/{}) - Enter to select, Esc to cancel ",
        filtered_conns.len(),
        app.connections.len()
    );

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    let list = List::new(items).block(block);
    frame.render_widget(list, chunks[1]);

    // Position cursor in filter input
    frame.set_cursor_position((
        chunks[0].x + 1 + app.picker_filter.len() as u16,
        chunks[0].y + 1,
    ));
}

fn render_sort_picker(frame: &mut Frame, app: &App) {
    // Use full screen
    let popup_area = frame.area();

    // Clear the area behind the popup
    frame.render_widget(Clear, popup_area);

    // Build list items
    let items: Vec<ListItem> = app
        .columns
        .iter()
        .enumerate()
        .map(|(i, col)| {
            let is_selected = i == app.sort_picker_index;
            let sort_info = app.get_sort_priority(i);

            let (prefix, suffix) = if let Some((priority, ascending)) = sort_info {
                let arrow = if ascending { "▲" } else { "▼" };
                (format!("[{}{}] ", priority, arrow), "")
            } else {
                ("[ ] ".to_string(), "")
            };

            let style = if is_selected {
                Style::default()
                    .bg(Color::Blue)
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD)
            } else if sort_info.is_some() {
                Style::default().fg(Color::Green)
            } else {
                Style::default().fg(Color::White)
            };

            ListItem::new(format!("{}{}{}", prefix, col, suffix)).style(style)
        })
        .collect();

    let block = Block::default()
        .title(" Sort Columns (Enter=toggle, a=asc, d=desc, c=clear, Esc=close) ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    let list = List::new(items).block(block);

    frame.render_widget(list, popup_area);
}

fn render_save_query(frame: &mut Frame, filename: &str) {
    let area = frame.area();
    let input_area = Rect {
        x: area.width / 4,
        y: area.height / 2 - 2,
        width: area.width / 2,
        height: 3,
    };

    frame.render_widget(Clear, input_area);

    let block = Block::default()
        .title(" Save Query (Enter to save, Esc to cancel) ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    let display_name = if filename.is_empty() {
        "filename.sql".to_string()
    } else if filename.ends_with(".sql") {
        filename.to_string()
    } else {
        format!("{}.sql", filename)
    };

    let paragraph = Paragraph::new(display_name)
        .block(block)
        .style(if filename.is_empty() {
            Style::default().fg(Color::DarkGray)
        } else {
            Style::default().fg(Color::White)
        });

    frame.render_widget(paragraph, input_area);

    frame.set_cursor_position((input_area.x + 1 + filename.len() as u16, input_area.y + 1));
}

fn render_load_query(frame: &mut Frame, app: &App) {
    let area = frame.area();
    let popup_area = Rect {
        x: area.width / 4,
        y: area.height / 4,
        width: area.width / 2,
        height: area.height / 2,
    };

    frame.render_widget(Clear, popup_area);

    if app.saved_queries.is_empty() {
        let block = Block::default()
            .title(" Load Query ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan));

        let msg = Paragraph::new("No saved queries found.\nSave queries with Ctrl+s")
            .block(block)
            .style(Style::default().fg(Color::DarkGray));

        frame.render_widget(msg, popup_area);
        return;
    }

    let items: Vec<ListItem> = app
        .saved_queries
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let style = if i == app.load_query_index {
                Style::default()
                    .bg(Color::Blue)
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };
            ListItem::new(name.as_str()).style(style)
        })
        .collect();

    let block = Block::default()
        .title(format!(
            " Load Query ({}) - Enter load, d delete, Esc cancel ",
            app.saved_queries.len()
        ))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    let list = List::new(items).block(block);
    frame.render_widget(list, popup_area);
}

fn render_help_overlay(frame: &mut Frame) {
    let area = frame.area();

    let header = |s: &'static str| {
        Line::styled(
            s,
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )
    };

    // Left column: Global + Editor
    let left_col = vec![
        header("Global"),
        Line::raw("  Ctrl+q       Quit"),
        Line::raw("  Ctrl+c       Connection picker"),
        Line::raw("  Ctrl+e       Execute query"),
        Line::raw("  Ctrl+f       Format SQL"),
        Line::raw("  Ctrl+r       Toggle results"),
        Line::raw("  Ctrl+p       Command palette"),
        Line::raw("  Ctrl+t       Toggle split h/v"),
        Line::raw("  Ctrl+b       Toggle explorer"),
        Line::raw("  Ctrl+g       Query history"),
        Line::raw("  Ctrl+s       Save query to file"),
        Line::raw("  Ctrl+o       Load query from file"),
        Line::raw("  Ctrl+x       Export results to CSV"),
        Line::raw("  Click [10k]  Query row limit"),
        Line::raw("  Tab          Switch focus"),
        Line::raw("  ?            Show this help"),
        Line::raw(""),
        header("Editor"),
        Line::raw("  Ctrl+x Ctrl+o  SQL autocomplete"),
        Line::raw("  .              Trigger completion"),
    ];

    // Right column: Results + Explorer
    let right_col = vec![
        header("Results"),
        Line::raw("  j/k          Navigate rows"),
        Line::raw("  h/l          Scroll horizontally"),
        Line::raw("  Ctrl+d/u     Page down/up"),
        Line::raw("  gg / G       First / last row"),
        Line::raw("  M / L        Middle / bottom row"),
        Line::raw("  0 / $        Scroll start / end"),
        Line::raw("  s            Sort picker"),
        Line::raw("  S            Clear sort"),
        Line::raw("  /            Filter results"),
        Line::raw("  v            Visual select mode"),
        Line::raw("  y            Copy (with options)"),
        Line::raw("  Y            Quick copy"),
        Line::raw("  H            Hide/show columns"),
        Line::raw("  V            Visualization mode"),
        Line::raw("  Click hdr    Sort by column"),
        Line::raw("  R-click hdr  Column stats"),
        Line::raw("  Dbl-click    Cell inspector"),
        Line::raw(""),
        header("Explorer"),
        Line::raw("  j/k          Navigate"),
        Line::raw("  h/l          Collapse / expand"),
        Line::raw("  Enter/Space  Expand/collapse"),
        Line::raw("  i            Insert name to editor"),
        Line::raw("  y            Copy name"),
        Line::raw("  /            Filter schema"),
        Line::raw("  r            Refresh schema"),
        Line::raw("  q            Close explorer"),
    ];

    // Size popup to the taller column (plus borders), clamped to the screen
    let content_height = left_col.len().max(right_col.len()) as u16 + 2;
    let popup_width = 80.min(area.width.saturating_sub(4));
    let popup_height = content_height.min(area.height.saturating_sub(2));
    let popup_area = Rect {
        x: (area.width.saturating_sub(popup_width)) / 2,
        y: (area.height.saturating_sub(popup_height)) / 2,
        width: popup_width,
        height: popup_height,
    };

    frame.render_widget(Clear, popup_area);

    let block = Block::default()
        .title(" Help (press any key to close) ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));
    let inner = block.inner(popup_area);
    frame.render_widget(block, popup_area);

    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(inner);

    frame.render_widget(Paragraph::new(left_col), cols[0]);
    frame.render_widget(Paragraph::new(right_col), cols[1]);
}

fn render_copy_options(frame: &mut Frame, app: &App) {
    let area = frame.area();

    let popup_width = 40.min(area.width.saturating_sub(4));
    let popup_height = 12;
    let popup_area = Rect {
        x: (area.width - popup_width) / 2,
        y: (area.height - popup_height) / 2,
        width: popup_width,
        height: popup_height,
    };

    frame.render_widget(Clear, popup_area);

    let (start, end) = app.get_selection_range();
    let row_count = end - start + 1;

    let tab_marker = if app.copy_delimiter == CopyDelimiter::Tab {
        "*"
    } else {
        " "
    };
    let comma_marker = if app.copy_delimiter == CopyDelimiter::Comma {
        "*"
    } else {
        " "
    };
    let pipe_marker = if app.copy_delimiter == CopyDelimiter::Pipe {
        "*"
    } else {
        " "
    };
    let header_marker = if app.copy_include_header { "Yes" } else { "No" };

    let text = vec![
        Line::raw(format!(
            "Copying {} row{}",
            row_count,
            if row_count == 1 { "" } else { "s" }
        )),
        Line::raw(""),
        Line::styled("Delimiter:", Style::default().add_modifier(Modifier::BOLD)),
        Line::raw(format!("  [{}] (1/c) Comma", comma_marker)),
        Line::raw(format!("  [{}] (2/t) Tab", tab_marker)),
        Line::raw(format!("  [{}] (3/p) Pipe", pipe_marker)),
        Line::raw(""),
        Line::raw(format!("  (h) Include header: {}", header_marker)),
        Line::raw(""),
        Line::styled(
            "Enter/y to copy, Esc to cancel",
            Style::default().fg(Color::DarkGray),
        ),
    ];

    let block = Block::default()
        .title(" Copy Options ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    let paragraph = Paragraph::new(text).block(block);
    frame.render_widget(paragraph, popup_area);
}

fn render_copy_columns(frame: &mut Frame, app: &App) {
    let area = frame.area();

    let popup_width = 50.min(area.width.saturating_sub(4));
    let popup_height = (app.columns.len() + 6).min(area.height.saturating_sub(4) as usize) as u16;
    let popup_area = Rect {
        x: (area.width - popup_width) / 2,
        y: (area.height - popup_height) / 2,
        width: popup_width,
        height: popup_height,
    };

    frame.render_widget(Clear, popup_area);

    let selected_count = app.export_columns.iter().filter(|&&x| x).count();
    let (sel_start, sel_end) = app.get_selection_range();
    let row_count = sel_end - sel_start + 1;

    let items: Vec<ListItem> = app
        .columns
        .iter()
        .enumerate()
        .map(|(i, col)| {
            let selected = app.export_columns.get(i).copied().unwrap_or(false);
            let checkbox = if selected { "[x]" } else { "[ ]" };
            let style = if i == app.export_picker_index {
                Style::default().bg(Color::DarkGray)
            } else {
                Style::default()
            };
            ListItem::new(Line::styled(format!(" {} {}", checkbox, col), style))
        })
        .collect();

    let block = Block::default()
        .title(format!(
            " Copy {} row{} - Columns ({}/{}) ",
            row_count,
            if row_count == 1 { "" } else { "s" },
            selected_count,
            app.columns.len()
        ))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    let inner = block.inner(popup_area);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(2)])
        .split(inner);

    frame.render_widget(block, popup_area);

    let list = List::new(items);
    frame.render_widget(list, chunks[0]);

    let help = Paragraph::new(vec![Line::styled(
        "Space: toggle  a: toggle all  Enter: next",
        Style::default().fg(Color::DarkGray),
    )]);
    frame.render_widget(help, chunks[1]);
}

fn render_export_columns(frame: &mut Frame, app: &App) {
    let area = frame.area();

    let popup_width = 50.min(area.width.saturating_sub(4));
    let popup_height = (app.columns.len() + 6).min(area.height.saturating_sub(4) as usize) as u16;
    let popup_area = Rect {
        x: (area.width - popup_width) / 2,
        y: (area.height - popup_height) / 2,
        width: popup_width,
        height: popup_height,
    };

    frame.render_widget(Clear, popup_area);

    let selected_count = app.export_columns.iter().filter(|&&x| x).count();

    let items: Vec<ListItem> = app
        .columns
        .iter()
        .enumerate()
        .map(|(i, col)| {
            let selected = app.export_columns.get(i).copied().unwrap_or(false);
            let checkbox = if selected { "[x]" } else { "[ ]" };
            let style = if i == app.export_picker_index {
                Style::default().bg(Color::DarkGray)
            } else {
                Style::default()
            };
            ListItem::new(Line::styled(format!(" {} {}", checkbox, col), style))
        })
        .collect();

    let block = Block::default()
        .title(format!(
            " Export Columns ({}/{}) ",
            selected_count,
            app.columns.len()
        ))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    // Split area for list and help text
    let inner = block.inner(popup_area);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(2)])
        .split(inner);

    frame.render_widget(block, popup_area);

    let list = List::new(items);
    frame.render_widget(list, chunks[0]);

    let help = Paragraph::new(vec![Line::styled(
        "Space: toggle  a: toggle all  Enter: export",
        Style::default().fg(Color::DarkGray),
    )]);
    frame.render_widget(help, chunks[1]);
}

fn render_limit_picker(frame: &mut Frame, app: &App) {
    let area = frame.area();

    const LIMIT_OPTIONS: [(QueryLimit, &str); 5] = [
        (QueryLimit::Limit(100), "100"),
        (QueryLimit::Limit(1000), "1,000"),
        (QueryLimit::Limit(10000), "10,000"),
        (QueryLimit::Limit(100000), "100,000"),
        (QueryLimit::NoLimit, "No limit (all rows)"),
    ];

    let popup_width = 35.min(area.width.saturating_sub(4));
    let popup_height = 10;
    let popup_area = Rect {
        x: (area.width - popup_width) / 2,
        y: (area.height - popup_height) / 2,
        width: popup_width,
        height: popup_height,
    };

    frame.render_widget(Clear, popup_area);

    let items: Vec<ListItem> = LIMIT_OPTIONS
        .iter()
        .enumerate()
        .map(|(i, (limit, label))| {
            let selected = app.query_limit == *limit;
            let marker = if selected { "*" } else { " " };
            let style = if i == app.limit_picker_index {
                Style::default().bg(Color::DarkGray)
            } else {
                Style::default()
            };
            ListItem::new(Line::styled(
                format!(" [{}] ({}) {}", marker, i + 1, label),
                style,
            ))
        })
        .collect();

    let block = Block::default()
        .title(format!(
            " Query Limit: {} ",
            app.query_limit.short_display()
        ))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    let inner = block.inner(popup_area);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(inner);

    frame.render_widget(block, popup_area);

    let list = List::new(items);
    frame.render_widget(list, chunks[0]);

    let help = Paragraph::new(Line::styled(
        "1-5: select  c: custom  Enter: confirm  Esc: cancel",
        Style::default().fg(Color::DarkGray),
    ));
    frame.render_widget(help, chunks[1]);
}

fn render_limit_custom(frame: &mut Frame, input: &str) {
    let area = frame.area();

    let popup_width = 30.min(area.width.saturating_sub(4));
    let popup_height = 5;
    let popup_area = Rect {
        x: (area.width - popup_width) / 2,
        y: (area.height - popup_height) / 2,
        width: popup_width,
        height: popup_height,
    };

    frame.render_widget(Clear, popup_area);

    let block = Block::default()
        .title(" Custom Limit ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    let inner = block.inner(popup_area);
    frame.render_widget(block, popup_area);

    let text = vec![
        Line::raw(format!(" > {}_", input)),
        Line::styled(
            " Enter: set  Esc: back",
            Style::default().fg(Color::DarkGray),
        ),
    ];
    let paragraph = Paragraph::new(text);
    frame.render_widget(paragraph, inner);
}

fn render_cell_detail(frame: &mut Frame, app: &App, row_idx: usize, col_idx: usize) {
    // Use results area to position panel
    let (res_x, res_y, res_w, res_h) = app.results_area;

    // Calculate panel width based on percentage of results area
    let panel_width = ((res_w as u32 * app.cell_detail_width as u32) / 100) as u16;
    let panel_width = panel_width.max(20).min(res_w.saturating_sub(10));

    // Position panel on the right side of results area
    let panel_area = Rect {
        x: res_x + res_w.saturating_sub(panel_width),
        y: res_y,
        width: panel_width,
        height: res_h,
    };

    frame.render_widget(Clear, panel_area);

    // Get the cell value and column name
    let display_rows = app.get_display_rows();
    let col_name = app
        .columns
        .get(col_idx)
        .map(|s| s.as_str())
        .unwrap_or("Column");
    let cell_value = display_rows
        .get(row_idx)
        .and_then(|row| row.get(col_idx))
        .map(|s| s.as_str())
        .unwrap_or("");

    let title = format!(" {} (row {}) ", col_name, row_idx + 1);
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    let inner = block.inner(panel_area);
    frame.render_widget(block, panel_area);

    // Split inner area: content area + help line
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(2)])
        .split(inner);

    // Render the cell value with word wrap
    let content = Paragraph::new(cell_value)
        .style(Style::default().fg(Color::White))
        .wrap(ratatui::widgets::Wrap { trim: false });
    frame.render_widget(content, chunks[0]);

    // Render help line
    let help_text = vec![Line::from(vec![
        Span::styled(" y ", Style::default().bg(Color::DarkGray)),
        Span::raw(" copy "),
        Span::styled(" h/l ", Style::default().bg(Color::DarkGray)),
        Span::raw(" prev/next col "),
        Span::styled(" j/k ", Style::default().bg(Color::DarkGray)),
        Span::raw(" next/prev row "),
        Span::styled(" Esc ", Style::default().bg(Color::DarkGray)),
        Span::raw(" close "),
    ])];
    let help = Paragraph::new(help_text);
    frame.render_widget(help, chunks[1]);
}

fn render_cancel_confirm(frame: &mut Frame, app: &mut App) {
    let area = frame.area();

    let popup_width = 40.min(area.width.saturating_sub(4));
    let popup_height = 7;
    let popup_area = Rect {
        x: (area.width - popup_width) / 2,
        y: (area.height - popup_height) / 2,
        width: popup_width,
        height: popup_height,
    };

    frame.render_widget(Clear, popup_area);

    let block = Block::default()
        .title(" Cancel Query? ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow));

    let inner = block.inner(popup_area);
    frame.render_widget(block, popup_area);

    // Message
    let msg = Paragraph::new("Are you sure you want to cancel the running query?")
        .alignment(Alignment::Center)
        .wrap(ratatui::widgets::Wrap { trim: true });
    let msg_area = Rect {
        x: inner.x,
        y: inner.y,
        width: inner.width,
        height: 2,
    };
    frame.render_widget(msg, msg_area);

    // Buttons row
    let btn_y = inner.y + 3;
    let btn_width = 10u16;
    let gap = 4u16;
    let total_btn_width = btn_width * 2 + gap;
    let btn_start_x = inner.x + (inner.width.saturating_sub(total_btn_width)) / 2;

    // Yes button
    let yes_area = Rect {
        x: btn_start_x,
        y: btn_y,
        width: btn_width,
        height: 3,
    };
    let yes_btn = Paragraph::new("  Yes  ")
        .alignment(Alignment::Center)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Red)),
        );
    frame.render_widget(yes_btn, yes_area);

    // No button
    let no_area = Rect {
        x: btn_start_x + btn_width + gap,
        y: btn_y,
        width: btn_width,
        height: 3,
    };
    let no_btn = Paragraph::new("   No   ")
        .alignment(Alignment::Center)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Green)),
        );
    frame.render_widget(no_btn, no_area);

    // Store button areas for click detection
    app.confirm_yes_area = Some((yes_area.x, yes_area.y, yes_area.width, yes_area.height));
    app.confirm_no_area = Some((no_area.x, no_area.y, no_area.width, no_area.height));
}

fn render_column_stats(frame: &mut Frame, app: &App, col_idx: usize) {
    let area = frame.area();

    let popup_width = 45.min(area.width.saturating_sub(4));
    let popup_height = 12;
    let popup_area = Rect {
        x: (area.width - popup_width) / 2,
        y: (area.height - popup_height) / 2,
        width: popup_width,
        height: popup_height,
    };

    frame.render_widget(Clear, popup_area);

    let col_name = app
        .columns
        .get(col_idx)
        .map(|s| s.as_str())
        .unwrap_or("Column");
    let title = format!(" {} Stats ", col_name);

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    let inner = block.inner(popup_area);
    frame.render_widget(block, popup_area);

    // Calculate stats from the current data
    let display_rows = app.get_display_rows();
    let values: Vec<&str> = display_rows
        .iter()
        .filter_map(|row| row.get(col_idx))
        .map(|s| s.as_str())
        .collect();

    let total = values.len();
    let nulls = values
        .iter()
        .filter(|v| v.is_empty() || **v == "NULL" || **v == "null")
        .count();
    let non_null = total - nulls;

    // Count distinct values
    let distinct: std::collections::HashSet<&str> = values.iter().copied().collect();
    let distinct_count = distinct.len();

    // Try to find min/max (as strings)
    let non_null_values: Vec<&str> = values
        .iter()
        .filter(|v| !v.is_empty() && **v != "NULL" && **v != "null")
        .copied()
        .collect();
    let min_val = non_null_values.iter().min().copied().unwrap_or("-");
    let max_val = non_null_values.iter().max().copied().unwrap_or("-");

    let stats_text = vec![
        Line::from(vec![
            Span::styled("Total rows:    ", Style::default().fg(Color::DarkGray)),
            Span::styled(format!("{}", total), Style::default().fg(Color::White)),
        ]),
        Line::from(vec![
            Span::styled("Non-null:      ", Style::default().fg(Color::DarkGray)),
            Span::styled(format!("{}", non_null), Style::default().fg(Color::Green)),
        ]),
        Line::from(vec![
            Span::styled("Null:          ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("{}", nulls),
                Style::default().fg(if nulls > 0 {
                    Color::Yellow
                } else {
                    Color::White
                }),
            ),
        ]),
        Line::from(vec![
            Span::styled("Distinct:      ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("{}", distinct_count),
                Style::default().fg(Color::Cyan),
            ),
        ]),
        Line::raw(""),
        Line::from(vec![
            Span::styled("Min:           ", Style::default().fg(Color::DarkGray)),
            Span::raw(min_val.chars().take(20).collect::<String>()),
        ]),
        Line::from(vec![
            Span::styled("Max:           ", Style::default().fg(Color::DarkGray)),
            Span::raw(max_val.chars().take(20).collect::<String>()),
        ]),
        Line::raw(""),
        Line::from(vec![
            Span::styled(" Esc ", Style::default().bg(Color::DarkGray)),
            Span::raw(" close  "),
            Span::styled(" h ", Style::default().bg(Color::DarkGray)),
            Span::raw(" hide  "),
            Span::styled(" H ", Style::default().bg(Color::DarkGray)),
            Span::raw(" manage"),
        ]),
    ];

    let content = Paragraph::new(stats_text);
    frame.render_widget(content, inner);
}

fn render_hidden_columns(frame: &mut Frame, app: &App) {
    let area = frame.area();

    let popup_width = 40.min(area.width.saturating_sub(4));
    let popup_height = (app.columns.len() + 4).min(area.height.saturating_sub(4) as usize) as u16;
    let popup_area = Rect {
        x: (area.width - popup_width) / 2,
        y: (area.height - popup_height) / 2,
        width: popup_width,
        height: popup_height,
    };

    frame.render_widget(Clear, popup_area);

    let block = Block::default()
        .title(" Show/Hide Columns ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    let inner = block.inner(popup_area);
    frame.render_widget(block, popup_area);

    let items: Vec<ListItem> = app
        .columns
        .iter()
        .enumerate()
        .map(|(i, col)| {
            let is_hidden = app.hidden_columns.contains(&i);
            let checkbox = if is_hidden { "☐" } else { "☑" };
            let style = if i == app.hidden_columns_index {
                Style::default()
                    .bg(Color::Blue)
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD)
            } else if is_hidden {
                Style::default().fg(Color::DarkGray)
            } else {
                Style::default().fg(Color::White)
            };
            ListItem::new(format!(" {} {}", checkbox, col)).style(style)
        })
        .collect();

    let list = List::new(items);
    frame.render_widget(list, inner);
}

fn render_command_palette(frame: &mut Frame, app: &App) {
    use crate::app::Command;

    let area = frame.area();

    let popup_width = 50.min(area.width.saturating_sub(4));
    let popup_height = 15.min(area.height.saturating_sub(4));
    let popup_area = Rect {
        x: (area.width - popup_width) / 2,
        y: area.height / 6, // Position near top like VS Code
        width: popup_width,
        height: popup_height,
    };

    frame.render_widget(Clear, popup_area);

    let block = Block::default()
        .title(" Command Palette ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    let inner = block.inner(popup_area);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // Filter input
            Constraint::Length(1), // Separator
            Constraint::Min(1),    // Command list
        ])
        .split(inner);

    frame.render_widget(block, popup_area);

    // Filter input
    let filter_text = if app.palette_filter.is_empty() {
        Span::styled("Type to filter...", Style::default().fg(Color::DarkGray))
    } else {
        Span::styled(&app.palette_filter, Style::default().fg(Color::White))
    };
    let filter_line = Line::from(vec![
        Span::styled("> ", Style::default().fg(Color::Cyan)),
        filter_text,
        Span::styled("█", Style::default().fg(Color::Cyan)), // Cursor
    ]);
    frame.render_widget(Paragraph::new(filter_line), chunks[0]);

    // Command list
    let commands = Command::all();
    let items: Vec<ListItem> = app
        .palette_filtered
        .iter()
        .enumerate()
        .map(|(display_idx, &cmd_idx)| {
            let cmd = &commands[cmd_idx];
            let style = if display_idx == app.palette_index {
                Style::default()
                    .bg(Color::Blue)
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };

            let text = format!(" {}", cmd.display());

            ListItem::new(text).style(style)
        })
        .collect();

    let list = List::new(items);
    frame.render_widget(list, chunks[2]);
}

fn render_history_picker(frame: &mut Frame, app: &App) {
    let area = frame.area();

    // Wide popup near the top, like a fuzzy finder
    let popup_width = 80.min(area.width.saturating_sub(4));
    let popup_height = 20.min(area.height.saturating_sub(4));
    let popup_area = Rect {
        x: (area.width.saturating_sub(popup_width)) / 2,
        y: area.height / 8,
        width: popup_width,
        height: popup_height,
    };

    frame.render_widget(Clear, popup_area);

    let count = app.history_picker_filtered.len();
    let block = Block::default()
        .title(format!(
            " Query History ({}) — enter: run · ^o: to editor · ^a: append ",
            count
        ))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    let inner = block.inner(popup_area);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // Filter input
            Constraint::Length(1), // Separator
            Constraint::Min(1),    // History list
        ])
        .split(inner);

    frame.render_widget(block, popup_area);

    // Filter input
    let filter_text = if app.history_picker_filter.is_empty() {
        Span::styled("Type to filter…", Style::default().fg(Color::DarkGray))
    } else {
        Span::styled(
            &app.history_picker_filter,
            Style::default().fg(Color::White),
        )
    };
    let filter_line = Line::from(vec![
        Span::styled("> ", Style::default().fg(Color::Cyan)),
        filter_text,
        Span::styled("█", Style::default().fg(Color::Cyan)),
    ]);
    frame.render_widget(Paragraph::new(filter_line), chunks[0]);

    // History list (each query collapsed to a single line)
    let entries = app.history_entries();
    let list_height = chunks[2].height as usize;
    // Keep the selected row in view
    let scroll = app
        .history_picker_index
        .saturating_sub(list_height.saturating_sub(1));

    let items: Vec<ListItem> = if entries.is_empty() {
        vec![ListItem::new(Span::styled(
            "  No history for this connection yet",
            Style::default().fg(Color::DarkGray),
        ))]
    } else {
        app.history_picker_filtered
            .iter()
            .enumerate()
            .skip(scroll)
            .take(list_height)
            .map(|(display_idx, &entry_idx)| {
                let raw = entries.get(entry_idx).map(|s| s.as_str()).unwrap_or("");
                // Collapse whitespace/newlines into a single line preview
                let preview: String = raw.split_whitespace().collect::<Vec<_>>().join(" ");
                let style = if display_idx == app.history_picker_index {
                    Style::default()
                        .bg(Color::Blue)
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::White)
                };
                ListItem::new(format!(" {}", preview)).style(style)
            })
            .collect()
    };

    let list = List::new(items);
    frame.render_widget(list, chunks[2]);
}
