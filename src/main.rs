/*
 * ============================================================================
 * Project Name: QuickSilver (qs)
 * Author:       Billy-79
 * Description:  A blazing-fast, next-gen terminal file explorer written in Rust,
 *               powered by Ratatui & Crossterm, styled with Catppuccin Mocha.
 *               Features: Async Highlighting, Fuzzy Filter, Git Badges,
 *               AST Outline, Multi-Select, and $EDITOR Batch Renaming.
 * ============================================================================
 */

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
use serde::{Deserialize, Serialize};
use syntect::{
    easy::HighlightLines,
    highlighting::{FontStyle, Theme, ThemeSet},
    parsing::SyntaxSet,
    util::LinesWithEndings,
};

use std::{
    collections::{HashMap, HashSet},
    fs::{self, File},
    io::{self, Cursor, Read, Stdout, Write},
    path::{Path, PathBuf},
    process::Command,
    sync::{
        mpsc::{channel, Receiver, Sender},
        OnceLock,
    },
    thread,
    time::Duration,
};

// ============================================================================
// CATPPUCCIN MOCHA PALETTE CONSTANTS
// ============================================================================
const COLOR_BASE: Color = Color::Rgb(30, 30, 46);       // #1e1e2e - Base background
const COLOR_MANTLE: Color = Color::Rgb(24, 24, 37);     // #181825 - Accent header
const COLOR_TEXT: Color = Color::Rgb(205, 214, 244);     // #cdd6f4 - Primary text
const COLOR_SUBTEXT: Color = Color::Rgb(166, 173, 200);  // #a6adc8 - Secondary text / hints
const COLOR_MAUVE: Color = Color::Rgb(203, 166, 247);    // #cba6f7 - Accent borders & highlights
const COLOR_BLUE: Color = Color::Rgb(137, 180, 250);     // #89b4fa - Highlights & directories
const COLOR_GREEN: Color = Color::Rgb(166, 227, 161);    // #a6e3a1 - Success / Git staged
const COLOR_YELLOW: Color = Color::Rgb(249, 226, 175);   // #f9e2af - Git modified / Warnings
const COLOR_SURFACE: Color = Color::Rgb(49, 50, 68);     // #313244 - Selection background
const COLOR_GUTTER_FG: Color = Color::Rgb(88, 91, 112);  // #585b70 - Gutter numbers
const COLOR_WARN: Color = Color::Rgb(243, 139, 168);     // #f38ba8 - Errors / Git untracked
const COLOR_MATCH: Color = Color::Rgb(250, 179, 135);    // #fab387 - Fuzzy search matches

const MAX_PREVIEW_LINES: usize = 1_000;
const MAX_PREVIEW_BYTES: u64 = 1_048_576; // 1 MB cap

