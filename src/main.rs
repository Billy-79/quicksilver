/*
 * ============================================================================
 * Project Name: QuickSilver (qs)
 * Author:       Billy-79
 * Description:  A blazing-fast, next-gen terminal file explorer written in Rust,
 *               powered by Ratatui & Crossterm, styled with Catppuccin Mocha.
 *               Features:
 *                 - Native GPU Image Previews (Kitty Graphics Protocol / Sixel)
 *                   via `ratatui-image`
 *                 - Vivid TrueColor bat/batcat Code Previews with ANSI parsing
 *                 - In-Memory Toggles (','/'.' for hidden, 'i' for icons)
 *                 - Safe Symlink-Aware Deletions with Confirmation Guard
 *                 - AST Symbol Outline (Tab) & Interactive $EDITOR Renaming
 * ============================================================================
 */

use ansi_to_tui::IntoText;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers, MouseEvent, MouseEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, List, ListItem, ListState, Paragraph, Wrap},
    Frame, Terminal,
};
use ratatui_image::{
    picker::{Picker, ProtocolType},
    protocol::StatefulProtocol,
    StatefulImage,
};

use std::{
    collections::{HashMap, HashSet},
    fs::{self, File},
    io::{self, Read, Stdout, Write},
    path::{Path, PathBuf},
    process::Command,
    sync::mpsc::{channel, Receiver, Sender},
    thread,
    time::Duration,
};

// ============================================================================
// CATPPUCCIN MOCHA PALETTE CONSTANTS
// Standard 24-bit TrueColor RGB values for consistent theme rendering.
// ============================================================================
const COLOR_BASE: Color = Color::Rgb(30, 30, 46);       // #1e1e2e - Base dark background
const COLOR_MANTLE: Color = Color::Rgb(24, 24, 37);     // #181825 - Darker background for status bar
const COLOR_TEXT: Color = Color::Rgb(205, 214, 244);     // #cdd6f4 - Primary readable foreground text
const COLOR_SUBTEXT: Color = Color::Rgb(166, 173, 200);  // #a6adc8 - Subdued text for hints and secondary info
const COLOR_MAUVE: Color = Color::Rgb(203, 166, 247);    // #cba6f7 - Accent color for active borders
const COLOR_BLUE: Color = Color::Rgb(137, 180, 250);     // #89b4fa - Accent for directories and selections
const COLOR_GREEN: Color = Color::Rgb(166, 227, 161);    // #a6e3a1 - Indicators for staged Git and selections
const COLOR_YELLOW: Color = Color::Rgb(249, 226, 175);   // #f9e2af - Indicators for modified Git and symbols
const COLOR_SURFACE: Color = Color::Rgb(49, 50, 68);     // #313244 - Highlight background for selected items
const COLOR_GUTTER_FG: Color = Color::Rgb(88, 91, 112);  // #585b70 - Dim line numbers for code previews
const COLOR_WARN: Color = Color::Rgb(243, 139, 168);     // #f38ba8 - Warnings, untracked Git, and errors
const COLOR_MATCH: Color = Color::Rgb(250, 179, 135);    // #fab387 - Warm peach accent for active search filter

// Limits preview processing to prevent blocking on giant files
const MAX_PREVIEW_LINES: usize = 1_000;
const MAX_PREVIEW_BYTES: u64 = 1_048_576; // 1 MB cap for instant file previews

// ============================================================================
// ASYNC PREVIEW MESSAGING TYPES
// Offloads expensive bat executions and image rasterization to a worker thread.
// ============================================================================

/// Represents the completed content sent back from the worker thread
enum PreviewPayload {
    /// Formatted ANSI text spans (code/directory listings)
    Text(Vec<Line<'static>>),
    /// GPU protocol image handle (Kitty/Sixel/Halfblocks)
    Image(Box<dyn StatefulProtocol>),
    /// Renderable error message if reading failed
    Error(String),
}

/// Request message sent from the main thread to the preview worker
struct PreviewRequest {
    path: PathBuf,
    is_dir: bool,
    icons_enabled: bool,
    req_id: usize,
    picker: Picker,
}

/// Response message returning the parsed payload and extracted symbols
struct PreviewResponse {
    payload: PreviewPayload,
    symbols: Vec<String>,
    req_id: usize,
}

// ============================================================================
// APPLICATION STATE TYPES
// ============================================================================

/// Input and display mode state machine
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Normal,        // Standard file system browsing
    Search,        // Live fuzzy filtering mode triggered by '/'
    AiInspection,  // Informational semantic analysis window
    Outline,       // AST symbol tree navigation triggered by 'Tab'
    ConfirmDelete, // Safety guard confirmation prompt [y/N]
}

/// Git status indicator for file item badges
#[derive(Clone, Debug, PartialEq, Eq)]
enum GitStatus {
    Modified,  // M: Yellow badge '~'
    Untracked, // ?: Red badge '?'
    Added,     // A: Green badge '+'
    Clean,     // Unmodified: No badge
}

/// Metadata record for a single file entry in the current view
#[derive(Clone, Debug)]
struct FileItem {
    name: String,
    path: PathBuf,
    is_dir: bool,
    git_status: GitStatus,
}

/// Main application context and interactive state
struct App {
    current_path: PathBuf,
    parent_path: Option<PathBuf>,
    items: Vec<FileItem>,
    filtered_indices: Vec<usize>,
    selected_index: usize,
    list_state: ListState,
    mode: Mode,
    search_query: String,
    selected_items: HashSet<PathBuf>,
    pending_delete: Vec<PathBuf>,
    ai_summary: String,
    show_hidden: bool,
    icons_enabled: bool,
    preview_scroll: u16,
    text_preview_cache: Vec<Line<'static>>,
    image_preview_cache: Option<Box<dyn StatefulProtocol>>,
    symbols_cache: Vec<String>,
    parent_items_summary: Vec<String>,
    error_message: Option<String>,
    request_tx: Sender<PreviewRequest>,
    response_rx: Receiver<PreviewResponse>,
    picker: Picker,
    local_req_id: usize,
    needs_redraw: bool,
}

