use emailmessage::Message;
use log::*;
use std::fmt;
use std::io;
use std::io::prelude::*;
use std::io::Stdout;
use std::process::{Command, Stdio};
use termion::cursor::Goto;
use termion::event::Key;
use termion::input::TermRead;
use termion::raw::{IntoRawMode, RawTerminal};
use tui::backend::TermionBackend;
use tui::layout::{Alignment, Constraint, Direction, Layout};
use tui::style::{Color, Modifier, Style};
use tui::widgets::{Block, Borders, Paragraph, Row, Table, Text};
use tui::Terminal;

mod notmuch;

struct MessageList {
    list: Vec<notmuch::Message>,
    selected: u16,
}

impl MessageList {
    fn new(list: Vec<notmuch::Message>) -> Self {
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

    fn get_selected(&self) -> Result<&notmuch::Message, failure::Error> {
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

struct App {
    state: AppState,
    messages: MessageList,
    styles: Styles,
    search_term: String,
}

impl App {
    fn new() -> App {
        App {
            state: AppState::Refresh,
            search_term: "tag:inbox".to_string(),
            messages: MessageList::new(vec![]),
            styles: Styles {
                selected: Style::default().fg(Color::Yellow).modifier(Modifier::BOLD),
                normal: Style::default(),
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
fn compose(
    app: &mut App,
    _terminal: &mut Terminal<TermionBackend<RawTerminal<Stdout>>>,
) -> Result<(), failure::Error> {
    debug!("compose");

    app.state = AppState::Refresh;

    let mut tmp_file = std::env::temp_dir();
    tmp_file.push("nutt-new.txt");

    let _ = Command::new("nvim")
        .arg(&tmp_file)
        .status()
        .expect("Failed to execute 'nvim'");

    let mut body = std::fs::read_to_string(&tmp_file)?;
    if body.lines().count() == 1 {
        body.push('\n');
    }

    std::fs::remove_file(tmp_file)?;
    // must include fields: From, Date
    let email: emailmessage::Message<&str> = Message::builder()
        .from("Me <me@localhost>".parse().unwrap())
        .date_now()
        .to("Me <me@localhost>".parse().unwrap())
        .subject("<subject>")
        .body(&body);

    let mut child = Command::new("notmuch")
        .arg("insert")
        .stdin(Stdio::piped())
        .spawn()?;

    let stdin = child
        .stdin
        .as_mut()
        .ok_or(failure::format_err!("Failed to run 'notmuch insert'"))?;
    stdin.write_all(format!("{}", email).as_bytes())?;

    Ok(())
}

fn refresh_index(app: &mut App) -> Result<(), failure::Error> {
    debug!("refresh_index: {}", &app.search_term);

    if app.search_term.is_empty() {
        app.search_term = "tag:inbox".to_string();
    }

    let messages = notmuch::parse_messages(&app.search_term)?;
    app.messages = MessageList::new(messages);
    app.state = AppState::Index;

    Ok(())
}

// TODO: refactor/split to smaller functions
// TODO: simplify scrolling logic which is awful now
fn show_index(
    app: &mut App,
    terminal: &mut Terminal<TermionBackend<RawTerminal<Stdout>>>,
) -> Result<(), failure::Error> {
    debug!("show_index");

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
                        Constraint::Length(20),
                        Constraint::Percentage(40),
                        Constraint::Percentage(30),
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
                Ok(Key::Down) | Ok(Key::Char('j')) => app.messages.select_next(),
                Ok(Key::Up) | Ok(Key::Char('k')) => app.messages.select_prev(),
                Ok(Key::Char('g')) => match io::stdin().keys().next().unwrap() {
                    Ok(Key::Char('g')) => {
                        app.messages.select_first();
                    }
                    _ => {}
                },
                Ok(Key::Char('G')) => app.messages.select_last(),
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
    //    debug!("format_subject: {:?} {}", &subject, &depth);

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
    msg: &notmuch::Message,
    atts: &[notmuch::Attachment],
) -> Vec<Text<'a>> {
    debug!("format_headers");

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

// TODO: refactor/split to smaller functions
fn view_selected(
    app: &mut App,
    terminal: &mut Terminal<TermionBackend<RawTerminal<Stdout>>>,
) -> Result<(), failure::Error> {
    debug!("view_selected");

    app.state = AppState::Index;

    let msg = app.messages.get_selected()?;

    let (body, atts) = notmuch::body_attachments(&msg.body)?;
    let headers = format_headers(&app, &msg, &atts);

    let body_len = body.lines().count() as u16;
    let content_len = body_len + atts.len() as u16;
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
            let items: Vec<Text> = atts
                .iter()
                .map(|att| match att {
                    notmuch::Attachment::File(_, _, _, name) => name,
                    notmuch::Attachment::Html(_, name) => name,
                })
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

fn write_file(fname: &std::path::PathBuf, data: &[u8]) -> Result<(), failure::Error> {
    use std::fs;
    use std::os::unix::fs::OpenOptionsExt;
    match fs::OpenOptions::new()
        .create(true)
        .write(true)
        .mode(0o600)
        .open(fname)
    {
        Err(e) => failure::bail!(e),
        Ok(mut f) => f.write_all(data)?,
    }
    Ok(())
}

fn show_attachment(id: &str, attachment: &notmuch::Attachment) -> Result<(), failure::Error> {
    debug!("show_attachment");

    let mut tmp_file = std::env::temp_dir();

    match attachment {
        notmuch::Attachment::File(part, fname, _mime, _name) => {
            tmp_file.push(fname);

            let child = Command::new("notmuch")
                .args(&["show", "--format=raw"])
                .arg(format!("--part={}", part))
                .arg(format!("id:{}", id))
                .output()?;

            write_file(&tmp_file, &child.stdout)?;
        }
        notmuch::Attachment::Html(s, _name) => {
            tmp_file.push(format!("{}.html", id));

            write_file(&tmp_file, s.as_bytes())?;
        }
    }

    let _child = Command::new("xdg-open").arg(tmp_file).status()?;

    Ok(())
}

fn get_terminal() -> Result<Terminal<TermionBackend<RawTerminal<Stdout>>>, failure::Error> {
    let stdout = io::stdout().into_raw_mode()?;
    // let stdout = MouseTerminal::from(stdout);
    // let stdout = AlternateScreen::from(stdout);
    let backend = TermionBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;
    terminal.hide_cursor()?;

    Ok(terminal)
}

fn main() -> Result<(), failure::Error> {
    env_logger::init();
    debug!("main");

    let mut app = App::new();
    let mut terminal = get_terminal()?;

    loop {
        match app.state {
            AppState::Refresh => {
                debug!("AppState::Refresh");
                refresh_index(&mut app)?;
            }
            AppState::Index => {
                show_index(&mut app, &mut terminal)?;
            }
            AppState::View => {
                view_selected(&mut app, &mut terminal)?;
            }
            AppState::Compose => {
                compose(&mut app, &mut terminal)?;
            }
            AppState::Exit => {
                break;
            }
            _ => (),
        }
    }

    Ok(())
}