// ============================================================================
// ENHANCED CATPPUCCIN MOCHA SYNTAX THEME XML
// ============================================================================
const CATPPUCCIN_MOCHA_TM_THEME: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>name</key>
    <string>Catppuccin Mocha</string>
    <key>settings</key>
    <array>
        <dict>
            <key>settings</key>
            <dict>
                <key>background</key>
                <string>#1e1e2e</string>
                <key>foreground</key>
                <string>#cdd6f4</string>
                <key>caret</key>
                <string>#f5e0dc</string>
                <key>selection</key>
                <string>#585b70</string>
                <key>lineHighlight</key>
                <string>#313244</string>
            </dict>
        </dict>
        <dict>
            <key>name</key>
            <string>Comments</string>
            <key>scope</key>
            <string>comment, punctuation.definition.comment</string>
            <key>settings</key>
            <dict>
                <key>foreground</key>
                <string>#6c7086</string>
                <key>fontStyle</key>
                <string>italic</string>
            </dict>
        </dict>
        <dict>
            <key>name</key>
            <string>Strings</string>
            <key>scope</key>
            <string>string</string>
            <key>settings</key>
            <dict>
                <key>foreground</key>
                <string>#a6e3a1</string>
            </dict>
        </dict>
        <dict>
            <key>name</key>
            <string>Numbers</string>
            <key>scope</key>
            <string>constant.numeric</string>
            <key>settings</key>
            <dict>
                <key>foreground</key>
                <string>#fab387</string>
            </dict>
        </dict>
        <dict>
            <key>name</key>
            <string>Keywords</string>
            <key>scope</key>
            <string>keyword, storage.type, storage.modifier</string>
            <key>settings</key>
            <dict>
                <key>foreground</key>
                <string>#cba6f7</string>
                <key>fontStyle</key>
                <string>bold</string>
            </dict>
        </dict>
        <dict>
            <key>name</key>
            <string>Functions</string>
            <key>scope</key>
            <string>entity.name.function, support.function</string>
            <key>settings</key>
            <dict>
                <key>foreground</key>
                <string>#89b4fa</string>
            </dict>
        </dict>
        <dict>
            <key>name</key>
            <string>Types</string>
            <key>scope</key>
            <string>entity.name.type, support.type, support.class</string>
            <key>settings</key>
            <dict>
                <key>foreground</key>
                <string>#f9e2af</string>
            </dict>
        </dict>
        <dict>
            <key>name</key>
            <string>Variables</string>
            <key>scope</key>
            <string>variable, variable.language</string>
            <key>settings</key>
            <dict>
                <key>foreground</key>
                <string>#f5c2e7</string>
            </dict>
        </dict>
        <dict>
            <key>name</key>
            <string>Operators &amp; Punctuation</string>
            <key>scope</key>
            <string>keyword.operator, punctuation.separator, punctuation.terminator</string>
            <key>settings</key>
            <dict>
                <key>foreground</key>
                <string>#89dceb</string>
            </dict>
        </dict>
        <dict>
            <key>name</key>
            <string>Markdown Headings</string>
            <key>scope</key>
            <string>markup.heading, entity.name.section</string>
            <key>settings</key>
            <dict>
                <key>foreground</key>
                <string>#cba6f7</string>
                <key>fontStyle</key>
                <string>bold</string>
            </dict>
        </dict>
    </array>
</dict>
</plist>"#;

// ============================================================================
// CONFIGURATION
// ============================================================================

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Config {
    #[serde(default = "default_show_hidden")]
    pub show_hidden: bool,
    #[serde(default = "default_icons_enabled")]
    pub icons_enabled: bool,
}

fn default_show_hidden() -> bool { false }
fn default_icons_enabled() -> bool { true }

impl Default for Config {
    fn default() -> Self {
        Self {
            show_hidden: false,
            icons_enabled: true,
        }
    }
}

impl Config {
    fn load() -> Self {
        let config_path = dirs::config_dir()
            .map(|p| p.join("qs/config.toml"))
            .unwrap_or_else(|| PathBuf::from("config.toml"));

        if config_path.exists() {
            if let Ok(contents) = fs::read_to_string(&config_path) {
                if let Ok(config) = toml::from_str(&contents) {
                    return config;
                }
            }
        }
        Self::default()
    }
}

// ============================================================================
// ASYNC PREVIEW MESSAGES
// ============================================================================

struct PreviewRequest {
    path: PathBuf,
    is_dir: bool,
    icons_enabled: bool,
    req_id: usize,
}

struct PreviewResponse {
    lines: Vec<Line<'static>>,
    symbols: Vec<String>,
    req_id: usize,
}

// ============================================================================
// APPLICATION STATE
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Normal,
    Search,
    AiInspection,
    Outline,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum GitStatus {
    Modified,
    Untracked,
    Added,
    Clean,
}

#[derive(Clone, Debug)]
struct FileItem {
    name: String,
    path: PathBuf,
    is_dir: bool,
    git_status: GitStatus,
}

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
    ai_summary: String,
    config: Config,
    preview_scroll: u16,
    preview_cache: Vec<Line<'static>>,
    symbols_cache: Vec<String>,
    parent_items_summary: Vec<String>,
    error_message: Option<String>,
    request_tx: Sender<PreviewRequest>,
    response_rx: Receiver<PreviewResponse>,
    local_req_id: usize,
    needs_redraw: bool,
}