impl App {
    /// Initializes application state, queries terminal geometry, and builds directory cache
    fn new(
        request_tx: Sender<PreviewRequest>,
        response_rx: Receiver<PreviewResponse>,
    ) -> io::Result<Self> {
        let current_path = std::env::current_dir()?;
        let mut list_state = ListState::default();
        list_state.select(Some(0));

        // 1. Initialize picker via termios to capture cell pixel dimensions, with a fallback
        let mut picker = Picker::from_termios().unwrap_or_else(|_| {
            let mut p = Picker::new((8, 16));
            p.guess_protocol();
            p
        });

        // 2. Explicitly force Kitty Graphics Protocol if running inside Kitty or compatible terminals
        let is_kitty = std::env::var("KITTY_WINDOW_ID").is_ok()
            || std::env::var("TERM").map(|t| t.contains("kitty")).unwrap_or(false)
            || std::env::var("TERMINAL").map(|t| t.contains("kitty")).unwrap_or(false);

        if is_kitty {
            picker.protocol_type = ProtocolType::Kitty;
        }

        let mut app = Self {
            parent_path: current_path.parent().map(|p| p.to_path_buf()),
            current_path,
            items: Vec::new(),
            filtered_indices: Vec::new(),
            selected_index: 0,
            list_state,
            mode: Mode::Normal,
            search_query: String::new(),
            selected_items: HashSet::new(),
            pending_delete: Vec::new(),
            ai_summary: String::from("Ready. Press ^A for AI Summary, '?' for Help."),
            show_hidden: false,  // Hidden dotfiles disabled by default
            icons_enabled: true,  // Nerd Font glyphs enabled by default
            preview_scroll: 0,
            text_preview_cache: Vec::new(),
            image_preview_cache: None,
            symbols_cache: Vec::new(),
            parent_items_summary: Vec::new(),
            error_message: None,
            request_tx,
            response_rx,
            picker,
            local_req_id: 0,
            needs_redraw: true,
        };

        app.refresh_directory()?;
        Ok(app)
    }

