use chrono::prelude::*;
use log::*;
use std::fmt;
use std::fs::File;
use std::io;
use std::io::prelude::*;
use std::io::Stdout;
use std::process::Command;
use termion::cursor::Goto;
use termion::event::Key;
use termion::input::{MouseTerminal, TermRead};
use termion::raw::{IntoRawMode, RawTerminal};
use termion::screen::AlternateScreen;
use tui::backend::TermionBackend;
use tui::layout::{Alignment, Constraint, Direction, Layout};
use tui::style::{Color, Modifier, Style};
use tui::widgets::{Block, Borders, Paragraph, Row, Table, Text};
use tui::Terminal;

mod message;

struct MessageList {
    list: Vec<message::Message>,
    selected: u16,
}

impl MessageList {
    fn new(list: Vec<message::Message>) -> Self {
        MessageList { list, selected: 0 }
    }

    fn select_next(&mut self) {
        if !self.list.is_empty() && self.selected < self.len() - 1 {
            self.selected += 1;
        }
    }

    fn select_prev(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
        }
    }

    fn select_first(&mut self) {
        self.selected = 0;
    }

    fn select_last(&mut self) {
        self.selected = self.list.len() as u16 - 1;
    }

    fn get_selected(&self) -> Result<&message::Message, failure::Error> {
        if !self.list.is_empty() {
            self.list
                .get(self.selected as usize)
                .ok_or(failure::format_err!("Selected message missing!"))
        } else {
            failure::bail!("Trying to get message from empty list")
        }
    }

    fn len(&self) -> u16 {
        self.list.len() as u16
    }
}

enum AppState {
    Refresh,
    Index,
    View,
    _EditSubject,
    Compose,
    Exit,
}

struct Styles {
    selected: Style,
    header: Style,
    normal: Style,
    subject: Style,
    attachment: Style,
}

struct Tags<'a>(&'a Vec<String>);

impl<'a> fmt::Display for Tags<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut result = String::from("[");
        self.0.iter().for_each(|s| {
            result.push_str(&format!("{},", s));
        });
        result = result.trim_end_matches(',').to_string();
        result.push(']');

        write!(f, "{}", result)
    }
}

// TODO: refactor styles to own struct/configuration object
struct App {
    state: AppState,
    messages: MessageList,
    styles: Styles,
    // selected_att: isize,
    search_term: String,
}

impl App {
    fn new() -> App {
        App {
            state: AppState::Refresh,
            search_term: "tag:inbox".to_string(),
            messages: MessageList::new(vec![]),
            // selected_att: -1,
            styles: Styles {
                selected: Style::default().fg(Color::Yellow).modifier(Modifier::BOLD),
                normal: Style::default().fg(Color::White),
                header: Style::default().fg(Color::Cyan),
                subject: Style::default()
                    .fg(Color::Rgb(255, 255, 255))
                    .modifier(Modifier::BOLD),
                attachment: Style::default().fg(Color::Blue),
            },
        }
    }
}

// TODO: this sits here only as placeholder method. Whole thing should be implemented properly
#[allow(dead_code)]
fn compose(app: &mut App) -> Result<(), failure::Error> {
    debug!("Compose called");

    app.state = AppState::Index;

    let fname = "/tmp/tmp-mail.txt";
    let mut file = File::open(fname)?;

    let _ = Command::new("/usr/bin/nvim")
        .arg(fname)
        .status()
        .expect("Failed to execute 'nvim'");

    let mut body = "".to_string();
    file.read_to_string(&mut body)?;

    let now = Local::now();
    let id_fmt = &format!(
        "<%Y%m%d%H%M%S.%f@localhost:{}>",
        std::process::id().to_string()
    );
    let email = format!(
        "Date: {}\nTo: {}\nSubject: {}\nMessage-Id:{}\nFrom: {}\n\n{}\n",
        now.to_rfc2822(),
        "me",
        "Otsikko",
        now.format(id_fmt),
        "me",
        body
    );
    file.write_all(email.as_bytes())?;
    file.flush()?;

    let _ = Command::new("/usr/bin/notmuch")
        .arg("insert")
        .stdin(file)
        .status()
        .expect("Failed to execute 'notmuch'");

    std::fs::remove_file(fname)?;
    Ok(())
}