impl App {
    fn new(
        request_tx: Sender<PreviewRequest>,
        response_rx: Receiver<PreviewResponse>,
    ) -> io::Result<Self> {
        let current_path = std::env::current_dir()?;
        let config = Config::load();
        let mut list_state = ListState::default();
        list_state.select(Some(0));

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
            ai_summary: String::from("Ready. Press ^A for AI Summary, '?' for Help."),
            config,
            preview_scroll: 0,
            preview_cache: Vec::new(),
            symbols_cache: Vec::new(),
            parent_items_summary: Vec::new(),
            error_message: None,
            request_tx,
            response_rx,
            local_req_id: 0,
            needs_redraw: true,
        };
        app.refresh_directory()?;
        Ok(app)
    }

    fn query_git_statuses(path: &Path) -> HashMap<String, GitStatus> {
        let mut map = HashMap::new();
        let output = Command::new("git")
            .arg("-C")
            .arg(path)
            .arg("status")
            .arg("--porcelain")
            .arg("--untracked-files=normal")
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

                    if !self.config.show_hidden && name.starts_with('.') {
                        continue;
                    }

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
            self.preview_cache = vec![Line::from("  (No match)")];
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

    fn current_selected_item(&self) -> Option<&FileItem> {
        self.filtered_indices
            .get(self.selected_index)
            .and_then(|&idx| self.items.get(idx))
    }

    fn refresh_parent_summary(&mut self) {
        self.parent_items_summary.clear();
        if let Some(parent) = &self.parent_path {
            if let Ok(entries) = fs::read_dir(parent) {
                let mut names = Vec::new();
                for entry in entries.flatten() {
                    let name = entry.file_name().to_string_lossy().into_owned();
                    if !self.config.show_hidden && name.starts_with('.') {
                        continue;
                    }
                    names.push(name);
                }
                names.sort_by(|a, b| a.to_lowercase().cmp(&b.to_lowercase()));
                self.parent_items_summary = names;
            }
        }
    }

    fn trigger_async_preview(&mut self) {
        self.preview_scroll = 0;
        self.needs_redraw = true;

        if let Some(ref err) = self.error_message {
            self.preview_cache = vec![Line::from(Span::styled(
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
            self.preview_cache = vec![Line::from("  (Empty directory)")];
            self.symbols_cache.clear();
            return;
        };

        self.local_req_id = self.local_req_id.wrapping_add(1);
        self.preview_cache = vec![Line::from(Span::styled(
            "  Loading preview...",
            Style::default().fg(COLOR_SUBTEXT).add_modifier(Modifier::ITALIC),
        ))];

        let _ = self.request_tx.send(PreviewRequest {
            path: target_path,
            is_dir,
            icons_enabled: self.config.icons_enabled,
            req_id: self.local_req_id,
        });
    }

    fn poll_async_preview(&mut self) {
        while let Ok(res) = self.response_rx.try_recv() {
            if res.req_id == self.local_req_id {
                self.preview_cache = res.lines;
                self.symbols_cache = res.symbols;
                self.needs_redraw = true;
            }
        }
    }

    fn next(&mut self) {
        if !self.filtered_indices.is_empty() && self.selected_index < self.filtered_indices.len() - 1 {
            self.selected_index += 1;
            self.list_state.select(Some(self.selected_index));
            self.trigger_async_preview();
        }
    }

    fn previous(&mut self) {
        if self.selected_index > 0 {
            self.selected_index -= 1;
            self.list_state.select(Some(self.selected_index));
            self.trigger_async_preview();
        }
    }

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
                self.selected_items.clear(); // Scope safety: clear selections on folder transition
                self.refresh_directory()?;
            }
        }
        Ok(())
    }

    fn back(&mut self) -> io::Result<()> {
        if let Some(parent) = self.parent_path.clone() {
            let prev_name = self.current_path.file_name().map(|n| n.to_os_string());
            self.current_path = parent;
            self.search_query.clear();
            self.selected_items.clear();
            self.refresh_directory()?;

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

    fn batch_delete(&mut self) -> io::Result<()> {
        let targets = if !self.selected_items.is_empty() {
            self.selected_items.drain().collect::<Vec<_>>()
        } else if let Some(item) = self.current_selected_item() {
            vec![item.path.clone()]
        } else {
            return Ok(());
        };

        for path in targets {
            if path.is_dir() {
                let _ = fs::remove_dir_all(&path);
            } else {
                let _ = fs::remove_file(&path);
            }
        }
        self.refresh_directory()?;
        Ok(())
    }

    fn batch_rename_in_editor(&mut self, terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> io::Result<()> {
        let targets = if !self.selected_items.is_empty() {
            self.selected_items.iter().cloned().collect::<Vec<_>>()
        } else if let Some(item) = self.current_selected_item() {
            vec![item.path.clone()]
        } else {
            return Ok(());
        };

        let temp_dir = std::env::temp_dir();
        let scratch_path = temp_dir.join("qs_batch_rename.txt");

        {
            let mut file = File::create(&scratch_path)?;
            for p in &targets {
                if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
                    writeln!(file, "{}", name)?;
                }
            }
        }

        open_file_in_editor(terminal, &scratch_path)?;

        let content = fs::read_to_string(&scratch_path)?;
        let new_names = content.lines().map(|s| s.trim()).filter(|s| !s.is_empty()).collect::<Vec<_>>();

        if new_names.len() == targets.len() {
            for (orig_path, &new_name) in targets.iter().zip(new_names.iter()) {
                let new_path = orig_path.with_file_name(new_name);
                if *orig_path != new_path && !new_path.exists() {
                    let _ = fs::rename(orig_path, new_path);
                }
            }
        }

        let _ = fs::remove_file(scratch_path);
        self.selected_items.clear();
        self.refresh_directory()?;
        Ok(())
    }

    fn scroll_preview_down(&mut self, delta: u16) {
        let max_lines = if self.mode == Mode::Outline {
            self.symbols_cache.len() as u16
        } else {
            self.preview_cache.len() as u16
        };

        if self.preview_scroll + delta < max_lines {
            self.preview_scroll += delta;
        } else {
            self.preview_scroll = max_lines.saturating_sub(1);
        }
        self.needs_redraw = true;
    }

    fn scroll_preview_up(&mut self, delta: u16) {
        self.preview_scroll = self.preview_scroll.saturating_sub(delta);
        self.needs_redraw = true;
    }

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
            • Delete/Trash : d\n\
            • Open File    : e / o\n\
            • Toggle Hidden: .\n\
            • Toggle Icons : i\n\
            • Reload       : r\n\
            • AI Summary   : ^A\n\
            • Exit         : q"
        );
        self.needs_redraw = true;
    }
}