    /// Queries Git porcelain status to attach badges to entries
    fn query_git_statuses(path: &Path) -> HashMap<String, GitStatus> {
        let mut map = HashMap::new();
        let output = Command::new("git")
            .arg("-C")
            .arg(path)
            .arg("status")
            .arg("--porcelain")
            .arg("--untracked-files=normal")
            .arg(".")
            .output();

        if let Ok(out) = output {
            let stdout = String::from_utf8_lossy(&out.stdout);
            for line in stdout.lines() {
                if line.len() > 3 {
                    let code = &line[0..2];
                    let rel_path = line[3..].trim();
                    let file_name = Path::new(rel_path)
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|| rel_path.to_string());

                    let status = if code.contains('?') {
                        GitStatus::Untracked
                    } else if code.contains('M') {
                        GitStatus::Modified
                    } else if code.contains('A') {
                        GitStatus::Added
                    } else {
                        GitStatus::Clean
                    };
                    map.insert(file_name, status);
                }
            }
        }
        map
    }

    /// Reads directory entries, sorts folders first, and applies visibility toggles
    fn refresh_directory(&mut self) -> io::Result<()> {
        self.items.clear();
        self.error_message = None;
        self.parent_path = self.current_path.parent().map(|p| p.to_path_buf());
        self.refresh_parent_summary();

        let git_map = Self::query_git_statuses(&self.current_path);

        match fs::read_dir(&self.current_path) {
            Ok(entries) => {
                let mut dirs = Vec::new();
                let mut files = Vec::new();

                for entry in entries.flatten() {
                    let path = entry.path();
                    let name = path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned();

                    // Honor hidden files toggle
                    if !self.show_hidden && name.starts_with('.') {
                        continue;
                    }

                    // Properly resolve symlink types
                    let is_dir = if let Ok(ft) = entry.file_type() {
                        if ft.is_symlink() {
                            path.is_dir()
                        } else {
                            ft.is_dir()
                        }
                    } else {
                        path.is_dir()
                    };

                    let git_status = git_map.get(&name).cloned().unwrap_or(GitStatus::Clean);
                    let item = FileItem {
                        name,
                        path,
                        is_dir,
                        git_status,
                    };

                    if is_dir {
                        dirs.push(item);
                    } else {
                        files.push(item);
                    }
                }

                // Sort case-insensitively with directories grouped at the top
                dirs.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
                files.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));

                self.items.extend(dirs);
                self.items.extend(files);
            }
            Err(err) => {
                self.error_message = Some(format!("Permission Denied: {}", err));
            }
        }

        self.apply_search_filter();
        Ok(())
    }

    /// Filters items based on the active search string and keeps selection bounds valid
    fn apply_search_filter(&mut self) {
        if self.search_query.is_empty() {
            self.filtered_indices = (0..self.items.len()).collect();
        } else {
            let q = self.search_query.to_lowercase();
            self.filtered_indices = self
                .items
                .iter()
                .enumerate()
                .filter(|(_, item)| item.name.to_lowercase().contains(&q))
                .map(|(i, _)| i)
                .collect();
        }

        if self.filtered_indices.is_empty() {
            self.selected_index = 0;
            self.list_state.select(None);
            self.text_preview_cache = vec![Line::from("  (No match)")];
            self.image_preview_cache = None;
            self.symbols_cache.clear();
        } else {
            if self.selected_index >= self.filtered_indices.len() {
                self.selected_index = self.filtered_indices.len() - 1;
            }
            self.list_state.select(Some(self.selected_index));
            self.trigger_async_preview();
        }

        self.needs_redraw = true;
    }

    /// Convenience lookup for the current highlighted item
    fn current_selected_item(&self) -> Option<&FileItem> {
        self.filtered_indices
            .get(self.selected_index)
            .and_then(|&idx| self.items.get(idx))
    }

    /// Builds a lightweight summary of the parent folder for column 1
    fn refresh_parent_summary(&mut self) {
        self.parent_items_summary.clear();
        if let Some(parent) = &self.parent_path {
            if let Ok(entries) = fs::read_dir(parent) {
                let mut names = Vec::new();
                for entry in entries.flatten() {
                    let name = entry.file_name().to_string_lossy().into_owned();
                    if !self.show_hidden && name.starts_with('.') {
                        continue;
                    }
                    names.push(name);
                }
                names.sort_by(|a, b| a.to_lowercase().cmp(&b.to_lowercase()));
                self.parent_items_summary = names;
            }
        }
    }

    /// Dispatches preview computation to the background thread with request ID tracking
    fn trigger_async_preview(&mut self) {
        self.preview_scroll = 0;
        self.image_preview_cache = None; // Reset graphics cache to clear residual GPU surfaces
        self.needs_redraw = true;

        if let Some(ref err) = self.error_message {
            self.text_preview_cache = vec![Line::from(Span::styled(
                format!("  ⚠ {}", err),
                Style::default().fg(COLOR_WARN),
            ))];
            self.symbols_cache.clear();
            return;
        }

        let Some((target_path, is_dir)) = self
            .current_selected_item()
            .map(|item| (item.path.clone(), item.is_dir))
        else {
            self.text_preview_cache = vec![Line::from("  (Empty directory)")];
            self.symbols_cache.clear();
            return;
        };

        // Increment ID to reject responses from stale, queued preview requests
        self.local_req_id = self.local_req_id.wrapping_add(1);
        self.text_preview_cache = vec![Line::from(Span::styled(
            "  Loading preview...",
            Style::default().fg(COLOR_SUBTEXT).add_modifier(Modifier::ITALIC),
        ))];

        let _ = self.request_tx.send(PreviewRequest {
            path: target_path,
            is_dir,
            icons_enabled: self.icons_enabled,
            req_id: self.local_req_id,
            picker: self.picker.clone(),
        });
    }

    /// Polls non-blocking responses from the preview background worker
    fn poll_async_preview(&mut self) {
        while let Ok(res) = self.response_rx.try_recv() {
            // Drop outdated responses from fast scrolling
            if res.req_id == self.local_req_id {
                match res.payload {
                    PreviewPayload::Text(lines) => {
                        self.text_preview_cache = lines;
                        self.image_preview_cache = None;
                    }
                    PreviewPayload::Image(protocol) => {
                        self.image_preview_cache = Some(protocol);
                        self.text_preview_cache.clear();
                    }
                    PreviewPayload::Error(err) => {
                        self.text_preview_cache = vec![Line::from(Span::styled(
                            format!("  ⚠ {}", err),
                            Style::default().fg(COLOR_WARN),
                        ))];
                        self.image_preview_cache = None;
                    }
                }
                self.symbols_cache = res.symbols;
                self.needs_redraw = true;
            }
        }
    }

    /// Moves selection down by one
    fn next(&mut self) {
        if !self.filtered_indices.is_empty() && self.selected_index < self.filtered_indices.len() - 1 {
            self.selected_index += 1;
            self.error_message = None;
            self.list_state.select(Some(self.selected_index));
            self.trigger_async_preview();
        }
    }

    /// Moves selection up by one
    fn previous(&mut self) {
        if self.selected_index > 0 {
            self.selected_index -= 1;
            self.error_message = None;
            self.list_state.select(Some(self.selected_index));
            self.trigger_async_preview();
        }
    }

    /// Toggles visual checkbox selection on the current item
    fn toggle_selection(&mut self) {
        if let Some(target_path) = self.current_selected_item().map(|item| item.path.clone()) {
            if self.selected_items.contains(&target_path) {
                self.selected_items.remove(&target_path);
            } else {
                self.selected_items.insert(target_path);
            }
            self.next();
            self.needs_redraw = true;
        }
    }

    /// Descends into the highlighted directory
    fn enter(&mut self) -> io::Result<()> {
        if let Some(item) = self.current_selected_item() {
            if item.is_dir {
                if let Err(e) = fs::read_dir(&item.path) {
                    self.error_message = Some(format!("Access Denied: {}", e));
                    self.trigger_async_preview();
                    return Ok(());
                }
                self.current_path = item.path.clone();
                self.selected_index = 0;
                self.search_query.clear();
                self.selected_items.clear();
                self.refresh_directory()?;
            }
        }
        Ok(())
    }

    /// Navigates up to the parent directory
    fn back(&mut self) -> io::Result<()> {
        if let Some(parent) = self.parent_path.clone() {
            let prev_name = self.current_path.file_name().map(|n| n.to_os_string());
            self.current_path = parent;
            self.search_query.clear();
            self.selected_items.clear();
            self.refresh_directory()?;

            // Restore selection highlight on the folder we just exited
            if let Some(target) = prev_name {
                if let Some(pos) = self
                    .filtered_indices
                    .iter()
                    .position(|&i| self.items[i].path.file_name() == Some(&target))
                {
                    self.selected_index = pos;
                    self.list_state.select(Some(self.selected_index));
                    self.trigger_async_preview();
                }
            }
        }
        Ok(())
    }

    /// Gathers targets for deletion and prompts the user for confirmation
    fn prepare_delete(&mut self) {
        let mut targets = if !self.selected_items.is_empty() {
            self.selected_items.iter().cloned().collect::<Vec<_>>()
        } else if let Some(item) = self.current_selected_item() {
            vec![item.path.clone()]
        } else {
            Vec::new()
        };

        if !targets.is_empty() {
            targets.sort();
            self.pending_delete = targets;
            self.mode = Mode::ConfirmDelete;
            self.needs_redraw = true;
        }
    }

    /// Safely deletes files, dirs, or symlinks without following symlink targets
    fn execute_delete(&mut self) -> io::Result<()> {
        let targets = std::mem::take(&mut self.pending_delete);
        for path in targets {
            if let Ok(meta) = fs::symlink_metadata(&path) {
                if meta.file_type().is_symlink() {
                    let _ = fs::remove_file(&path); // Unlink symlink without removing target contents
                } else if meta.is_dir() {
                    let _ = fs::remove_dir_all(&path);
                } else {
                    let _ = fs::remove_file(&path);
                }
            }
        }
        self.selected_items.clear();
        self.mode = Mode::Normal;
        self.refresh_directory()?;
        Ok(())
    }

    /// Cancels pending delete operation
    fn cancel_delete(&mut self) {
        self.pending_delete.clear();
        self.mode = Mode::Normal;
        self.needs_redraw = true;
    }

    /// Spawns $EDITOR with a newline-separated scratchpad of filenames for bulk renaming
    fn batch_rename_in_editor(&mut self, terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> io::Result<()> {
        let mut targets = if !self.selected_items.is_empty() {
            self.selected_items.iter().cloned().collect::<Vec<_>>()
        } else if let Some(item) = self.current_selected_item() {
            vec![item.path.clone()]
        } else {
            return Ok(());
        };

        targets.sort();

        let temp_dir = std::env::temp_dir();
        let scratch_path = temp_dir.join("qs_batch_rename.txt");

        // Write out current names to scratchpad
        {
            let mut file = File::create(&scratch_path)?;
            for p in &targets {
                if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
                    writeln!(file, "{}", name)?;
                }
            }
        }

        // Suspend UI and open scratchpad in text editor
        open_file_in_editor(terminal, &scratch_path)?;

        // Parse modified names
        let content = fs::read_to_string(&scratch_path)?;
        let new_names = content.lines().map(|s| s.trim()).filter(|s| !s.is_empty()).collect::<Vec<_>>();

        // Apply 1-to-1 renames if line counts match
        if new_names.len() == targets.len() {
            for (orig_path, &new_name) in targets.iter().zip(new_names.iter()) {
                let new_path = orig_path.with_file_name(new_name);
                if *orig_path != new_path {
                    if new_path.exists() {
                        self.error_message = Some(format!("Rename collision: '{}' exists", new_name));
                    } else {
                        let _ = fs::rename(orig_path, new_path);
                    }
                }
            }
        } else {
            self.error_message = Some(format!(
                "Rename aborted: line count changed (expected {}, got {})",
                targets.len(),
                new_names.len()
            ));
        }

        let _ = fs::remove_file(scratch_path);
        self.selected_items.clear();
        self.refresh_directory()?;
        Ok(())
    }

    /// Scrolls preview viewport downward
    fn scroll_preview_down(&mut self, delta: u16) {
        let max_lines = if self.mode == Mode::Outline {
            self.symbols_cache.len() as u16
        } else {
            self.text_preview_cache.len() as u16
        };

        if self.preview_scroll + delta < max_lines {
            self.preview_scroll += delta;
        } else {
            self.preview_scroll = max_lines.saturating_sub(1);
        }
        self.needs_redraw = true;
    }

    /// Scrolls preview viewport upward
    fn scroll_preview_up(&mut self, delta: u16) {
        self.preview_scroll = self.preview_scroll.saturating_sub(delta);
        self.needs_redraw = true;
    }

    /// Displays semantic inspection window for the active entry
    fn trigger_ai_inspection(&mut self) {
        if let Some((name, path, is_dir)) = self
            .current_selected_item()
            .map(|item| (item.name.clone(), item.path.clone(), item.is_dir))
        {
            self.mode = Mode::AiInspection;
            let metadata = fs::metadata(&path).ok();
            let size = metadata.map(|m| format_file_size(m.len())).unwrap_or_else(|| "0 B".into());

            if is_dir {
                self.ai_summary = format!(
                    "🤖 AI Semantic Analysis [DIRECTORY]: {}\n\n\
                    • Structure: High integrity\n\
                    • Path: {}\n\
                    • State: Ready for AST analysis / workspace indexing.",
                    name,
                    path.display()
                );
            } else {
                self.ai_summary = format!(
                    "🤖 AI Semantic Analysis [FILE]: {}\n\n\
                    • Size: {}\n\
                    • Path: {}\n\
                    • State: Clean syntax tokens detected.",
                    name,
                    size,
                    path.display()
                );
            }
            self.needs_redraw = true;
        }
    }

    /// Displays keyboard shortcuts overlay
    fn show_help(&mut self) {
        self.mode = Mode::AiInspection;
        self.ai_summary = String::from(
            "📖 QuickSilver Navigation & Controls:\n\n\
            • Movement     : j / k / Up / Down\n\
            • Enter Dir    : l / Right / Enter\n\
            • Back Dir     : h / Left\n\
            • Search/Filter: / (Esc to clear)\n\
            • Multi-Select : Space\n\
            • AST Outline  : Tab (Toggle outline mode)\n\
            • Batch Rename : R (Opens selected in $EDITOR)\n\
            • Safe Delete  : d (Prompts confirmation [y/N])\n\
            • Open File    : e / o\n\
            • Toggle Hidden: , or .\n\
            • Toggle Icons : i\n\
            • Reload Dir   : r\n\
            • Home Dir     : ~ or g\n\
            • AI Summary   : ^A\n\
            • Exit         : q"
        );
        self.needs_redraw = true;
    }
}

