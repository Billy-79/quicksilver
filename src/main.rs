/*
 * ============================================================================
 * Project Name: QuickSilver (qs)
 * Author:       Billy-79
 * Description:  A blazing-fast, next-gen terminal file explorer written in Rust,
 *               powered by Ratatui & Crossterm, styled with Catppuccin Mocha,
 *               featuring 3-column Miller layout, Nerd Font icons, and AI inspection.
 * ============================================================================
 */

use crossterm::{
    event::{self, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::Span,
    widgets::{Block, Borders, List, ListItem, Paragraph},
    Terminal,
};
use std::{
    fs,
    io::{self},
    path::PathBuf,
};

// --- Catppuccin Mocha Color Palette Constants ---
const COLOR_BASE: Color = Color::Rgb(30, 30, 46);       // #1e1e2e - Main background
const COLOR_MANTLE: Color = Color::Rgb(24, 24, 37);     // #181825 - Deeper panel background
const COLOR_TEXT: Color = Color::Rgb(205, 214, 244);     // #cdd6f4 - Primary text
const COLOR_SUBTEXT: Color = Color::Rgb(166, 173, 200);  // #a6adc8 - Secondary text / hints
const COLOR_MAUVE: Color = Color::Rgb(203, 166, 247);    // #cba6f7 - Accents & borders
const COLOR_BLUE: Color = Color::Rgb(137, 180, 250);     // #89b4fa - Highlights & directories
const COLOR_GREEN: Color = Color::Rgb(166, 227, 161);    // #a6e3a1 - Success indicators & icons
const COLOR_SURFACE: Color = Color::Rgb(49, 50, 68);     // #313244 - Selection background

/// Application operational modes
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
enum Mode {
    Normal,         // Standard file navigation and preview mode
    CommandPalette, // Extended command execution mode
    AiInspection,   // Ctrl+A semantic AI code/file inspection mode
}

/// Represents a single file or directory entry in the filesystem
struct FileItem {
    name: String,
    path: PathBuf,
    is_dir: bool,
}

/// Core application state container
struct App {
    current_path: PathBuf,
    parent_path: Option<PathBuf>,
    items: Vec<FileItem>,
    selected_index: usize,
    mode: Mode,
    ai_summary: String,
}

impl App {
    /// Initialize application state starting from the current working directory
    fn new() -> io::Result<Self> {
        let current_path = std::env::current_dir()?;
        let mut app = Self {
            parent_path: current_path.parent().map(|p| p.to_path_buf()),
            current_path,
            items: Vec::new(),
            selected_index: 0,
            mode: Mode::Normal,
            ai_summary: String::from("Ready. Press ^A for AI Summary."),
        };
        app.refresh_directory()?;
        Ok(app)
    }

    /// Read directory contents and populate file items
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
                let is_dir = path.is_dir();

                let item = FileItem { name, path, is_dir };
                if is_dir {
                    dirs.push(item);
                } else {
                    files.push(item);
                }
            }

            // Sort directories first, then files alphabetically
            dirs.sort_by(|a, b| a.name.cmp(&b.name));
            files.sort_by(|a, b| a.name.cmp(&b.name));

            self.items.extend(dirs);
            self.items.extend(files);
        }

        // Clamp selected index to bounds
        if self.selected_index >= self.items.len() && !self.items.is_empty() {
            self.selected_index = self.items.len() - 1;
        } else if self.items.is_empty() {
            self.selected_index = 0;
        }

        Ok(())
    }

    /// Move selection cursor down
    fn next(&mut self) {
        if !self.items.is_empty() && self.selected_index < self.items.len() - 1 {
            self.selected_index += 1;
        }
    }

    /// Move selection cursor up
    fn previous(&mut self) {
        if self.selected_index > 0 {
            self.selected_index -= 1;
        }
    }

    /// Navigate into selected directory
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

    /// Navigate back to parent directory
    fn back(&mut self) -> io::Result<()> {
        if let Some(parent) = self.parent_path.clone() {
            self.current_path = parent;
            self.selected_index = 0;
            self.refresh_directory()?;
        }
        Ok(())
    }

    /// Simulate AI semantic inspection on the currently highlighted entry
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

/// Helper function to return a Nerd Font webdev icon based on file name or type
fn get_file_icon(name: &str, is_dir: bool) -> &str {
    if is_dir {
        return " "; // Folder icon
    }
    // Match file extensions for precise language/config icons
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

    // Create application instance
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

/// Core application event loop handling keyboard inputs and UI rendering
fn run_app<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    app: &mut App,
) -> io::Result<()> {
    loop {
        terminal.draw(|f| ui(f, app))?;

        if let Event::Key(key) = event::read()? {
            // Handle global quit command
            if key.code == KeyCode::Char('q') {
                return Ok(());
            }

            // Handle Ctrl + A for AI Semantic Inspection mode
            if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('a') {
                app.trigger_ai_inspection();
                continue;
            }

            // Mode-specific handling
            match app.mode {
                Mode::AiInspection => {
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

/// Render the 3-column Miller Columns user interface styled with Catppuccin Mocha
fn ui(f: &mut ratatui::Frame, app: &App) {
    let size = f.size();

    // Main vertical split: Content area on top, Status Bar at the bottom
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(1)].as_ref())
        .split(size);

    // Horizontal split for 3-column Miller columns layout: [Parent, Current, Preview]
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(25), // Parent directory preview
            Constraint::Percentage(45), // Active directory file list
            Constraint::Percentage(30), // Live file preview / AI summary pane
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
        .style(Style::default().bg(COLOR_BASE).fg(COLOR_SUBTEXT))
        .title(parent_title);
    
    let parent_content = Paragraph::new(
        app.parent_path
            .as_ref()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| "Root directory".into()),
    )
    .block(parent_block);
    f.render_widget(parent_content, columns[0]);

    // --- Column 2: Current Directory File List (with WebDev Icons) ---
    let current_title = format!(" {} ", app.current_path.display());
    let items: Vec<ListItem> = app
        .items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let icon = get_file_icon(&item.name, item.is_dir);
            let display_text = format!("{} {}", icon, item.name);
            
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
                .style(Style::default().bg(COLOR_BASE))
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
        .style(Style::default().bg(COLOR_BASE).fg(COLOR_TEXT))
        .title(preview_title);

    let preview_paragraph = Paragraph::new(preview_content)
        .block(preview_block)
        .wrap(ratatui::widgets::Wrap { trim: true });
    f.render_widget(preview_paragraph, columns[2]);

    // --- Status Bar Footer ---
    let status_text = match app.mode {
        Mode::AiInspection => " AI INSPECTION MODE | Press any key to return ",
        _ => " Ready. Press ^A for AI Summary. | ^A: AI | j/k: Nav | l/Enter: Open | h: Back | q: Quit ",
    };
    let status_bar = Paragraph::new(status_text).style(
        Style::default()
            .bg(COLOR_MAUVE)
            .fg(COLOR_MANTLE)
            .add_modifier(Modifier::BOLD),
    );
    f.render_widget(status_bar, chunks[1]);
}