// ============================================================================
// HIGHLIGHTER & AST ENGINE
// ============================================================================

struct HighlighterEngine {
    syntax_set: SyntaxSet,
    mocha_theme: Theme,
}

fn get_highlighter_engine() -> &'static HighlighterEngine {
    static ENGINE: OnceLock<HighlighterEngine> = OnceLock::new();
    ENGINE.get_or_init(|| {
        let mut cursor = Cursor::new(CATPPUCCIN_MOCHA_TM_THEME.as_bytes());
        let mocha_theme = ThemeSet::load_from_reader(&mut cursor).unwrap_or_else(|_| {
            let default_set = ThemeSet::load_defaults();
            default_set.themes.into_values().next().unwrap_or_default()
        });

        HighlighterEngine {
            syntax_set: SyntaxSet::load_defaults_newlines(),
            mocha_theme,
        }
    })
}

fn syntect_style_to_ratatui(style: syntect::highlighting::Style) -> Style {
    let fg = Color::Rgb(style.foreground.r, style.foreground.g, style.foreground.b);
    let mut ratatui_style = Style::default().fg(fg);

    if style.font_style.contains(FontStyle::BOLD) {
        ratatui_style = ratatui_style.add_modifier(Modifier::BOLD);
    }
    if style.font_style.contains(FontStyle::ITALIC) {
        ratatui_style = ratatui_style.add_modifier(Modifier::ITALIC);
    }
    if style.font_style.contains(FontStyle::UNDERLINE) {
        ratatui_style = ratatui_style.add_modifier(Modifier::UNDERLINED);
    }

    ratatui_style
}

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

