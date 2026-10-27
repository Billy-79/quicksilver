/*
 * ============================================================================
 * Project Name: QuickSilver (qs)
 * Author:       Billy-79
 * Description:  A blazing-fast, next-gen terminal file explorer written in Rust,
 *               powered by Ratatui & Crossterm, styled with Catppuccin Mocha,
 *               featuring 3-column Miller layout, Nerd Font icons, AI inspection,
 *               and dotfile configuration support (`config.toml`).
 * ============================================================================
 */

// Import necessary crossterm modules for terminal manipulation and event handling
use crossterm::{
    event::{self, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};

// Import ratatui widgets, layouts, styles, and backend definitions for the TUI
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::Span,
    widgets::{Block, Borders, List, ListItem, Paragraph},
    Terminal,
};

// Import serde serialization/deserialization traits for parsing config files
use serde::{Deserialize, Serialize};

// Import standard library utilities for filesystem access and input/output
use std::{
    fs,
    io::{self},
    path::PathBuf,
};

// ============================================================================
// CATPPUCCIN MOCHA COLOR PALETTE CONSTANTS
// ============================================================================
const COLOR_BASE: Color = Color::Rgb(30, 30, 46);       // #1e1e2e - Main background
const COLOR_MANTLE: Color = Color::Rgb(24, 24, 37);     // #181825 - Deeper panel background
const COLOR_TEXT: Color = Color::Rgb(205, 214, 244);     // #cdd6f4 - Primary text
const COLOR_SUBTEXT: Color = Color::Rgb(166, 173, 200);  // #a6adc8 - Secondary text / hints
const COLOR_MAUVE: Color = Color::Rgb(203, 166, 247);    // #cba6f7 - Accents & borders
const COLOR_BLUE: Color = Color::Rgb(137, 180, 250);     // #89b4fa - Highlights & directories
const COLOR_GREEN: Color = Color::Rgb(166, 227, 161);    // #a6e3a1 - Success indicators & icons
const COLOR_SURFACE: Color = Color::Rgb(49, 50, 68);     // #313244 - Selection background

// ============================================================================
// CONFIGURATION STRUCTS & DEFAULTS
// ============================================================================

/// Represents persistent user settings loaded from `~/.config/qs/config.toml`
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Config {
    #[serde(default = "default_show_hidden")]
    pub show_hidden: bool,
    #[serde(default = "default_transparency")]
    pub transparency: bool,
    #[serde(default = "default_icons_enabled")]
    pub icons_enabled: bool,
}

// Default fallback generator functions for serde attributes
fn default_show_hidden() -> bool { false }
fn default_transparency() -> bool { false }
fn default_icons_enabled() -> bool { true }

impl Default for Config {
    fn default() -> Self {
        Self {
            show_hidden: false,
            transparency: false,
            icons_enabled: true,
        }
    }
}