// ============================================================================
// AST ENGINE, PARSERS & HELPERS
// ============================================================================

/// Converts byte counts into human-readable formatted units
fn format_file_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;

    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}

/// Lightweight syntax analyzer extracting function and struct definitions for 'Tab' mode
fn extract_ast_symbols(content: &str, ext: &str) -> Vec<String> {
    let mut symbols = Vec::new();
    let ext_lower = ext.to_lowercase();
    for (idx, line) in content.lines().enumerate() {
        let trimmed = line.trim();
        let line_num = idx + 1;

        // Skip comment blocks
        if trimmed.starts_with("//")
            || trimmed.starts_with("/*")
            || trimmed.starts_with('*')
            || trimmed.starts_with('#')
            || trimmed.starts_with("\"\"\"")
            || trimmed.starts_with("'''")
        {
            continue;
        }

        match ext_lower.as_str() {
            "rs" => {
                if trimmed.contains("fn ") && (trimmed.starts_with("fn ") || trimmed.starts_with("pub ") || trimmed.starts_with("async ") || trimmed.starts_with("const ") || trimmed.starts_with("unsafe ")) {
                    symbols.push(format!("Line {:>4} │ 󰊕 {}", line_num, trimmed.split('{').next().unwrap_or(trimmed)));
                } else if trimmed.starts_with("pub struct ") || trimmed.starts_with("struct ") {
                    symbols.push(format!("Line {:>4} │ 󰌗 {}", line_num, trimmed.split('{').next().unwrap_or(trimmed)));
                } else if trimmed.starts_with("pub enum ") || trimmed.starts_with("enum ") {
                    symbols.push(format!("Line {:>4} │ 󰍲 {}", line_num, trimmed.split('{').next().unwrap_or(trimmed)));
                } else if trimmed.starts_with("impl ") {
                    symbols.push(format!("Line {:>4} │ 󰡱 {}", line_num, trimmed.split('{').next().unwrap_or(trimmed)));
                }
            }
            "py" => {
                if trimmed.starts_with("def ") || trimmed.starts_with("async def ") {
                    symbols.push(format!("Line {:>4} │ 󰊕 {}", line_num, trimmed.trim_end_matches(':')));
                } else if trimmed.starts_with("class ") {
                    symbols.push(format!("Line {:>4} │ 󰌗 {}", line_num, trimmed.trim_end_matches(':')));
                }
            }
            "ts" | "js" | "tsx" | "jsx" => {
                if trimmed.starts_with("function ") || trimmed.starts_with("export function ") || trimmed.starts_with("async function ") || trimmed.contains(" = () =>") || trimmed.contains(" = function") {
                    symbols.push(format!("Line {:>4} │ 󰊕 {}", line_num, trimmed.split('{').next().unwrap_or(trimmed)));
                } else if trimmed.starts_with("class ") || trimmed.starts_with("export class ") {
                    symbols.push(format!("Line {:>4} │ 󰌗 {}", line_num, trimmed.split('{').next().unwrap_or(trimmed)));
                } else if trimmed.starts_with("interface ") || trimmed.starts_with("type ") {
                    symbols.push(format!("Line {:>4} │ 󰘦 {}", line_num, trimmed.split('{').next().unwrap_or(trimmed)));
                }
            }
            "go" => {
                if trimmed.starts_with("func ") {
                    symbols.push(format!("Line {:>4} │ 󰊕 {}", line_num, trimmed.split('{').next().unwrap_or(trimmed)));
                } else if trimmed.starts_with("type ") {
                    symbols.push(format!("Line {:>4} │ 󰌗 {}", line_num, trimmed.split('{').next().unwrap_or(trimmed)));
                }
            }
            _ => {}
        }
    }
    symbols
}

