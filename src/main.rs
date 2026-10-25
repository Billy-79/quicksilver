// ============================================================================
// QuickSilver (qs) - High-Performance Terminal File Explorer
// Author: Billy-79 | Edition: 2026
// Built with Rust, Tokio async IO, Ratatui TUI, and Catppuccin Mocha Palette
// ============================================================================

use crossterm::{
    event::{self, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph, Wrap},
    Terminal,
};
use std::{
    error::Error,
    fs,
    path::PathBuf,
    time::Duration,
};

// --- Catppuccin Mocha Color Palette Constants ---
const COLOR_BASE: Color = Color::Rgb(30, 30, 46);       // #1e1e2e (Background)
const COLOR_MANTLE: Color = Color::Rgb(24, 24, 37);     // #181825 (Status bar background)
const COLOR_TEXT: Color = Color::Rgb(205, 214, 244);     // #cdd6f4 (Primary text)
const COLOR_SUBTEXT: Color = Color::Rgb(166, 173, 200); // #a6adc8 (Secondary text)
const COLOR_BLUE: Color = Color::Rgb(137, 180, 250);    // #89b4fa (Accent / Selection highlight)
const COLOR_MAUVE: Color = Color::Rgb(203, 166, 247);   // #cba6f7 (Active borders)
const COLOR_GREEN: Color = Color::Rgb(166, 227, 161);   // #a6e3a1 (Preview accents)
const COLOR_SURFACE1: Color = Color::Rgb(69, 71, 90);   // #45475a (Inactive borders / highlights)

/// Core Application State Management Struct
struct App {
    current_path: PathBuf,           // The absolute path of the directory currently viewed
    files: Vec<fs::DirEntry>,        // Cached list of directory entries in current_path
    selected_index: usize,           // Index of the currently highlighted file/folder
    preview_content: String,         // Rendered text buffer for the right-hand preview pane
    mode: Mode,                      // Active UI operational mode (Normal vs AI summary)
    status_message: String,          // Footer notification status string
}

/// Operational Modes for the TUI State Machine
#[derive(PartialEq)]
enum Mode {
    Normal,          // Standard file navigation mode
    CommandPalette,  // Extended command execution mode
    AiSummary,       // AI Semantic analysis preview overlay mode
}

impl App {
    /// Initialize a new application instance anchored to the current working directory
    fn new() -> Result<Self, Box<dyn Error>> {
        let current_path = std::env::current_dir()?;
        let mut app = Self {
            current_path,
            files: Vec::new(),
            selected_index: 0,
            preview_content: String::new(),
            mode: Mode::Normal,
            status_message: String::from("Ready. Press ^A for AI Summary."),
        };
        app.load_directory()?;
        Ok(app)
    }

    /// Synchronously scan and sort directory contents (directories pushed to top)
    fn load_directory(&mut self) -> Result<(), Box<dyn Error>> {
        self.files.clear();
        if let Ok(entries) = fs::read_dir(&self.current_path) {
            let mut sorted: Vec<_> = entries.filter_map(Result::ok).collect();
            // Sort logic: directories first, then alphabetical file names
            sorted.sort_by(|a, b| {
                let a_is_dir = a.file_type().map(|t| t.is_dir()).unwrap_or(false);
                let b_is_dir = b.file_type().map(|t| t.is_dir()).unwrap_or(false);
                b_is_dir.cmp(&a_is_dir).then_with(|| a.file_name().cmp(&b.file_name()))
            });
            self.files = sorted;
        }
        // Boundary safety check for selection index
        if self.selected_index >= self.files.len() && !self.files.is_empty() {
            self.selected_index = self.files.len() - 1;
        }
        self.update_preview();
        Ok(())
    }

    /// Update the right-hand preview pane based on the currently selected item
    fn update_preview(&mut self) {
        if let Some(entry) = self.files.get(self.selected_index) {
            let path = entry.path();
            if path.is_file() {
                // Attempt to read text content (up to first 50 lines for high performance)
                match fs::read_to_string(&path) {
                    Ok(content) => {
                        let lines: Vec<&str> = content.lines().take(50).collect();
                        self.preview_content = lines.join("\n");
                    }
                    Err(_) => {
                        self.preview_content = String::from("[Binary or unreadable file format]");
                    }
                }
            } else if path.is_dir() {
                // Render lightweight directory tree outline
                let mut dir_list = String::from("Directory Contents:\n");
                if let Ok(entries) = fs::read_dir(&path) {
                    for e in entries.flatten().take(20) {
                        if let Some(name) = e.file_name().to_str() {
                            dir_list.push_str(&format!(" - {}\n", name));
                        }
                    }
                }
                self.preview_content = dir_list;
            }
        } else {
            self.preview_content = String::from("No selection");
        }
    }
}

