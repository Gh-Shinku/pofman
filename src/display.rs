use crate::manager::Row;
use anyhow::Result;
use crossterm::{
    queue,
    style::{Attribute, Color, ContentStyle, PrintStyledContent},
    terminal,
};
use std::{
    env,
    io::{self, IsTerminal, Write},
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

pub fn colors_enabled() -> bool {
    !env::var_os("NO_COLOR").is_some_and(|value| !value.is_empty())
        && env::var("TERM").as_deref() != Ok("dumb")
}

pub fn clean(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}

pub fn truncate(text: &str, width: usize) -> String {
    let text = clean(text);
    if text.width() <= width {
        return text;
    }
    if width == 0 {
        return String::new();
    }
    let mut used = 0;
    let mut output = String::new();
    for c in text.chars() {
        let size = c.width().unwrap_or(0);
        if used + size > width - 1 {
            break;
        }
        output.push(c);
        used += size;
    }
    output.push('…');
    output
}

fn padded(text: &str, width: usize) -> String {
    let value = truncate(text, width);
    format!("{value}{}", " ".repeat(width.saturating_sub(value.width())))
}

pub fn name_width(rows: &[Row], max: usize) -> usize {
    rows.iter()
        .map(|row| clean(&row.name).width())
        .max()
        .unwrap_or(7)
        .clamp(7, max.max(7))
}

pub fn list(rows: &[Row]) -> Result<()> {
    let terminal = io::stdout().is_terminal();
    let color = terminal && colors_enabled();
    let width = terminal
        .then(|| terminal::size().ok().map(|(w, _)| w as usize))
        .flatten();
    let mut stdout = io::stdout().lock();
    let mut print = |text: String, tint: Option<Color>, bold: bool| -> io::Result<()> {
        let mut style = ContentStyle::default();
        if color {
            style.foreground_color = tint;
        }
        if terminal && bold {
            style.attributes.set(Attribute::Bold);
        }
        queue!(stdout, PrintStyledContent(style.apply(text)))
    };
    let running = rows.iter().filter(|row| row.pid.is_some()).count();
    if terminal {
        print("\n  pofman".into(), Some(Color::Cyan), true)?;
        print(
            format!("  {} profiles · {running} running\n\n", rows.len()),
            Some(Color::DarkGrey),
            false,
        )?;
    }
    if rows.is_empty() {
        print("  No profiles yet.\n  Add [[profiles]] entries with name and command to config.toml.\n".into(), Some(Color::DarkGrey), false)?;
    } else {
        let name = name_width(rows, width.map_or(40, |w| (w / 4).clamp(7, 28)));
        // Compact terminals keep the essentials; full commands remain intact in pipes.
        let show_pid = width.is_none_or(|w| w >= 40);
        let show_command = width.is_none_or(|w| w >= 65);
        let prefix = if terminal { "  " } else { "" };
        let header = format!(
            "{prefix}{}  {}{}{}\n",
            padded("PROFILE", name),
            padded("STATUS", 9),
            if show_pid {
                format!("  {}", padded("PID", 7))
            } else {
                String::new()
            },
            if show_command { "  COMMAND" } else { "" }
        );
        print(header, Some(Color::DarkGrey), true)?;
        for row in rows {
            print(format!("{prefix}{}  ", padded(&row.name, name)), None, true)?;
            print(
                padded(row.status(), 9),
                Some(if row.pid.is_some() {
                    Color::Green
                } else {
                    Color::DarkGrey
                }),
                false,
            )?;
            if show_pid {
                print(
                    format!(
                        "  {}",
                        padded(&row.pid.map_or_else(|| "—".into(), |p| p.to_string()), 7)
                    ),
                    Some(Color::DarkGrey),
                    false,
                )?;
            }
            if show_command {
                let command = width.map_or_else(
                    || clean(&row.command),
                    |w| truncate(&row.command, w.saturating_sub(prefix.len() + name + 24)),
                );
                print(format!("  {command}"), Some(Color::DarkGrey), false)?;
            }
            print("\n".into(), None, false)?;
        }
    }
    if terminal {
        print("\n".into(), None, false)?;
    }
    stdout.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncation_uses_terminal_columns_and_removes_control_characters() {
        assert_eq!(truncate("数据库 tunnel", 6), "数据…");
        assert_eq!(truncate("long", 0), "");
        assert_eq!(truncate("line\nnext", 20), "line next");
        assert_eq!(padded("中文", 6).width(), 6);
    }
}
