use crate::{
    interactive::{InteractiveSession, State},
    query::DenseTagTable,
};
use crossterm::{
    event::{self, KeyCode, KeyEvent, KeyEventKind},
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    ExecutableCommand,
};
use ratatui::{
    prelude::{Backend, Constraint, CrosstermBackend, Direction, Layout, Terminal},
    text::{Line, Text},
    widgets::{
        Block, Borders, Padding, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState,
    },
    Frame,
};
use std::io::stdout;

/// Count digits in the integer as written in base 10.
fn count_digits(mut num: usize) -> u8 {
    if num == 0 {
        return 1;
    }
    let mut digits = 0u8;
    while num > 0 {
        digits += 1;
        num /= 10;
    }
    digits
}

struct TuiApp {
    session: InteractiveSession,
    scroll: usize,
    scrollstate: ScrollbarState,
    frameheight: usize,
    file_index_width: u8,
}

impl TuiApp {
    fn init(table: DenseTagTable) -> Self {
        let ntags = table.tags().len();
        let nfiles = table.files().len();
        TuiApp {
            session: InteractiveSession::init(table),
            scroll: 0,
            scrollstate: ScrollbarState::new(ntags),
            frameheight: 0,
            file_index_width: count_digits(nfiles - 1),
        }
    }

    fn can_scroll(&self) -> bool {
        self.session.taglist().len() + 1 > self.frameheight
    }

    fn keyevent(&mut self, evt: KeyEvent) {
        match evt.kind {
            KeyEventKind::Press | KeyEventKind::Repeat => match evt.code {
                KeyCode::Char(c) => {
                    self.session.command_mut().push(c);
                    self.session.stop_autocomplete();
                }
                KeyCode::Backspace => {
                    self.session.command_mut().pop();
                    self.session.stop_autocomplete();
                }
                KeyCode::Enter => {
                    self.session.process_input();
                    if let State::ListsUpdated = self.session.state() {
                        self.scroll = 0;
                        self.scrollstate = self
                            .scrollstate
                            .content_length(self.session.taglist().len());
                        self.session.set_state(State::Default);
                    }
                }
                KeyCode::Esc => {
                    self.session.command_mut().clear();
                    self.session.stop_autocomplete();
                }
                KeyCode::Up if self.can_scroll() => {
                    self.scroll = self.scroll.saturating_sub(1);
                    self.scrollstate = self.scrollstate.position(self.scroll);
                }
                KeyCode::Down if self.can_scroll() => {
                    self.scroll = self.scroll.saturating_add(1);
                    self.scrollstate = self.scrollstate.position(self.scroll);
                }
                KeyCode::Tab => self.session.autocomplete(),
                _ => {}
            },
            KeyEventKind::Release => {} // Do nothing.
        }
    }
}

/// Start the interactive TUI mode of ftag.
pub fn start(table: DenseTagTable) -> std::io::Result<()> {
    stdout().execute(EnterAlternateScreen)?;
    enable_raw_mode()?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;
    terminal.clear()?;
    let mut app = TuiApp::init(table);
    run_app(&mut terminal, &mut app)?;
    // Clean up.
    stdout().execute(LeaveAlternateScreen)?;
    disable_raw_mode()?;
    Ok(())
}

fn run_app<B: Backend>(terminal: &mut Terminal<B>, app: &mut TuiApp) -> std::io::Result<()> {
    const DELAY: u64 = 20;
    // Main application loop. The terminal is only redrawn when an
    // event is registered, so it is necessary to draw it once at
    // first.
    terminal.draw(|f| render(f, app))?;
    loop {
        // Poll events to see if redraw needed.
        if event::poll(std::time::Duration::from_millis(DELAY))? {
            // If a key event occurs, handle it
            if let event::Event::Key(key) = crossterm::event::read()? {
                app.keyevent(key);
            }
            terminal.draw(|f| render(f, app))?;
        }
        if let State::Exit = app.session.state() {
            break;
        };
    }
    Ok(())
}

/// Given `prev` and `curr`, this function removes the common prefix
/// from `curr` and returns the resulting string as part of a
/// tuple. The first element of the tuple is the length of the prefix
/// that was trimmed.
fn remove_common_prefix<'a>(prev: &str, curr: &'a str) -> (usize, &'a str) {
    let stop = usize::min(prev.len(), curr.len());
    let mut start = 0usize;
    for (i, (l, r)) in prev.chars().zip(curr.chars()).enumerate() {
        if !(i < stop && l == r) {
            break;
        }
        if l == std::path::MAIN_SEPARATOR {
            start = i;
        }
    }
    (start, &curr[start..])
}

fn render(f: &mut Frame, app: &mut TuiApp) {
    const TAGWIDTH_PERCENT: u16 = 20;
    app.frameheight = f.size().height as usize;
    let hlayout = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(vec![
            Constraint::Percentage(TAGWIDTH_PERCENT),
            Constraint::Percentage(100 - TAGWIDTH_PERCENT),
        ])
        .split(f.size());
    let rblocks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(vec![
            Constraint::Max(1001),
            Constraint::Min(4),
            Constraint::Length(2),
        ])
        .split(hlayout[1]);
    let lblocks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(vec![Constraint::Max(1000), Constraint::Length(6)])
        .split(hlayout[0]);
    let tagblock = lblocks[0];
    let filterblock = lblocks[1];
    let fileblock = rblocks[0];
    let echoblock = rblocks[1];
    let cmdblock = rblocks[2];
    // Tags.
    f.render_widget(
        Paragraph::new(
            app.session
                .taglist()
                .iter()
                .map(|t| Line::from(t.clone()))
                .collect::<Vec<_>>(),
        )
        .block(
            Block::new()
                .borders(Borders::TOP | Borders::RIGHT)
                .padding(Padding::horizontal(4)),
        )
        .scroll((app.scroll as u16, 0)),
        tagblock,
    );
    // Scroll bar.
    f.render_stateful_widget(
        Scrollbar::default()
            .orientation(ScrollbarOrientation::VerticalLeft)
            .begin_symbol(Some("↑"))
            .end_symbol(Some("↓")),
        tagblock,
        &mut app.scrollstate,
    );
    {
        let mut prevfile: &str = "";
        f.render_widget(
            Paragraph::new(
                app.session
                    .filelist()
                    .iter()
                    .enumerate()
                    .map(|(filecounter, file)| {
                        let out = format!(
                            "[{}] {}",
                            {
                                let nspaces = app.file_index_width - count_digits(filecounter);
                                format!("{}{filecounter}", " ".repeat(nspaces as usize))
                            },
                            {
                                let (space, trimmed) = remove_common_prefix(prevfile, file);
                                format!("{}{}", ".".repeat(space), trimmed)
                            }
                        );
                        prevfile = file;
                        Line::from(out)
                    })
                    .collect::<Vec<_>>(),
            )
            .block(
                Block::new()
                    .borders(Borders::TOP)
                    .padding(Padding::horizontal(2)),
            ),
            fileblock,
        );
    }
    f.render_widget(
        Paragraph::new(Text::from(app.session.echo())).block(
            Block::new()
                .padding(Padding::horizontal(2))
                .borders(Borders::TOP),
        ),
        echoblock,
    );
    f.render_widget(
        Paragraph::new(Text::from(app.session.filter_str())).block(
            Block::new()
                .padding(Padding::horizontal(2))
                .borders(Borders::TOP | Borders::RIGHT),
        ),
        filterblock,
    );
    f.render_widget(
        Paragraph::new(Text::from(format!(">>> {}█", app.session.command())))
            .block(Block::new().borders(Borders::TOP)),
        cmdblock,
    );
}