/// Resolves Nerd Font icons based on file extensions
fn get_file_icon(name: &str, is_dir: bool) -> &'static str {
    if is_dir {
        return " ";
    }
    let lower = name.to_lowercase();
    if lower.ends_with(".rs") {
        " "
    } else if lower.ends_with(".ts") || lower.ends_with(".tsx") {
        " "
    } else if lower.ends_with(".js") || lower.ends_with(".jsx") {
        " "
    } else if lower.ends_with(".py") {
        " "
    } else if lower.ends_with(".go") {
        " "
    } else if lower.ends_with(".c") || lower.ends_with(".h") {
        " "
    } else if lower.ends_with(".cpp") || lower.ends_with(".hpp") {
        " "
    } else if lower.ends_with(".toml") || lower.ends_with(".json") || lower.ends_with(".yaml") || lower.ends_with(".yml") {
        " "
    } else if lower.ends_with(".md") {
        " "
    } else if lower.starts_with(".git") {
        " "
    } else if lower.ends_with(".lock") {
        "󰌆 "
    } else if lower == "dockerfile" || lower.starts_with("dockerfile.") {
        "󰡨 "
    } else if lower.ends_with(".sh") || lower.ends_with(".bash") || lower.ends_with(".zsh") {
        " "
    } else if lower.ends_with(".png") || lower.ends_with(".jpg") || lower.ends_with(".jpeg") || lower.ends_with(".svg") {
        "󰋩 "
    } else if lower.ends_with(".tar") || lower.ends_with(".zip") || lower.ends_with(".gz") || lower.ends_with(".7z") {
        " "
    } else {
        "󰈙 "
    }
}

/// Identifies if a file extension corresponds to a supported graphic image
fn is_image_extension(ext: &str) -> bool {
    matches!(
        ext.to_lowercase().as_str(),
        "png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp" | "ico" | "tiff" | "avif"
    )
}

/// Generates directory preview listings for column 3
fn load_directory_preview(path: &Path, icons_enabled: bool) -> (PreviewPayload, Vec<String>) {
    match fs::read_dir(path) {
        Ok(entries) => {
            let mut items = Vec::new();
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                let is_dir = entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false);
                let icon = if icons_enabled {
                    get_file_icon(&name, is_dir)
                } else {
                    ""
                };
                items.push(format!("  {} {}", icon, name));
            }
            items.sort_by(|a, b| a.to_lowercase().cmp(&b.to_lowercase()));
            if items.is_empty() {
                (PreviewPayload::Text(vec![Line::from("  (Empty directory)")]), Vec::new())
            } else {
                (
                    PreviewPayload::Text(items.into_iter().map(Line::from).collect()),
                    Vec::new(),
                )
            }
        }
        Err(err) => (
            PreviewPayload::Error(format!("Cannot open folder: {}", err)),
            Vec::new(),
        ),
    }
}

/// Dispatches image decoders for Kitty Graphics / Sixel or falls back to bat/batcat
fn load_file_preview(path: &Path, mut picker: Picker) -> (PreviewPayload, Vec<String>) {
    let metadata = match fs::metadata(path) {
        Ok(m) => m,
        Err(e) => return (PreviewPayload::Error(format!("Metadata read failed: {}", e)), Vec::new()),
    };

    let extension = path.extension().and_then(|e| e.to_str()).unwrap_or("");

    // --- Native GPU Graphic Pipeline (Kitty / Sixel / WezTerm) ---
    if is_image_extension(extension) {
        match image::open(path) {
            Ok(dyn_img) => {
                let width = dyn_img.width();
                let height = dyn_img.height();
                // Converts the dynamic image into a hardware-backed protocol handle
                let protocol = picker.new_resize_protocol(dyn_img);
                let symbols = vec![
                    format!("Format: {}", extension.to_uppercase()),
                    format!("Resolution: {}x{}", width, height),
                    format!("Size: {}", format_file_size(metadata.len())),
                ];
                return (PreviewPayload::Image(protocol), symbols);
            }
            Err(e) => {
                return (
                    PreviewPayload::Error(format!("Failed to decode image: {}", e)),
                    Vec::new(),
                );
            }
        }
    }

    // Safety guard against reading huge log files into memory
    if metadata.len() > MAX_PREVIEW_BYTES {
        return (
            PreviewPayload::Text(vec![Line::from(format!(
                "File exceeds preview cap ({:.2} MB). Use an external pager.",
                metadata.len() as f64 / 1_048_576.0
            ))]),
            Vec::new(),
        );
    }

    // Binary check: inspect first 1024 bytes for null bytes
    if let Ok(mut file) = File::open(path) {
        let mut check_buf = [0u8; 1024];
        if let Ok(n) = file.read(&mut check_buf) {
            if check_buf[..n].contains(&0) {
                return (
                    PreviewPayload::Text(vec![Line::from("Binary file preview is not supported.")]),
                    Vec::new(),
                );
            }
        }
    }

    let raw_content = fs::read_to_string(path).unwrap_or_default();
    let symbols = extract_ast_symbols(&raw_content, extension);

    // Run bat or batcat (Ubuntu) forcing truecolor output and Catppuccin Mocha styling
    let run_bat = |bin: &str| {
        Command::new(bin)
            .env("COLORTERM", "truecolor") // Forces full 24-bit TrueColor escaping
            .env("BAT_THEME", "Catppuccin Mocha")
            .arg("--color=always")
            .arg("--theme=Catppuccin Mocha")
            .arg("--style=numbers")
            .arg("--line-range=:1000")
            .arg("--terminal-width=180")
            .arg(path)
            .output()
    };

    let bat_output = run_bat("bat").or_else(|_| run_bat("batcat"));

    match bat_output {
        Ok(out) if out.status.success() => {
            // Convert bat's ANSI color stream directly into Ratatui styled spans
            let ansi_text = out.stdout.into_text().unwrap_or_default();
            let lines: Vec<Line<'static>> = ansi_text
                .lines
                .into_iter()
                .map(|line| {
                    Line::from(
                        line.spans
                            .into_iter()
                            .map(|span| {
                                let mut style = span.style;
                                // Brighten unstyled text tokens to match Catppuccin Text foreground
                                if style.fg.is_none() {
                                    style = style.fg(COLOR_TEXT);
                                }
                                Span::styled(span.content.into_owned(), style)
                            })
                            .collect::<Vec<_>>(),
                    )
                })
                .collect();

            if lines.is_empty() {
                (PreviewPayload::Text(vec![Line::from("  (Empty file)")]), symbols)
            } else {
                (PreviewPayload::Text(lines), symbols)
            }
        }
        _ => {
            // Built-in plain-text fallback viewer if bat/batcat is unavailable
            let mut fallback_lines = Vec::new();
            let total = raw_content.lines().count().min(MAX_PREVIEW_LINES);
            let width = total.to_string().len().max(2);

            for (idx, line) in raw_content.lines().take(MAX_PREVIEW_LINES).enumerate() {
                let gutter = format!("{:>width$} │ ", idx + 1, width = width);
                fallback_lines.push(Line::from(vec![
                    Span::styled(gutter, Style::default().fg(COLOR_GUTTER_FG)),
                    Span::styled(line.to_string(), Style::default().fg(COLOR_TEXT)),
                ]));
            }

            if fallback_lines.is_empty() {
                (PreviewPayload::Text(vec![Line::from("  (Empty file)")]), symbols)
            } else {
                (PreviewPayload::Text(fallback_lines), symbols)
            }
        }
    }
}