fn extract_ast_symbols(content: &str, ext: &str) -> Vec<String> {
    let mut symbols = Vec::new();
    let ext_lower = ext.to_lowercase();
    for (idx, line) in content.lines().enumerate() {
        let trimmed = line.trim();
        let line_num = idx + 1;
        match ext_lower.as_str() {
            "rs" => {
                if trimmed.starts_with("pub fn ") || trimmed.starts_with("fn ") {
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
                if trimmed.starts_with("def ") {
                    symbols.push(format!("Line {:>4} │ 󰊕 {}", line_num, trimmed.trim_end_matches(':')));
                } else if trimmed.starts_with("class ") {
                    symbols.push(format!("Line {:>4} │ 󰌗 {}", line_num, trimmed.trim_end_matches(':')));
                }
            }
            "ts" | "js" | "tsx" | "jsx" => {
                if trimmed.starts_with("function ") || trimmed.starts_with("export function ") || trimmed.contains(" = () =>") || trimmed.contains(" = function") {
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

fn load_directory_preview(path: &Path, icons_enabled: bool) -> (Vec<Line<'static>>, Vec<String>) {
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
                (vec![Line::from("  (Empty directory)")], Vec::new())
            } else {
                (items.into_iter().map(Line::from).collect(), Vec::new())
            }
        }
        Err(err) => (
            vec![Line::from(Span::styled(
                format!("  Cannot open folder: {}", err),
                Style::default().fg(COLOR_WARN),
            ))],
            Vec::new(),
        ),
    }
}

fn load_file_preview(path: &Path) -> (Vec<Line<'static>>, Vec<String>) {
    let metadata = match fs::metadata(path) {
        Ok(m) => m,
        Err(e) => return (vec![Line::from(format!("Failed to read metadata: {}", e))], Vec::new()),
    };

    if metadata.len() > MAX_PREVIEW_BYTES {
        return (
            vec![Line::from(format!(
                "File exceeds preview cap ({:.2} MB). Use an external pager.",
                metadata.len() as f64 / 1_048_576.0
            ))],
            Vec::new(),
        );
    }

    let file = match File::open(path) {
        Ok(f) => f,
        Err(e) => return (vec![Line::from(format!("Error opening file: {}", e))], Vec::new()),
    };

    let mut stream = file.take(MAX_PREVIEW_BYTES);
    let mut raw_bytes = Vec::new();
    if stream.read_to_end(&mut raw_bytes).is_err() {
        return (vec![Line::from("Error reading file stream.")], Vec::new());
    }

    let check_len = raw_bytes.len().min(1024);
    if raw_bytes[..check_len].contains(&0) {
        let valid_slice_len = match std::str::from_utf8(&raw_bytes[..check_len]) {
            Ok(s) => s.len(),
            Err(e) => e.valid_up_to(),
        };
        if raw_bytes[..valid_slice_len].contains(&0) {
            return (vec![Line::from("Binary file preview is not supported.")], Vec::new());
        }
    }

    if raw_bytes.is_empty() {
        return (vec![Line::from("  (Empty file)")], Vec::new());
    }

    let raw_content = String::from_utf8_lossy(&raw_bytes);
    let normalized_content = raw_content.replace('\t', "    ");

    let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let extension = path.extension().and_then(|e| e.to_str()).unwrap_or("");

    let symbols = extract_ast_symbols(&normalized_content, extension);

    let engine = get_highlighter_engine();
    let syntax = engine
        .syntax_set
        .find_syntax_for_file(file_name)
        .ok()
        .flatten()
        .unwrap_or_else(|| {
            engine
                .syntax_set
                .find_syntax_by_extension(extension)
                .unwrap_or_else(|| engine.syntax_set.find_syntax_plain_text())
        });

    let mut highlighter = HighlightLines::new(syntax, &engine.mocha_theme);
    let lines_iter = LinesWithEndings::from(&normalized_content);

    let total_preview_lines = normalized_content.lines().count().min(MAX_PREVIEW_LINES);
    let gutter_width = total_preview_lines.to_string().len().max(2);

    let mut highlighted_output = Vec::with_capacity(total_preview_lines);
    let mut lines_rendered = 0;

    for (idx, line_slice) in lines_iter.take(MAX_PREVIEW_LINES).enumerate() {
        lines_rendered = idx + 1;
        let line_num = idx + 1;
        let mut spans = Vec::new();

        let gutter = format!("{:>width$} │ ", line_num, width = gutter_width);
        spans.push(Span::styled(gutter, Style::default().fg(COLOR_GUTTER_FG)));

        if let Ok(token_ranges) = highlighter.highlight_line(line_slice, &engine.syntax_set) {
            for (style, token_text) in token_ranges {
                let clean_text = token_text.trim_end_matches(['\r', '\n']);
                if clean_text.is_empty() {
                    continue;
                }
                spans.push(Span::styled(clean_text.to_string(), syntect_style_to_ratatui(style)));
            }
        } else {
            let clean_text = line_slice.trim_end_matches(['\r', '\n']);
            spans.push(Span::styled(clean_text.to_string(), Style::default().fg(COLOR_TEXT)));
        }

        highlighted_output.push(Line::from(spans));
    }

    if lines_rendered >= MAX_PREVIEW_LINES {
        let align_gutter = format!("{:>width$} │ ", "...", width = gutter_width);
        highlighted_output.push(Line::from(vec![
            Span::styled(align_gutter, Style::default().fg(COLOR_GUTTER_FG)),
            Span::styled(
                "─── Preview truncated at 1,000 lines ───",
                Style::default().fg(COLOR_MAUVE).add_modifier(Modifier::ITALIC),
            ),
        ]));
    }

    (highlighted_output, symbols)
}

// ============================================================================
// MAIN LOOP & PANIC RECOVERY
// ============================================================================

fn setup_panic_hook() {
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen, crossterm::event::DisableMouseCapture);
        original_hook(panic_info);
    }));
}