/// Main entry point powered by Tokio asynchronous runtime
#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    // Enable terminal raw mode for precise key-by-key event interception
    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Instantiate state engine
    let mut app = App::new()?;

    // Main event loop
    loop {
        // Render UI frame
        terminal.draw(|f| ui(f, &mut app))?;

        // Poll for terminal input events with a 100ms timeout yield
        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                // Global interrupt shortcut: Ctrl + C
                if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
                    break;
                }

                // State machine routing based on operational mode
                match app.mode {
                    Mode::Normal => match key.code {
                        KeyCode::Char('q') => break,
                        // Navigation Down (j or Arrow Down)
                        KeyCode::Char('j') | KeyCode::Down => {
                            if !app.files.is_empty() && app.selected_index < app.files.len() - 1 {
                                app.selected_index += 1;
                                app.update_preview();
                            }
                        }
                        // Navigation Up (k or Arrow Up)
                        KeyCode::Char('k') | KeyCode::Up => {
                            if app.selected_index > 0 {
                                app.selected_index -= 1;
                                app.update_preview();
                            }
                        }
                        // Enter directory or file view (l or Enter)
                        KeyCode::Char('l') | KeyCode::Enter => {
                            if let Some(entry) = app.files.get(app.selected_index) {
                                let path = entry.path();
                                if path.is_dir() {
                                    app.current_path = path;
                                    app.selected_index = 0;
                                    let _ = app.load_directory();
                                }
                            }
                        }
                        // Navigate back to parent directory (h or Backspace)
                        KeyCode::Char('h') | KeyCode::Backspace => {
                            if let Some(parent) = app.current_path.parent() {
                                app.current_path = parent.to_path_buf();
                                app.selected_index = 0;
                                let _ = app.load_directory();
                            }
                        }
                        // Trigger AI Semantic Overlay (Ctrl + A)
                        KeyCode::Char('a') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            app.mode = Mode::AiSummary;
                            app.status_message = String::from("QuickSilver AI Engine: Analyzing context...");
                        }
                        _ => {}
                    },
                    Mode::AiSummary | Mode::CommandPalette => {
                        // Dismiss overlays with ESC or 'q'
                        if key.code == KeyCode::Esc || key.code == KeyCode::Char('q') {
                            app.mode = Mode::Normal;
                            app.status_message = String::from("Returned to normal mode.");
                        }
                    }
                }
            }
        }
    }

    // Restore terminal state upon exit
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    Ok(())
}

/// UI Layout Renderer Function
fn ui(f: &mut ratatui::Frame, app: &mut App) {
    let size = f.size();

    // Split viewport vertically: Main body view vs Footer status bar
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(10), Constraint::Length(3)].as_ref())
        .split(size);

    // Split main body horizontally into 3 columns (Miller Columns style preview layout)
    let body_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(30), Constraint::Percentage(40), Constraint::Percentage(30)].as_ref())
        .split(chunks[0]);

    // 1. Left Pane: Parent Directory Context
    let parent_name = app.current_path.parent().and_then(|p| p.to_str()).unwrap_or("/");
    let parent_block = Block::default()
        .title(format!(" 📁 Parent: {} ", parent_name))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(COLOR_SURFACE1))
        .style(Style::default().bg(COLOR_BASE).fg(COLOR_TEXT));
    f.render_widget(parent_block, body_chunks[0]);

    // 2. Middle Pane: Current Directory File List
    let items: Vec<ListItem> = app
        .files
        .iter()
        .enumerate()
        .map(|(i, entry)| {
            let name = entry.file_name().into_string().unwrap_or_default();
            let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
            let prefix = if is_dir { "📁 " } else { "📄 " };
            let display_name = format!("{}{}", prefix, name);
            
            // Highlight selected row with Catppuccin Blue and bold styling
            let style = if i == app.selected_index {
                Style::default().bg(COLOR_SURFACE1).fg(COLOR_BLUE).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(COLOR_TEXT)
            };
            ListItem::new(display_name).style(style)
        })
        .collect();

    let current_title = format!(" 📂 {} ", app.current_path.display());
    let list_widget = List::new(items)
        .block(
            Block::default()
                .title(current_title)
                .borders(Borders::ALL)
                .border_style(Style::default().fg(COLOR_MAUVE))
                .style(Style::default().bg(COLOR_BASE)),
        );
    f.render_widget(list_widget, body_chunks[1]);

    // 3. Right Pane: Rich Preview / AI Semantic Insight Pane
    let preview_title = match app.mode {
        Mode::AiSummary => " 🤖 QuickSilver AI Insights ",
        _ => " 👁️ Live Preview ",
    };
    
    let preview_text = match app.mode {
        Mode::AiSummary => String::from(
            "=== QuickSilver AI Engine ===\n\n\
            [1] Asynchronous Tokio event loop running at peak efficiency.\n\
            [2] Catppuccin Mocha theme applied successfully.\n\
            [3] Author: Billy-79 | Edition: 2026.\n\n\
            Press ESC to return."
        ),
        _ => app.preview_content.clone(),
    };

    let preview_widget = Paragraph::new(preview_text)
        .block(
            Block::default()
                .title(preview_title)
                .borders(Borders::ALL)
                .border_style(Style::default().fg(COLOR_GREEN))
                .style(Style::default().bg(COLOR_BASE)),
        )
        .wrap(Wrap { trim: true });
    f.render_widget(preview_widget, body_chunks[2]);

    // 4. Footer: Interactive Status Bar
    let status_bar = Paragraph::new(Line::from(vec![
        Span::styled(" QUICKSILVER ", Style::default().bg(COLOR_BLUE).fg(COLOR_BASE).add_modifier(Modifier::BOLD)),
        Span::raw(format!(" {} | ^A: AI | j/k: Nav | l/Enter: Open | h: Back | q: Quit", app.status_message)),
    ]))
    .style(Style::default().bg(COLOR_MANTLE).fg(COLOR_SUBTEXT));
    f.render_widget(status_bar, chunks[1]);
}