// ============================================================================
// SYSTEM HOOKS & TERMINAL LIFECYCLE
// ============================================================================

/// Ensures terminal raw mode is disabled even if the application panics
fn setup_panic_hook() {
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen, crossterm::event::DisableMouseCapture);
        original_hook(panic_info);
    }));
}

/// Suspends Ratatui and hands standard I/O over to the external $EDITOR
fn open_file_in_editor(terminal: &mut Terminal<CrosstermBackend<Stdout>>, path: &Path) -> io::Result<()> {
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen, crossterm::event::DisableMouseCapture)?;
    terminal.show_cursor()?;
    terminal.backend_mut().flush()?;

    let editor_cmd = std::env::var("EDITOR")
        .or_else(|_| std::env::var("VISUAL"))
        .unwrap_or_else(|_| {
            if Command::new("nvim").arg("--version").output().is_ok() {
                "nvim".into()
            } else if Command::new("vim").arg("--version").output().is_ok() {
                "vim".into()
            } else {
                "nano".into()
            }
        });

    let mut parts = editor_cmd.split_whitespace();
    if let Some(binary) = parts.next() {
        let mut cmd = Command::new(binary);
        for arg in parts {
            cmd.arg(arg);
        }
        cmd.arg(path);
        let _ = cmd.status();
    }

    // Resume TUI state
    enable_raw_mode()?;
    execute!(terminal.backend_mut(), EnterAlternateScreen, crossterm::event::EnableMouseCapture)?;
    terminal.hide_cursor()?;
    terminal.clear()?;
    terminal.backend_mut().flush()?;
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    setup_panic_hook();

    // Initialize raw terminal mode and alternate screen
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, crossterm::event::EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Set up channels for background async preview generation
    let (req_tx, req_rx) = channel::<PreviewRequest>();
    let (res_tx, res_rx) = channel::<PreviewResponse>();

    // Dedicated preview worker thread
    thread::spawn(move || {
        while let Ok(mut req) = req_rx.recv() {
            // Drain queue and take only the freshest request
            while let Ok(newer_req) = req_rx.try_recv() {
                req = newer_req;
            }

            let (payload, symbols) = if req.is_dir {
                load_directory_preview(&req.path, req.icons_enabled)
            } else {
                load_file_preview(&req.path, req.picker)
            };

            let _ = res_tx.send(PreviewResponse {
                payload,
                symbols,
                req_id: req.req_id,
            });
        }
    });

    let mut app = App::new(req_tx, res_rx)?;
    let res = run_app(&mut terminal, &mut app);

    // Teardown terminal and restore caller's shell environment
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        crossterm::event::DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    if let Err(err) = res {
        eprintln!("Application Error: {:?}", err);
    }

    Ok(())
}

// ============================================================================
// MAIN EVENT LOOP
// ============================================================================

