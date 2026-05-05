mod api;
mod app;
mod ui;

use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};
use std::io;

fn main() -> Result<()> {
    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run_app(&mut terminal);

    // Restore terminal
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    if let Err(e) = result {
        eprintln!("Error: {}", e);
    }

    Ok(())
}

fn run_app(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<()> {
    let mut app = app::App::new();

    loop {
        terminal.draw(|f| ui::render(f, &app))?;

        // Poll for events with 250ms timeout
        if event::poll(std::time::Duration::from_millis(250))? {
            if let Event::Key(key) = event::read()? {
                match (key.code, key.modifiers) {
                    (KeyCode::Char('q'), _)
                    | (KeyCode::Esc, _)
                    | (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                        break;
                    }
                    (KeyCode::Char('r'), _) => {
                        app.status_msg.clear();
                        app.refresh();
                    }
                    (KeyCode::Char('1'), _) => app.tab = 0,
                    (KeyCode::Char('2'), _) => app.tab = 1,
                    (KeyCode::Char('3'), _) => app.tab = 2,
                    (KeyCode::Char('4'), _) => app.tab = 3,
                    _ => {}
                }
            }
        }

        // Auto-refresh
        if app.last_refresh.elapsed() >= app.refresh_interval {
            app.status_msg.clear();
            app.refresh();
        }
    }

    Ok(())
}