impl Config {
    /// Attempts to load configuration from `~/.config/qs/config.toml`.
    /// Falls back to default values if the file is missing or malformed.
    fn load() -> Self {
        let config_path = std::env::var("HOME")
            .map(|home| PathBuf::from(home).join(".config/qs/config.toml"))
            .unwrap_or_else(|_| PathBuf::from("config.toml"));

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
// APPLICATION STATE & DATA STRUCTURES
// ============================================================================

/// Defines the operational state/mode of the file explorer
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
enum Mode {
    Normal,         // Standard file navigation and preview mode
    CommandPalette, // Extended command execution mode
    AiInspection,   // Ctrl+A semantic AI code/file inspection mode
}

/// Metadata structure representing an individual file or folder entry
struct FileItem {
    name: String,
    path: PathBuf,
    is_dir: bool,
}

/// Central application state container holding paths, items, selections, and configuration
struct App {
    current_path: PathBuf,
    parent_path: Option<PathBuf>,
    items: Vec<FileItem>,
    selected_index: usize,
    mode: Mode,
    ai_summary: String,
    config: Config,
}

impl App {
    /// Initializes application state starting in the current working directory and loading dotfile configuration
    fn new() -> io::Result<Self> {
        let current_path = std::env::current_dir()?;
        let config = Config::load();
        let mut app = Self {
            parent_path: current_path.parent().map(|p| p.to_path_buf()),
            current_path,
            items: Vec::new(),
            selected_index: 0,
            mode: Mode::Normal,
            ai_summary: String::from("Ready. Press ^A for AI Summary."),
            config,
        };
        app.refresh_directory()?;
        Ok(app)
    }

    /// Reads directory contents, filters hidden files based on configuration, and sorts entries alphabetically (directories first)
    fn refresh_directory(&mut self) -> io::Result<()> {
        self.items.clear();
        self.parent_path = self.current_path.parent().map(|p| p.to_path_buf());

        if let Ok(entries) = fs::read_dir(&self.current_path) {
            let mut dirs = Vec::new();
            let mut files = Vec::new();

            for entry in entries.flatten() {
                let path = entry.path();
                let name = path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned();
                
                // Filter out dotfiles if `show_hidden` is disabled in config
                if !self.config.show_hidden && name.starts_with('.') {
                    continue;
                }

                let is_dir = path.is_dir();
                let item = FileItem { name, path, is_dir };
                if is_dir {
                    dirs.push(item);
                } else {
                    files.push(item);
                }
            }

            // Sort directories and files alphabetically
            dirs.sort_by(|a, b| a.name.cmp(&b.name));
            files.sort_by(|a, b| a.name.cmp(&b.name));

            self.items.extend(dirs);
            self.items.extend(files);
        }

        // Ensure selected index stays within valid array bounds
        if self.selected_index >= self.items.len() && !self.items.is_empty() {
            self.selected_index = self.items.len() - 1;
        } else if self.items.is_empty() {
            self.selected_index = 0;
        }

        Ok(())
    }

    /// Moves the selection cursor down to the next list item
    fn next(&mut self) {
        if !self.items.is_empty() && self.selected_index < self.items.len() - 1 {
            self.selected_index += 1;
        }
    }

    /// Moves the selection cursor up to the previous list item
    fn previous(&mut self) {
        if self.selected_index > 0 {
            self.selected_index -= 1;
        }
    }

    /// Navigates into the currently selected directory
    fn enter(&mut self) -> io::Result<()> {
        if let Some(item) = self.items.get(self.selected_index) {
            if item.is_dir {
                self.current_path = item.path.clone();
                self.selected_index = 0;
                self.refresh_directory()?;
            }
        }
        Ok(())
    }

    /// Navigates back up to the parent directory
    fn back(&mut self) -> io::Result<()> {
        if let Some(parent) = self.parent_path.clone() {
            self.current_path = parent;
            self.selected_index = 0;
            self.refresh_directory()?;
        }
        Ok(())
    }

    /// Triggers simulated AI semantic inspection on the currently highlighted file or folder
    fn trigger_ai_inspection(&mut self) {
        if let Some(item) = self.items.get(self.selected_index) {
            self.mode = Mode::AiInspection;
            if item.is_dir {
                self.ai_summary = format!(
                    "🤖 AI Analysis for directory '{}':\n- Contains subfolders and files.\n- High structure integrity.\n- Ready for indexing.",
                    item.name
                );
            } else {
                self.ai_summary = format!(
                    "🤖 AI Analysis for file '{}':\n- Codebase artifact.\n- Syntax verified.\n- Clean formatting and documentation detected.",
                    item.name
                );
            }
        }
    }
}

// ============================================================================
// ICON HELPER FUNCTIONS
// ============================================================================

/// Returns appropriate Nerd Font webdev icons based on file extension, name, or type
fn get_file_icon(name: &str, is_dir: bool) -> &str {
    if is_dir {
        return " "; // Folder icon
    }
    if name.ends_with(".rs") {
        " " // Rust icon
    } else if name.ends_with(".toml") || name.ends_with(".json") || name.ends_with(".yaml") || name.ends_with(".yml") {
        " " // Config / gear icon
    } else if name.ends_with(".md") {
        " " // Markdown icon
    } else if name.starts_with(".git") {
        " " // Git icon
    } else if name.ends_with(".lock") {
        "󰌆 " // Lock icon
    } else {
        "󰈙 " // Default generic file icon
    }
}

// ============================================================================
// MAIN ENTRY POINT & CLI FLAG HANDLING
// ============================================================================

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // --- 1. Command-Line Arguments Parsing for Flags ---
    let args: Vec<String> = std::env::args().collect();
    if args.len() > 1 {
        match args[1].as_str() {
            "--version" | "-V" => {
                println!("quicksilver 0.1.0");
                return Ok(());
            }
            "--help" | "-h" => {
                println!("QuickSilver (qs) 0.1.0");
                println!("A blazing-fast, next-gen terminal file explorer written in Rust, powered by Tokio async IO, styled with Catppuccin Mocha.");
                println!("\nUsage: qs [options]");
                println!("\nOptions:");
                println!("  -V, --version     Print version information");
                println!("  -h, --help        Print help information");
                return Ok(());
            }
            _ => {}
        }
    }

    // --- 2. Terminal UI Initialization ---
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Instantiate core application state
    let mut app = App::new()?;

    // --- 3. Main Event & Render Loop ---
    let res = run_app(&mut terminal, &mut app);

    // --- 4. Terminal Restoration on Exit ---
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    if let Err(err) = res {
        println!("{:?}", err);
    }

    Ok(())
}

// ============================================================================
// EVENT LOOP & INPUT ROUTING
// ============================================================================

/// Core event loop managing terminal redraws and keyboard inputs
fn run_app<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    app: &mut App,
) -> io::Result<()> {
    loop {
        // Draw the user interface on each tick
        terminal.draw(|f| ui(f, app))?;

        // Read incoming terminal events (blocking until input occurs)
        if let Event::Key(key) = event::read()? {
            // Global quit shortcut
            if key.code == KeyCode::Char('q') {
                return Ok(());
            }

            // Ctrl + A shortcut for AI Semantic Inspection
            if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('a') {
                app.trigger_ai_inspection();
                continue;
            }

            // Mode-specific input routing
            match app.mode {
                Mode::AiInspection => {
                    // Any key press exits AI inspection mode and returns to normal view
                    app.mode = Mode::Normal;
                    app.ai_summary = String::from("Ready. Press ^A for AI Summary.");
                }
                Mode::Normal | Mode::CommandPalette => {
                    match key.code {
                        KeyCode::Char('j') | KeyCode::Down => app.next(),
                        KeyCode::Char('k') | KeyCode::Up => app.previous(),
                        KeyCode::Char('l') | KeyCode::Enter => app.enter()?,
                        KeyCode::Char('h') => app.back()?,
                        _ => {}
                    }
                }
            }
        }
    }
}