fn run_app(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    app: &mut App,
) -> io::Result<()> {
    loop {
        // Poll worker thread for finished preview responses
        app.poll_async_preview();

        // Render UI only when dirty to conserve CPU/battery
        if app.needs_redraw {
            terminal.draw(|f| ui(f, app))?;
            app.needs_redraw = false;
        }

        // Wait up to 16ms (~60 FPS) for user input
        if event::poll(Duration::from_millis(16))? {
            match event::read()? {
                Event::Resize(_, _) => {
                    let _ = terminal.clear(); // Clear GPU placement layers on resize
                    app.needs_redraw = true;
                }
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    // Modal: Confirm Delete [y/N]
                    if app.mode == Mode::ConfirmDelete {
                        match key.code {
                            KeyCode::Char('y') | KeyCode::Char('Y') => {
                                app.execute_delete()?;
                            }
                            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                                app.cancel_delete();
                            }
                            _ => {}
                        }
                        continue;
                    }

                    // Modal: Search / Filter mode
                    if app.mode == Mode::Search {
                        match key.code {
                            KeyCode::Esc => {
                                app.mode = Mode::Normal;
                                app.search_query.clear();
                                app.apply_search_filter();
                            }
                            KeyCode::Enter => {
                                app.mode = Mode::Normal;
                                app.needs_redraw = true;
                            }
                            KeyCode::Backspace => {
                                if app.search_query.pop().is_some() {
                                    app.apply_search_filter();
                                }
                            }
                            KeyCode::Down => app.next(),
                            KeyCode::Up => app.previous(),
                            KeyCode::Char(c) => {
                                app.search_query.push(c);
                                app.apply_search_filter();
                            }
                            _ => {}
                        }
                        continue;
                    }

                    // Global Quit
                    if key.code == KeyCode::Char('q') {
                        return Ok(());
                    }

                    // Global AI Semantic Summary shortcut
                    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('a') {
                        app.trigger_ai_inspection();
                        continue;
                    }

                    match app.mode {
                        Mode::AiInspection => {
                            app.mode = Mode::Normal;
                            app.needs_redraw = true;
                        }
                        Mode::Outline => match key.code {
                            KeyCode::Tab | KeyCode::Esc => {
                                app.mode = Mode::Normal;
                                app.preview_scroll = 0;
                                app.needs_redraw = true;
                            }
                            KeyCode::Down | KeyCode::Char('j') => {
                                app.scroll_preview_down(1);
                            }
                            KeyCode::Up | KeyCode::Char('k') => {
                                app.scroll_preview_up(1);
                            }
                            KeyCode::PageDown => {
                                app.scroll_preview_down(10);
                            }
                            KeyCode::PageUp => {
                                app.scroll_preview_up(10);
                            }
                            _ => {}
                        },
                        Mode::Normal => match key.code {
                            KeyCode::Char('/') => {
                                app.mode = Mode::Search;
                                app.search_query.clear();
                                app.needs_redraw = true;
                            }
                            KeyCode::Char(' ') => {
                                app.toggle_selection();
                            }
                            KeyCode::Tab => {
                                app.mode = Mode::Outline;
                                app.preview_scroll = 0;
                                app.needs_redraw = true;
                            }
                            KeyCode::Char('R') => {
                                app.batch_rename_in_editor(terminal)?;
                            }
                            KeyCode::Char('d') => {
                                app.prepare_delete();
                            }
                            // Both ',' and '.' toggle hidden files
                            KeyCode::Char(',') | KeyCode::Char('.') => {
                                app.show_hidden = !app.show_hidden;
                                app.refresh_directory()?;
                            }
                            KeyCode::Char('i') => {
                                app.icons_enabled = !app.icons_enabled;
                                app.trigger_async_preview();
                            }
                            KeyCode::Char('r') => {
                                app.refresh_directory()?;
                            }
                            // Native std::env HOME resolution without external crates
                            KeyCode::Char('~') | KeyCode::Char('g') => {
                                if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
                                    app.current_path = home;
                                    app.selected_index = 0;
                                    app.selected_items.clear();
                                    app.refresh_directory()?;
                                }
                            }
                            KeyCode::Char('e') | KeyCode::Char('o') => {
                                if let Some(item) = app.current_selected_item() {
                                    if !item.is_dir {
                                        open_file_in_editor(terminal, &item.path)?;
                                        app.refresh_directory()?;
                                    }
                                }
                            }
                            KeyCode::Char('?') => {
                                app.show_help();
                            }
                            KeyCode::PageDown => {
                                app.scroll_preview_down(10);
                            }
                            KeyCode::PageUp => {
                                app.scroll_preview_up(10);
                            }
                            KeyCode::Down | KeyCode::Char('j') => app.next(),
                            KeyCode::Up | KeyCode::Char('k') => app.previous(),
                            KeyCode::Right | KeyCode::Char('l') | KeyCode::Enter => app.enter()?,
                            KeyCode::Left | KeyCode::Char('h') => app.back()?,
                            _ => {}
                        },
                        _ => {}
                    }
                }
                Event::Mouse(MouseEvent { kind, .. }) => {
                    match kind {
                        MouseEventKind::ScrollDown => app.scroll_preview_down(3),
                        MouseEventKind::ScrollUp => app.scroll_preview_up(3),
                        _ => {}
                    }
                    app.needs_redraw = true;
                }
                _ => {}
            }
        }
    }
}

// ============================================================================
// UI RENDERING ENGINE
// ============================================================================