fn refresh_index(app: &mut App) -> Result<(), failure::Error> {
    debug!("Refreshing index: {}", &app.search_term);

    if app.search_term.is_empty() {
        app.search_term = "tag:inbox".to_string();
    }

    let messages = message::parse_messages(&app.search_term)?;
    app.messages = MessageList::new(messages);
    app.state = AppState::Index;

    Ok(())
}

// TODO: refactor/split to smaller functions
// TODO: simplify scrolling logic which is awful now
fn show_index(
    app: &mut App,
    terminal: &mut Terminal<TermionBackend<AlternateScreen<MouseTerminal<RawTerminal<Stdout>>>>>,
) -> Result<(), failure::Error> {
    let mut is_input = false;
    let input = &mut String::new();
    let mut scroll = 0;

    loop {
        terminal.hide_cursor()?;
        terminal.draw(|mut f| {
            let view_height = f.size().height - 5;

            if app.messages.selected < scroll {
                scroll -= scroll - app.messages.selected;
            } else if app.messages.selected - scroll >= view_height {
                scroll = app.messages.selected + 1 - view_height;
            }

            let rects = Layout::default()
                .direction(Direction::Vertical)
                .horizontal_margin(1)
                .constraints([Constraint::Length(3), Constraint::Percentage(100)].as_ref())
                .split(f.size());

            let (input_style, search_text) = match is_input {
                true => (app.styles.selected, Text::Raw(input.as_str().into())),
                _ => (
                    app.styles.normal,
                    Text::Raw(app.search_term.as_str().into()),
                ),
            };

            // render inputbox
            f.render_widget(
                Paragraph::new([search_text].iter())
                    .style(input_style)
                    .block(
                        Block::default()
                            .borders(Borders::ALL)
                            .border_style(input_style),
                    )
                    .alignment(Alignment::Left)
                    .wrap(true),
                rects[0],
            );

            // format index rows
            let rows = app
                .messages
                .list
                .iter()
                .skip(scroll as usize)
                .map(|m| {
                    vec![
                        m.date_relative.to_string(),
                        m.headers.get("From").unwrap_or(&"n/a".into()).to_string(),
                        format_subject(m.headers.get("Subject"), m.depth),
                        Tags(&m.tags).to_string(),
                    ]
                })
                .enumerate()
                .map(
                    |(i, item)| match (is_input, i as u16 + scroll == app.messages.selected) {
                        (false, true) => Row::StyledData(item.into_iter(), app.styles.selected),
                        _ => Row::StyledData(item.into_iter(), app.styles.normal),
                    },
                );

            // render index
            f.render_widget(
                Table::new(["Date", "From", "Subject", "Tags"].iter(), rows)
                    .column_spacing(2)
                    .header_style(app.styles.header)
                    .block(
                        Block::default().borders(Borders::BOTTOM | Borders::LEFT | Borders::RIGHT),
                    )
                    .header_gap(0)
                    .widths(&[
                        Constraint::Length(12),
                        Constraint::Length(18),
                        Constraint::Length(45),
                        Constraint::Min(10),
                    ]),
                rects[1],
            );
        })?;

        // handle input
        if is_input {
            terminal.show_cursor()?;
            write!(
                terminal.backend_mut(),
                "{}",
                Goto(3 + input.len() as u16, 2)
            )?;
            io::stdout().flush().ok();

            match io::stdin().keys().next().unwrap() {
                Ok(Key::Char('\n')) => {
                    app.search_term = input.to_string();
                    input.clear();
                    app.state = AppState::Refresh;
                    break;
                }
                Ok(Key::Backspace) => {
                    let _ = input.pop();
                }
                Ok(Key::Esc) => is_input = false,
                Ok(Key::Char(ch)) => (*input).push(ch),
                _ => {}
            }
        } else {
            match io::stdin().keys().next().unwrap() {
                Ok(Key::Down) | Ok(Key::Char('j')) => {
                    app.messages.select_next();
                }
                Ok(Key::Up) | Ok(Key::Char('k')) => {
                    app.messages.select_prev();
                }

                Ok(Key::Char('g')) => match io::stdin().keys().next().unwrap() {
                    Ok(Key::Char('g')) => {
                        app.messages.select_first();
                    }
                    _ => {}
                },
                Ok(Key::Char('G')) => {
                    app.messages.select_last();
                }
                Ok(Key::Char('q')) => {
                    app.state = AppState::Exit;
                    break;
                }
                Ok(Key::Char('\n')) => {
                    if !app.messages.list.is_empty() {
                        app.state = AppState::View;
                    }
                    break;
                }
                Ok(Key::Char('m')) => {
                    app.state = AppState::Compose;
                    break;
                }
                Ok(Key::Char('l')) => is_input = true,
                _ => {}
            }
        }
    }

    Ok(())
}

