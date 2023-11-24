use crate::query::DenseTagTable;
use crossterm::{
    event::{self, KeyCode, KeyEventKind},
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    ExecutableCommand,
};
use ratatui::{
    prelude::{Backend, Constraint, CrosstermBackend, Direction, Layout, Terminal},
    widgets::{Block, Borders, List, ListItem, Padding},
};
use std::io::stdout;

struct State {
    table: DenseTagTable,
}

pub(crate) fn start(table: DenseTagTable) -> std::io::Result<()> {
    stdout().execute(EnterAlternateScreen)?;
    enable_raw_mode()?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;
    terminal.clear()?;
    let mut app = State { table };
    run_app(&mut terminal, &mut app)?;
    // Clean up.
    stdout().execute(LeaveAlternateScreen)?;
    disable_raw_mode()?;
    return Ok(());
}

fn run_app<B: Backend>(terminal: &mut Terminal<B>, state: &mut State) -> std::io::Result<()> {
    let (tagwidth, maxfilelen) = {
        (
            usize::max(
                state
                    .table
                    .tags()
                    .iter()
                    .map(|t| t.len())
                    .max()
                    .unwrap_or(0),
                32,
            ) as u16,
            state
                .table
                .files()
                .iter()
                .map(|f| f.len())
                .max()
                .unwrap_or(0) as u16,
        )
    };
    // Main application loop
    loop {
        // Update state
        // Render the UI
        terminal.draw(|f| {
            let hlayout = Layout::default()
                .direction(Direction::Horizontal)
                .constraints(vec![
                    Constraint::Length(tagwidth),
                    Constraint::Min(maxfilelen),
                ])
                .split(f.size());
            let vlayout = Layout::default()
                .direction(Direction::Vertical)
                .constraints(vec![
                    Constraint::Percentage(90),
                    Constraint::Percentage(5),
                    Constraint::Length(1),
                ])
                .split(hlayout[1]);
            let tagblock = hlayout[0];
            let fileblock = vlayout[0];
            let echoblock = vlayout[1];
            let cmdblock = vlayout[2];
            f.render_widget(
                List::new(
                    state
                        .table
                        .tags()
                        .iter()
                        .map(|t| ListItem::new(t.clone()))
                        .collect::<Vec<_>>(),
                )
                .block(
                    Block::new()
                        .borders(Borders::ALL)
                        .padding(Padding::horizontal(2)),
                ),
                tagblock,
            );
            f.render_widget(
                List::new(
                    state
                        .table
                        .files()
                        .iter()
                        .map(|f| ListItem::new(f.clone()))
                        .collect::<Vec<_>>(),
                )
                .block(
                    Block::new()
                        .borders(Borders::ALL)
                        .padding(Padding::horizontal(2)),
                ),
                fileblock,
            );
        })?;
        // Check for user input every 250 milliseconds
        if event::poll(std::time::Duration::from_millis(250))? {
            // If a key event occurs, handle it
            if let crossterm::event::Event::Key(key) = crossterm::event::read()? {
                if key.kind == KeyEventKind::Press {
                    match key.code {
                        // KeyCode::Char('j') => counter += 1,
                        // KeyCode::Char('k') => counter -= 1,
                        KeyCode::Char('q') => break,
                        _ => {}
                    }
                }
            }
        }
    }
    return Ok(());
}