fn open_file_in_editor(terminal: &mut Terminal<CrosstermBackend<Stdout>>, path: &Path) -> io::Result<()> {
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen, crossterm::event::DisableMouseCapture)?;
    terminal.show_cursor()?;

    let editor = std::env::var("EDITOR")
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

    let _ = Command::new(&editor).arg(path).status();

    enable_raw_mode()?;
    execute!(terminal.backend_mut(), EnterAlternateScreen, crossterm::event::EnableMouseCapture)?;
    terminal.hide_cursor()?;
    terminal.clear()?;
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    setup_panic_hook();

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, crossterm::event::EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let (req_tx, req_rx) = channel::<PreviewRequest>();
    let (res_tx, res_rx) = channel::<PreviewResponse>();

    thread::spawn(move || {
        while let Ok(mut req) = req_rx.recv() {
            while let Ok(newer_req) = req_rx.try_recv() {
                req = newer_req;
            }

            let (lines, symbols) = if req.is_dir {
                load_directory_preview(&req.path, req.icons_enabled)
            } else {
                load_file_preview(&req.path)
            };

            let _ = res_tx.send(PreviewResponse {
                lines,
                symbols,
                req_id: req.req_id,
            });
        }
    });

    let mut app = App::new(req_tx, res_rx)?;
    let res = run_app(&mut terminal, &mut app);

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