fn format_subject(subject: Option<&String>, depth: usize) -> String {
    let mut prefix = String::from("");
    for _ in 0..depth {
        prefix.push_str("  ");
    }
    format!(
        "{}{}",
        prefix,
        subject.unwrap_or(&"<no subject>".to_string())
    )
}

fn format_headers<'a>(
    app: &App,
    msg: &message::Message,
    atts: &[message::Attachment],
) -> Vec<Text<'a>> {
    let mut headers = vec![];
    for (header, style, default) in &[
        ("From", app.styles.header, "".to_string()),
        ("To", app.styles.header, "".to_string()),
        ("Date", app.styles.header, "".to_string()),
        ("Attachments", app.styles.attachment, atts.len().to_string()),
        ("Subject", app.styles.subject, "".to_string()),
    ] {
        headers.push(Text::styled(
            format!(
                "{}: {}\n",
                header,
                msg.headers.get(*header).unwrap_or(&default.to_string())
            ),
            *style,
        ));
    }

    headers
}

fn format_attachments(atts: &Vec<message::Attachment>) -> Vec<String> {
    atts.iter()
        .map(|att| {
            let (file, mime) = match att {
                message::Attachment::File(_part, fname, mime_type) => {
                    (fname.as_str(), mime_type.as_str())
                }
                _ => ("<alternative>", "text/html"),
            };
            format!("[{} ({})]\n", file, mime)
        })
        .collect()
}