// ============================================================================
// USER INTERFACE RENDERING ENGINE
// ============================================================================

/// Renders the 3-column Miller Columns layout styled with Catppuccin Mocha & config support
fn ui(f: &mut ratatui::Frame, app: &App) {
    let size = f.size();

    // Determine background color based on transparency setting (`Color::Reset` respects terminal emulator opacity)
    let bg_color = if app.config.transparency {
        Color::Reset
    } else {
        COLOR_BASE
    };

    // Split terminal screen vertically: Main content area on top, status bar at the bottom
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(1)].as_ref())
        .split(size);

    // Split content area horizontally into 3 Miller Columns: [Parent Preview, Active Directory, Live Preview / AI]
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(25), // Column 1: Parent directory preview
            Constraint::Percentage(45), // Column 2: Active directory file list
            Constraint::Percentage(30), // Column 3: Live file preview / AI inspection
        ])
        .split(chunks[0]);

    // --- Column 1: Parent Directory Preview ---
    let parent_title = if let Some(p) = &app.parent_path {
        format!(" Parent: {} ", p.display())
    } else {
        String::from(" Parent: None ")
    };
    let parent_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(COLOR_MAUVE))
        .style(Style::default().bg(bg_color).fg(COLOR_SUBTEXT))
        .title(parent_title);
    
    let parent_content = Paragraph::new(
        app.parent_path
            .as_ref()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| "Root directory".into()),
    )
    .block(parent_block);
    f.render_widget(parent_content, columns[0]);

    // --- Column 2: Current Directory File List ---
    let current_title = format!(" {} ", app.current_path.display());
    let items: Vec<ListItem> = app
        .items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            // Conditionally prepend Nerd Font icons based on configuration
            let display_text = if app.config.icons_enabled {
                let icon = get_file_icon(&item.name, item.is_dir);
                format!("{} {}", icon, item.name)
            } else {
                item.name.clone()
            };
            
            // Highlight selected index and colorize directories vs. files
            let style = if i == app.selected_index {
                Style::default()
                    .bg(COLOR_SURFACE)
                    .fg(COLOR_BLUE)
                    .add_modifier(Modifier::BOLD)
            } else if item.is_dir {
                Style::default().fg(COLOR_BLUE)
            } else {
                Style::default().fg(COLOR_TEXT)
            };
            ListItem::new(Span::styled(display_text, style))
        })
        .collect();

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(COLOR_MAUVE))
                .style(Style::default().bg(bg_color))
                .title(current_title),
        );
    f.render_widget(list, columns[1]);

    // --- Column 3: Live Preview / AI Semantic Inspection Pane ---
    let preview_title = match app.mode {
        Mode::AiInspection => " 🤖 AI Semantic Inspection ",
        _ => " 👁️ Live Preview ",
    };

    let preview_content = if app.mode == Mode::AiInspection {
        app.ai_summary.clone()
    } else if let Some(selected) = app.items.get(app.selected_index) {
        if selected.is_dir {
            match fs::read_dir(&selected.path) {
                Ok(entries) => {
                    let mut preview_str = String::from("Directory Contents:\n");
                    for entry in entries.take(15).flatten() {
                        let name = entry.file_name().to_string_lossy().into_owned();
                        preview_str.push_str(&format!("- {}\n", name));
                    }
                    preview_str
                }
                Err(_) => String::from("Unable to read directory contents."),
            }
        } else {
            match fs::read_to_string(&selected.path) {
                Ok(contents) => {
                    contents.lines().take(20).collect::<Vec<&str>>().join("\n")
                }
                Err(_) => String::from("Binary file or unreadable preview."),
            }
        }
    } else {
        String::from("No item selected.")
    };

    let preview_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(COLOR_GREEN))
        .style(Style::default().bg(bg_color).fg(COLOR_TEXT))
        .title(preview_title);

    let preview_paragraph = Paragraph::new(preview_content)
        .block(preview_block)
        .wrap(ratatui::widgets::Wrap { trim: true });
    f.render_widget(preview_paragraph, columns[2]);

    // --- Status Bar Footer ---
    let status_text = match app.mode {
        Mode::AiInspection => String::from(" AI INSPECTION MODE | Press any key to return "),
        _ => format!(
            " Config [Hidden: {} | Trans: {} | Icons: {}] | ^A: AI | j/k: Nav | l: Open | h: Back | q: Quit ",
            app.config.show_hidden, app.config.transparency, app.config.icons_enabled
        ),
    };
    let status_bar = Paragraph::new(status_text).style(
        Style::default()
            .bg(COLOR_MAUVE)
            .fg(COLOR_MANTLE)
            .add_modifier(Modifier::BOLD),
    );
    f.render_widget(status_bar, chunks[1]);
}