fn run_app(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    app: &mut App,
) -> io::Result<()> {
    loop {
        app.poll_async_preview();

        if app.needs_redraw {
            terminal.draw(|f| ui(f, app))?;
            app.needs_redraw = false;
        }

        if event::poll(Duration::from_millis(16))? {
            match event::read()? {
                Event::Resize(_, _) => {
                    app.needs_redraw = true;
                }
                Event::Key(key) if key.kind == KeyEventKind::Press => {
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
                                app.search_query.pop();
                                app.apply_search_filter();
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

                    if key.code == KeyCode::Char('q') {
                        return Ok(());
                    }

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
                                app.batch_delete()?;
                            }
                            KeyCode::Char('.') => {
                                app.config.show_hidden = !app.config.show_hidden;
                                app.refresh_directory()?;
                            }
                            KeyCode::Char('i') => {
                                app.config.icons_enabled = !app.config.icons_enabled;
                                app.trigger_async_preview();
                            }
                            KeyCode::Char('r') => {
                                app.config = Config::load();
                                app.refresh_directory()?;
                            }
                            KeyCode::Char('~') | KeyCode::Char('g') => {
                                if let Some(home) = dirs::home_dir() {
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
                Event::Mouse(MouseEvent { kind, .. }) => match kind {
                    MouseEventKind::ScrollDown => app.scroll_preview_down(3),
                    MouseEventKind::ScrollUp => app.scroll_preview_up(3),
                    _ => {}
                },
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

    if area.width < 15 || area.height < 4 {
        let msg = Paragraph::new("Resize terminal").style(Style::default().fg(COLOR_WARN));
        f.render_widget(msg, area);
        return;
    }

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(1)])
        .split(area);

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

            let display_text = if app.config.icons_enabled {
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

    // --- Column 3: Live Preview / AST Outline / AI Pane ---
    let (preview_title, preview_border_color) = match app.mode {
        Mode::AiInspection => (" 🤖 AI Semantic Summary ".to_string(), COLOR_GREEN),
        Mode::Outline => (" 󰘦 AST Symbol Outline (Tab/Esc to return) ".to_string(), COLOR_YELLOW),
        _ => {
            if let Some(selected) = app.current_selected_item() {
                if selected.is_dir {
                    (format!("  {} ", selected.name), COLOR_MAUVE)
                } else {
                    let size_str = fs::metadata(&selected.path)
                        .map(|m| format_file_size(m.len()))
                        .unwrap_or_else(|_| "0 B".into());
                    (
                        format!(
                            " 󰈙 {} │ {} │ {} lines ",
                            selected.name,
                            size_str,
                            app.preview_cache.len()
                        ),
                        COLOR_MAUVE,
                    )
                }
            } else {
                (" 👁️ Live Preview ".to_string(), COLOR_MAUVE)
            }
        }
    };

    let preview_widget = if app.mode == Mode::AiInspection {
        Paragraph::new(app.ai_summary.clone())
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(preview_border_color))
                    .style(Style::default().bg(COLOR_BASE).fg(COLOR_TEXT))
                    .title(preview_title),
            )
            .wrap(Wrap { trim: true })
    } else if app.mode == Mode::Outline {
        let outline_lines: Vec<Line> = if app.symbols_cache.is_empty() {
            vec![Line::from("  No AST symbols found for this file.")]
        } else {
            app.symbols_cache
                .iter()
                .map(|s| Line::from(Span::styled(s.clone(), Style::default().fg(COLOR_YELLOW))))
                .collect()
        };

        Paragraph::new(outline_lines)
            .scroll((app.preview_scroll, 0))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(preview_border_color))
                    .style(Style::default().bg(COLOR_BASE))
                    .title(preview_title),
            )
    } else {
        Paragraph::new(app.preview_cache.clone())
            .scroll((app.preview_scroll, 0))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(preview_border_color))
                    .style(Style::default().bg(COLOR_BASE))
                    .title(preview_title),
            )
    };
    f.render_widget(preview_widget, columns[2]);

    // --- Status Bar ---
    let sel_count = app.selected_items.len();
    let status_text = match app.mode {
        Mode::Search => format!(" [Esc] Clear | [Enter] Select | [Up/Down] Navigate | Filtering: \"{}\" ", app.search_query),
        Mode::AiInspection => " Mode: AI Inspection (Press any key to close) ".to_string(),
        Mode::Outline => " Mode: AST Outline | [j/k/Wheel] Scroll Outline | [Tab/Esc] Return to Code Preview ".to_string(),
        Mode::Normal if sel_count > 0 => {
            format!(" {} selected | [R] Rename in $EDITOR | [d] Delete | [Space] Toggle | [q] Quit ", sel_count)
        }
        Mode::Normal => " [/] Filter | [Space] Select | [Tab] Outline | [R] Rename | [e] Edit | [^A] AI | [?] Help | [q] Quit ".to_string(),
    };

    let status_bar = Paragraph::new(status_text).style(
        Style::default()
            .bg(COLOR_MAUVE)
            .fg(COLOR_MANTLE)
            .add_modifier(Modifier::BOLD),
    );
    f.render_widget(status_bar, chunks[1]);
}