// TODO: refactor/split to smaller functions
fn view_selected(
    app: &mut App,
    terminal: &mut Terminal<TermionBackend<AlternateScreen<MouseTerminal<RawTerminal<Stdout>>>>>,
) -> Result<(), failure::Error> {
    app.state = AppState::Index;

    let msg = app.messages.get_selected()?;

    let (body, atts) = message::body_attachments(&msg.body)?;
    let headers = format_headers(&app, &msg, &atts);
    let attachments = format_attachments(&atts);

    let body_len = body.lines().count() as u16;
    let content_len = body_len + attachments.len() as u16;
    let body_text = vec![Text::Raw(body.into())];
    let (mut scroll, mut scroll_max) = (0, 0);
    let headers_len = headers.len() as u16;
    let mut selected_att: Option<usize> = None;

    loop {
        terminal.draw(|mut f| {
            let view_height = f.size().height - headers_len - 4;
            if content_len > view_height {
                scroll_max = content_len - view_height;
            }

            // build layout
            let rects = Layout::default()
                .direction(Direction::Vertical)
                .margin(1)
                .constraints(
                    [
                        Constraint::Length(headers.len() as u16 + 1),
                        Constraint::Max(std::cmp::min(view_height - atts.len() as u16, body_len)),
                        Constraint::Length(atts.len() as u16),
                    ]
                    .as_ref(),
                )
                .split(f.size());

            // render headers
            f.render_widget(
                Paragraph::new(headers.iter())
                    .block(Block::default())
                    .alignment(Alignment::Left)
                    .wrap(true),
                rects[0],
            );

            // render body
            f.render_widget(
                Paragraph::new(body_text.iter())
                    .block(Block::default())
                    .alignment(Alignment::Left)
                    .scroll(scroll)
                    .wrap(true),
                rects[1],
            );

            // render attachments
            let items: Vec<Text> = attachments
                .iter()
                .enumerate()
                .map(|(i, s)| match selected_att {
                    Some(selected) if selected == i => {
                        Text::styled(s.to_string(), app.styles.selected)
                    }
                    _ => Text::styled(s.to_string(), app.styles.attachment),
                })
                .collect();
            f.render_widget(
                Paragraph::new(items.iter())
                    .block(Block::default().borders(Borders::TOP).border_style(
                        match selected_att {
                            Some(_) => app.styles.selected,
                            _ => app.styles.normal,
                        },
                    ))
                    .style(app.styles.attachment),
                rects[2],
            );
        })?;

        match io::stdin().keys().next().unwrap() {
            Ok(Key::Char('q')) | Ok(Key::Char('i')) => break,
            Ok(Key::Char('j')) | Ok(Key::Down) => {
                if scroll < scroll_max {
                    scroll += 1;
                } else {
                    match selected_att {
                        Some(selected) if selected < atts.len() - 1 => {
                            selected_att = Some(selected + 1);
                        }
                        None if !atts.is_empty() => selected_att = Some(0),
                        _ => {}
                    }
                }
            }
            Ok(Key::Char('k')) | Ok(Key::Up) => {
                if let Some(selected) = selected_att {
                    match selected > 0 {
                        true => selected_att = Some(selected - 1),
                        _ => selected_att = None,
                    }
                } else if scroll > 0 {
                    scroll -= 1;
                }
            }
            Ok(Key::Char('g')) => match io::stdin().keys().next().unwrap() {
                Ok(Key::Char('g')) => scroll = 0,
                _ => {}
            },
            Ok(Key::Char('G')) => scroll = scroll_max,
            Ok(Key::Char('\n')) => {
                if let Some(selected) = selected_att {
                    show_attachment(&msg.id, &atts[selected as usize])?;
                }
            }
            _ => {}
        }
    }

    Ok(())
}

fn show_attachment(id: &str, attachment: &message::Attachment) -> Result<(), failure::Error> {
    let mut tmp_file = std::env::temp_dir();
    match attachment {
        message::Attachment::File(part, fname, _mime) => {
            tmp_file.push(fname);
            let child = Command::new("notmuch")
                .args(&["show", "--format=raw"])
                .arg(format!("--part={}", part))
                .arg(format!("id:{}", id))
                .output()?;

            std::fs::write(&tmp_file, child.stdout)?;
        }
        message::Attachment::Html(s) => {
            tmp_file.push(format!("{}.html", id));
            std::fs::write(&tmp_file, s.as_bytes())?;
        }
    }

    let _child = Command::new("xdg-open").arg(&tmp_file).status()?;
    std::fs::remove_file(tmp_file)?;

    Ok(())
}

fn main() -> Result<(), failure::Error> {
    env_logger::init();

    let stdout = io::stdout().into_raw_mode()?;
    let stdout = MouseTerminal::from(stdout);
    let stdout = AlternateScreen::from(stdout);
    let backend = TermionBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;
    terminal.hide_cursor()?;

    let mut app = App::new();

    loop {
        match app.state {
            AppState::Refresh => {
                refresh_index(&mut app)?;
            }
            AppState::Index => {
                show_index(&mut app, &mut terminal)?;
            }
            AppState::View => {
                view_selected(&mut app, &mut terminal)?;
            }
            AppState::Exit => {
                break;
            }
            _ => (),
        }
    }

    Ok(())
}
