//! Minimal ratatui dashboard.
//!
//! Layout:
//!
//! ```text
//! +----------------------------------------------------+
//! |  status:  connected   slot 309845112   42 swaps/s  |
//! +----------------+-----------------+-----------------+
//! |  Trade Tape    |  Top Spreads    |  Sandwiches     |
//! |  ...           |  ...            |  ...            |
//! +----------------+-----------------+-----------------+
//! ```
//!
//! Press `q` or `Esc` to quit.

use std::{io::Stdout, time::Duration};

use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use mev_radar_arb::ArbEvent;
use mev_radar_dex::SwapEvent;
use mev_radar_sandwich::SandwichEvent;
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Modifier, Style},
    widgets::{Block, Borders, List, ListItem, Paragraph},
    Terminal,
};

#[derive(Debug, Default)]
pub struct DashboardState {
    pub status_line: String,
    pub recent_swaps: Vec<SwapEvent>,
    pub top_spreads: Vec<ArbEvent>,
    pub recent_sandwiches: Vec<SandwichEvent>,
}

pub struct Dashboard {
    term: Terminal<CrosstermBackend<Stdout>>,
}

impl Dashboard {
    pub fn enter() -> anyhow::Result<Self> {
        enable_raw_mode()?;
        let mut stdout = std::io::stdout();
        execute!(stdout, EnterAlternateScreen)?;
        let backend = CrosstermBackend::new(stdout);
        let term = Terminal::new(backend)?;
        Ok(Self { term })
    }

    pub fn render(&mut self, state: &DashboardState) -> anyhow::Result<()> {
        self.term.draw(|f| {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(3), Constraint::Min(1)])
                .split(f.area());

            let status = Paragraph::new(state.status_line.clone())
                .block(Block::default().borders(Borders::ALL).title("status"));
            f.render_widget(status, chunks[0]);

            let cols = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([
                    Constraint::Percentage(40),
                    Constraint::Percentage(30),
                    Constraint::Percentage(30),
                ])
                .split(chunks[1]);

            let swaps_items: Vec<ListItem> = state
                .recent_swaps
                .iter()
                .rev()
                .take(20)
                .map(|s| {
                    ListItem::new(format!(
                        "{} {} {}->{} {}@{}",
                        s.dex.as_str(),
                        &s.signature[..s.signature.len().min(8)],
                        short(&s.mint_in),
                        short(&s.mint_out),
                        s.amount_in,
                        s.amount_out,
                    ))
                })
                .collect();
            let swaps = List::new(swaps_items)
                .block(Block::default().borders(Borders::ALL).title("trade tape"));
            f.render_widget(swaps, cols[0]);

            let arb_items: Vec<ListItem> = state
                .top_spreads
                .iter()
                .map(|a| {
                    ListItem::new(format!(
                        "{}/{}  {}bps  buy {} sell {}",
                        short(a.pair.base()),
                        short(a.pair.quote()),
                        a.spread_bps,
                        a.buy_dex.as_str(),
                        a.sell_dex.as_str(),
                    ))
                })
                .collect();
            let arb = List::new(arb_items).block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("top spreads")
                    .title_style(Style::default().add_modifier(Modifier::BOLD)),
            );
            f.render_widget(arb, cols[1]);

            let sand_items: Vec<ListItem> = state
                .recent_sandwiches
                .iter()
                .rev()
                .take(20)
                .map(|s| {
                    ListItem::new(format!(
                        "slot {} atk {}.. extracted {}",
                        s.slot,
                        &s.attacker[..s.attacker.len().min(6)],
                        s.extracted_amount,
                    ))
                })
                .collect();
            let sand = List::new(sand_items)
                .block(Block::default().borders(Borders::ALL).title("sandwiches"));
            f.render_widget(sand, cols[2]);
        })?;

        Ok(())
    }

    /// Non-blocking poll for a quit key. Returns `true` to exit.
    pub fn should_quit(&self) -> anyhow::Result<bool> {
        if event::poll(Duration::from_millis(50))?
            && let Event::Key(k) = event::read()?
            && k.kind == KeyEventKind::Press
            && matches!(k.code, KeyCode::Char('q') | KeyCode::Esc)
        {
            return Ok(true);
        }
        Ok(false)
    }
}

impl Drop for Dashboard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(self.term.backend_mut(), LeaveAlternateScreen);
        let _ = self.term.show_cursor();
    }
}

fn short(s: &str) -> String {
    if s.len() <= 6 {
        s.to_string()
    } else {
        format!("{}…", &s[..6])
    }
}