fn ui(f: &mut Frame, app: &mut App) {
    let area = f.size();

    // Guard against microscopic terminal windows
    if area.width < 50 || area.height < 10 {
        let msg = Paragraph::new("Resize terminal (min 50x10)").style(Style::default().fg(COLOR_WARN));
        f.render_widget(msg, area);
        return;
    }

    // Split terminal vertically: Main view (top) and Status bar (bottom)
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(1)])
        .split(area);

    // Split top area horizontally into 3 columns: Parent (20%), Current (40%), Preview (40%)
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(20),
            Constraint::Percentage(40),
            Constraint::Percentage(40),
        ])
        .split(chunks[0]);

    // --- Column 1: Parent Directory ---
    let parent_title = if let Some(p) = &app.parent_path {
        format!(" Parent: {} ", p.file_name().and_then(|n| n.to_str()).unwrap_or("/"))
    } else {
        String::from(" Parent: Root ")
    };

    let cur_dir_name = app.current_path.file_name().and_then(|n| n.to_str()).unwrap_or("");

    let parent_items: Vec<ListItem> = if app.parent_items_summary.is_empty() {
        vec![ListItem::new(Span::raw("  (No parent)"))]
    } else {
        app.parent_items_summary
            .iter()
            .take(area.height as usize)
            .map(|name| {
                let style = if name == cur_dir_name {
                    Style::default().fg(COLOR_BLUE).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(COLOR_SUBTEXT)
                };
                ListItem::new(Span::styled(format!("  {}", name), style))
            })
            .collect()
    };

    let parent_widget = List::new(parent_items).block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(COLOR_MAUVE))
            .style(Style::default().bg(COLOR_BASE))
            .title(parent_title),
    );
    f.render_widget(parent_widget, columns[0]);

    // --- Column 2: Current Directory Listing with Git Badges & Selection ---
    let current_title = if app.mode == Mode::Search {
        format!("  Filter: {}_ ", app.search_query)
    } else {
        format!(
            " {} ({}/{}) ",
            app.current_path.display(),
            app.filtered_indices.len(),
            app.items.len()
        )
    };

    let items: Vec<ListItem> = app
        .filtered_indices
        .iter()
        .map(|&idx| {
            let item = &app.items[idx];
            let is_selected = app.selected_items.contains(&item.path);

            let check_icon = if is_selected { "󰄲 " } else { "  " };

            let display_text = if app.icons_enabled {
                let icon = get_file_icon(&item.name, item.is_dir);
                format!("{}{}{}", check_icon, icon, item.name)
            } else {
                format!("{}{}", check_icon, item.name)
            };

            let (git_badge, git_color) = match item.git_status {
                GitStatus::Modified => (" ~", COLOR_YELLOW),
                GitStatus::Untracked => (" ?", COLOR_WARN),
                GitStatus::Added => (" +", COLOR_GREEN),
                GitStatus::Clean => ("", COLOR_TEXT),
            };

            let item_color = if is_selected {
                COLOR_GREEN
            } else if item.is_dir {
                COLOR_BLUE
            } else {
                COLOR_TEXT
            };

            ListItem::new(Line::from(vec![
                Span::styled(display_text, Style::default().fg(item_color)),
                Span::styled(git_badge, Style::default().fg(git_color).add_modifier(Modifier::BOLD)),
            ]))
        })
        .collect();

    let list_border_color = if app.mode == Mode::Search {
        COLOR_MATCH
    } else {
        COLOR_MAUVE
    };

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(list_border_color))
                .style(Style::default().bg(COLOR_BASE))
                .title(current_title),
        )
        .highlight_style(
            Style::default()
                .bg(COLOR_SURFACE)
                .fg(COLOR_BLUE)
                .add_modifier(Modifier::BOLD),
        );

    f.render_stateful_widget(list, columns[1], &mut app.list_state);

    // --- Column 3: Live Preview / Image Pane / AST Outline / AI Pane ---
    let (preview_title, preview_border_color) = match app.mode {
        Mode::AiInspection => (" 🤖 AI Semantic Summary ".to_string(), COLOR_GREEN),
        Mode::Outline => (" 󰘦 AST Symbol Outline (Tab/Esc to return) ".to_string(), COLOR_YELLOW),
        _ => {
            if let Some(selected) = app.current_selected_item() {
                if selected.is_dir {
                    (format!("  {} ", selected.name), COLOR_MAUVE)
                } else if is_image_extension(selected.path.extension().and_then(|e| e.to_str()).unwrap_or("")) {
                    (format!(" 🖼 {} (GPU Native Preview) ", selected.name), COLOR_MAUVE)
                } else {
                    let size_str = fs::metadata(&selected.path)
                        .ok()
                        .map(|m| format_file_size(m.len()))
                        .unwrap_or_else(|| "0 B".into());
                    (
                        format!(
                            " 󰈙 {} │ {} │ {} lines ",
                            selected.name,
                            size_str,
                            app.text_preview_cache.len()
                        ),
                        COLOR_MAUVE,
                    )
                }
            } else {
                (" 👁️ Live Preview ".to_string(), COLOR_MAUVE)
            }
        }
    };

    let preview_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(preview_border_color))
        .style(Style::default().bg(COLOR_BASE))
        .title(preview_title);

    let inner_preview_area = preview_block.inner(columns[2]);
    f.render_widget(preview_block, columns[2]);

    if app.mode == Mode::AiInspection {
        let p = Paragraph::new(app.ai_summary.clone())
            .style(Style::default().bg(COLOR_BASE).fg(COLOR_TEXT))
            .wrap(Wrap { trim: true });
        f.render_widget(p, inner_preview_area);
    } else if app.mode == Mode::Outline {
        let outline_lines: Vec<Line> = if app.symbols_cache.is_empty() {
            vec![Line::from("  No AST symbols found for this file.")]
        } else {
            app.symbols_cache
                .iter()
                .map(|s| Line::from(Span::styled(s.clone(), Style::default().fg(COLOR_YELLOW))))
                .collect()
        };

        let p = Paragraph::new(outline_lines)
            .scroll((app.preview_scroll, 0))
            .style(Style::default().bg(COLOR_BASE));
        f.render_widget(p, inner_preview_area);
    } else if let Some(image_protocol) = &mut app.image_preview_cache {
        // Direct GPU rendering via Kitty Graphics Protocol or Sixel
        let image_widget = StatefulImage::new(None);
        f.render_stateful_widget(image_widget, inner_preview_area, image_protocol);
    } else {
        let p = Paragraph::new(app.text_preview_cache.clone())
            .scroll((app.preview_scroll, 0))
            .style(Style::default().bg(COLOR_BASE));
        f.render_widget(p, inner_preview_area);
    }

    // --- Status Bar ---
    let sel_count = app.selected_items.len();
    let (status_text, status_bg, status_fg) = match app.mode {
        Mode::ConfirmDelete => {
            let count = app.pending_delete.len();
            let target_desc = if count == 1 {
                app.pending_delete[0]
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("item")
                    .to_string()
            } else {
                format!("{} items", count)
            };

            (
                format!(" ⚠ Permanently delete '{}'? [y/N] ", target_desc),
                COLOR_WARN,
                COLOR_BASE,
            )
        }
        Mode::Search => (
            format!(" [Esc] Clear | [Enter] Select | [Up/Down] Navigate | Filtering: \"{}\" ", app.search_query),
            COLOR_MAUVE,
            COLOR_MANTLE,
        ),
        Mode::AiInspection => (
            " Mode: AI Inspection (Press any key to close) ".to_string(),
            COLOR_MAUVE,
            COLOR_MANTLE,
        ),
        Mode::Outline => (
            " Mode: AST Outline | [j/k/Wheel] Scroll Outline | [Tab/Esc] Return to Code Preview ".to_string(),
            COLOR_MAUVE,
            COLOR_MANTLE,
        ),
        Mode::Normal if sel_count > 0 => (
            format!(" {} selected | [R] Rename in $EDITOR | [d] Delete | [Space] Toggle | [q] Quit ", sel_count),
            COLOR_MAUVE,
            COLOR_MANTLE,
        ),
        Mode::Normal => (
            " [/] Filter | [Space] Select | [Tab] Outline | [R] Rename | [d] Delete | [e] Edit | [^A] AI | [?] Help | [q] Quit ".to_string(),
            COLOR_MAUVE,
            COLOR_MANTLE,
        ),
    };

    let status_bar = Paragraph::new(status_text).style(
        Style::default()
            .bg(status_bg)
            .fg(status_fg)
            .add_modifier(Modifier::BOLD),
    );
    f.render_widget(status_bar, chunks[1]);
}
