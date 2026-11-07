use crate::{
    display::{clean, colors_enabled, name_width},
    manager::{Manager, Row as ProfileRow},
};
use anyhow::{Result, bail};
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Margin},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState, Wrap},
};
use std::{
    io::{self, IsTerminal},
    time::Duration,
};

struct Theme {
    accent: Style,
    muted: Style,
    running: Style,
    error: Style,
    selected: Style,
}
impl Theme {
    fn new() -> Self {
        if colors_enabled() {
            Self {
                accent: Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
                muted: Style::default().fg(Color::DarkGray),
                running: Style::default().fg(Color::Green),
                error: Style::default().fg(Color::Red),
                selected: Style::default()
                    .bg(Color::DarkGray)
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            }
        } else {
            Self {
                accent: Style::default().add_modifier(Modifier::BOLD),
                muted: Style::default(),
                running: Style::default(),
                error: Style::default(),
                selected: Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD),
            }
        }
    }
}

fn draw(
    frame: &mut Frame,
    rows: &[ProfileRow],
    state: &mut TableState,
    message: Option<&str>,
    theme: &Theme,
) {
    let area = frame.area().inner(Margin {
        horizontal: 2,
        vertical: 1,
    });
    if area.height < 13 || area.width < 24 {
        frame.render_widget(
            Paragraph::new("Enlarge the terminal to view profiles.  q quit")
                .wrap(Wrap { trim: true }),
            area,
        );
        return;
    }
    let [title, body, detail, footer] = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(3),
        Constraint::Length(5),
        Constraint::Length(2),
    ])
    .areas(area);
    let running = rows.iter().filter(|row| row.pid.is_some()).count();
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(vec![
                Span::styled("pofman", theme.accent),
                Span::styled("  /  SSH tunnels", theme.muted),
            ]),
            Line::from(Span::styled(
                format!("{} profiles   ·   {running} running", rows.len()),
                theme.muted,
            )),
        ]),
        title,
    );

    let show_pid = body.width >= 42;
    let show_command = body.width >= 85;
    let mut widths = vec![
        Constraint::Length(name_width(rows, (body.width as usize / 3).clamp(7, 28)) as u16),
        Constraint::Length(10),
    ];
    let mut headers = vec!["PROFILE", "STATUS"];
    if show_pid {
        widths.push(Constraint::Length(7));
        headers.push("PID");
    }
    if show_command {
        widths.push(Constraint::Min(0));
        headers.push("COMMAND");
    }
    let table_rows = rows.iter().map(|profile| {
        let mut cells = vec![
            Cell::from(clean(&profile.name)),
            Cell::from(profile.status()).style(if profile.pid.is_some() {
                theme.running
            } else {
                theme.muted
            }),
        ];
        if show_pid {
            cells.push(
                Cell::from(profile.pid.map_or_else(|| "—".into(), |p| p.to_string()))
                    .style(theme.muted),
            );
        }
        if show_command {
            cells.push(Cell::from(clean(&profile.command)).style(theme.muted));
        }
        Row::new(cells)
    });
    let table = Table::new(table_rows, widths)
        .header(Row::new(headers).style(theme.muted).bottom_margin(1))
        .column_spacing(2)
        .row_highlight_style(theme.selected)
        .highlight_symbol("› ");
    frame.render_stateful_widget(table, body, state);
    if rows.is_empty() {
        frame.render_widget(
            Paragraph::new(
                "No profiles yet.\nAdd [[profiles]] with name and command to config.toml.",
            )
            .style(theme.muted)
            .wrap(Wrap { trim: true }),
            body,
        );
    }
    let command = state
        .selected()
        .and_then(|i| rows.get(i))
        .map(|row| clean(&row.command))
        .unwrap_or_else(|| "Select a profile to inspect its command.".into());
    frame.render_widget(
        Paragraph::new(command).wrap(Wrap { trim: false }).block(
            Block::default()
                .borders(Borders::TOP)
                .border_style(theme.muted)
                .title(Line::from(Span::styled(" COMMAND ", theme.muted))),
        ),
        detail,
    );
    let action = state
        .selected()
        .and_then(|i| rows.get(i))
        .map_or(
            "toggle",
            |row| if row.pid.is_some() { "stop" } else { "start" },
        );
    let mut lines = vec![Line::from(vec![
        Span::styled("↑↓", theme.accent),
        Span::styled(" move    ", theme.muted),
        Span::styled("space", theme.accent),
        Span::styled(format!(" {action}    "), theme.muted),
        Span::styled("q", theme.accent),
        Span::styled(" quit", theme.muted),
    ])];
    if let Some(message) = message {
        lines.push(Line::from(Span::styled(clean(message), theme.error)));
    }
    frame.render_widget(Paragraph::new(lines), footer);
}

pub fn run(manager: &Manager) -> Result<()> {
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        bail!("Interactive mode requires a terminal; use `pofman list`");
    }
    let theme = Theme::new();
    ratatui::run(|terminal| -> Result<()> {
        let mut state = TableState::default().with_selected(0);
        let mut rows = Vec::new();
        let mut message: Option<String> = None;
        loop {
            match manager.rows() {
                Ok(updated) => {
                    rows = updated;
                }
                Err(error) => message = Some(format!("{error:#}")),
            }
            state.select(if rows.is_empty() {
                None
            } else {
                Some(state.selected().unwrap_or(0).min(rows.len() - 1))
            });
            terminal.draw(|frame| draw(frame, &rows, &mut state, message.as_deref(), &theme))?;
            if !event::poll(Duration::from_millis(500))? {
                continue;
            }
            let Event::Key(key) = event::read()? else {
                continue;
            };
            if key.kind != KeyEventKind::Press {
                continue;
            }
            message = None;
            match key.code {
                KeyCode::Char('q') | KeyCode::Esc => break,
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => break,
                KeyCode::Up | KeyCode::Char('k') => state.select_previous(),
                KeyCode::Down | KeyCode::Char('j') => state.select_next(),
                KeyCode::Home => state.select_first(),
                KeyCode::End => state.select_last(),
                KeyCode::Char(' ') => {
                    if let Some(row) = state.selected().and_then(|i| rows.get(i)) {
                        let result = if row.pid.is_some() {
                            manager.stop(&row.name)
                        } else {
                            manager.start(&row.name)
                        };
                        if let Err(error) = result {
                            message = Some(format!("{error:#}"));
                        }
                    }
                }
                _ => {}
            }
        }
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};

    #[test]
    fn responsive_layout_keeps_status_details_and_keys_visible() {
        let rows = vec![ProfileRow {
            name: "数据库".into(),
            command: "ssh -L 5433:localhost:5432 server".into(),
            pid: Some(1234),
        }];
        for width in [38, 80, 120] {
            let mut terminal = Terminal::new(TestBackend::new(width, 24)).unwrap();
            let mut selected = TableState::default().with_selected(0);
            terminal
                .draw(|frame| draw(frame, &rows, &mut selected, None, &Theme::new()))
                .unwrap();
            let buffer = terminal.backend().buffer();
            let screen = (0..24)
                .map(|y| {
                    (0..width)
                        .map(|x| buffer[(x, y)].symbol())
                        .collect::<String>()
                })
                .collect::<Vec<_>>()
                .join("\n");
            // Wide glyphs occupy a second buffer cell containing a blank.
            assert!(screen.replace(' ', "").contains("数据库"));
            assert!(screen.contains("running"));
            assert!(screen.contains("COMMAND"));
            assert!(screen.contains("space stop"));
            assert!(!screen.contains("Config:"));
        }
    }
}
