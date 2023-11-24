use crate::query::DenseTagTable;
use crossterm::{
    event::{self, KeyCode, KeyEventKind},
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    ExecutableCommand,
};
use ratatui::{
    prelude::{Backend, CrosstermBackend, Terminal},
    widgets::Paragraph,
};
use std::io::stdout;

pub(crate) fn start(_table: DenseTagTable) -> std::io::Result<()> {
    let mut terminal = init_term(CrosstermBackend::new(stdout()))?;
    let mut counter = 0;
    // Main application loop
    loop {
        // Render the UI
        terminal.draw(|f| {
            f.render_widget(Paragraph::new(format!("Counter: {counter}")), f.size());
        })?;
        // Check for user input every 250 milliseconds
        if event::poll(std::time::Duration::from_millis(250))? {
            // If a key event occurs, handle it
            if let crossterm::event::Event::Key(key) = crossterm::event::read()? {
                if key.kind == KeyEventKind::Press {
                    match key.code {
                        KeyCode::Char('j') => counter += 1,
                        KeyCode::Char('k') => counter -= 1,
                        KeyCode::Char('q') => break,
                        _ => {}
                    }
                }
            }
        }
    }
    return clean_up();
}

fn init_term<B: Backend>(backend: B) -> std::io::Result<Terminal<B>> {
    stdout().execute(EnterAlternateScreen)?;
    enable_raw_mode()?;
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;
    return Ok(terminal);
}

fn clean_up() -> std::io::Result<()> {
    // Clean up.
    stdout().execute(LeaveAlternateScreen)?;
    disable_raw_mode()?;
    return Ok(());
}
