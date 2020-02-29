use chrono::prelude::*;
use log::*;
use std::fs::File;
use std::io;
use std::io::prelude::*;
use std::io::Stdout;
use std::process::Command;
use termion::event::Key;
use termion::input::MouseTerminal;
use termion::input::TermRead;
use termion::raw::IntoRawMode;
use termion::raw::RawTerminal;
use termion::screen::AlternateScreen;
use tui::backend::TermionBackend;
use tui::layout::{Alignment, Constraint, Direction, Layout};
use tui::style::{Color, Modifier, Style};
use tui::widgets::{Block, Borders, Paragraph, Text};
use tui::Terminal;

mod message;

enum AppState {
    Refresh,
    Index,
    View,
    _EditSubject,
    Compose,
    Exit,
}
struct App {
    state: AppState,
    messages: Vec<message::Message>,
    selected: usize,
    selected_att: isize,
    search_term: String,
    style_selected: Style,
    style_header: Style,
    style_normal: Style,
    style_subject: Style,
    style_attachment: Style,
}

impl App {
    fn new() -> App {
        App {
            state: AppState::Refresh,
            search_term: "tag:inbox".to_string(),
            messages: vec![],
            selected: 0,
            selected_att: -1,
            style_selected: Style::default().fg(Color::Yellow).modifier(Modifier::BOLD),
            style_normal: Style::default().fg(Color::White),
            style_header: Style::default().fg(Color::Cyan),
            style_subject: Style::default()
                .fg(Color::Rgb(255, 255, 255))
                .modifier(Modifier::BOLD),
            style_attachment: Style::default().fg(Color::Blue),
        }
    }
}

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
    // let id = now.format(id_fmt);
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

    app.messages = message::parse_messages(&app.search_term)?;
    app.state = AppState::Index;

    Ok(())
}

fn show_index(
    app: &mut App,
    terminal: &mut Terminal<TermionBackend<AlternateScreen<MouseTerminal<RawTerminal<Stdout>>>>>,
) -> Result<(), failure::Error> {
    debug!("Showing index: {} messages", &app.messages.len());

    loop {
        terminal.draw(|mut f| {
            let rects = Layout::default()
                .direction(Direction::Vertical)
                .horizontal_margin(1)
                .constraints([Constraint::Percentage(100)].as_ref())
                .split(f.size());

            let mut lines: Vec<Text> = vec![];

            for (i, m) in app.messages.iter().enumerate() {
                lines.push(Text::Styled(
                    format!(
                        "{:<12}  {:<40} {:?}\n",
                        &m.date_relative,
                        &m.headers
                            .get("Subject")
                            .unwrap_or(&"<no subject>".to_string()),
                        &m.tags
                    )
                    .into(),
                    if i == app.selected {
                        app.style_selected
                    } else {
                        app.style_normal
                    },
                ));
            }

            let title = &format!("{} notes", lines.len());
            f.render_widget(
                Paragraph::new(lines.iter())
                    .block(Block::default().title(&title).borders(Borders::TOP))
                    .alignment(Alignment::Left)
                    .wrap(true),
                rects[0],
            );
        })?;

        match io::stdin().keys().next().unwrap() {
            Ok(Key::Down) | Ok(Key::Char('j')) => {
                if !app.messages.is_empty() && (app.selected < app.messages.len() - 1) {
                    app.selected += 1;
                } else {
                    app.selected = app.selected;
                }
            }
            Ok(Key::Up) | Ok(Key::Char('k')) => {
                if !app.messages.is_empty() && app.selected > 0 {
                    app.selected -= 1;
                }
            }
            Ok(Key::Char('q')) => {
                app.state = AppState::Exit;
                break;
            }
            Ok(Key::Char('\n')) => {
                app.state = AppState::View;
                break;
            }
            Ok(Key::Char('m')) => {
                app.state = AppState::Compose;
                break;
            }
            _ => {}
        }
    }
    Ok(())
}

fn view_selected(
    app: &mut App,
    terminal: &mut Terminal<TermionBackend<AlternateScreen<MouseTerminal<RawTerminal<Stdout>>>>>,
) -> Result<(), failure::Error> {
    app.state = AppState::Index;

    let msg = app
        .messages
        .get(app.selected)
        .ok_or(failure::format_err!("Selected message missing!"))?;

    let (body, atts) = message::body_attachments(&msg.body)?;

    let mut headers = vec![];
    for (header, style, default) in &[
        ("From", app.style_header, "".to_string()),
        ("To", app.style_header, "".to_string()),
        ("Date", app.style_header, "".to_string()),
        ("Attachments", app.style_attachment, atts.len().to_string()),
        ("Subject", app.style_subject, "".to_string()),
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

    let body_len = body.lines().count() as u16;
    let content_len = body_len + atts.len() as u16;
    let body_text = vec![Text::Raw(body.into())];
    let (mut scroll, mut scroll_max) = (0, 0);
    let headers_len = headers.len() as u16 - 4;

    loop {
        terminal.draw(|mut f| {
            let view_height = f.size().height - headers_len;
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
                        Constraint::Length(body_len + 1),
                        Constraint::Length(atts.len() as u16 + 1),
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
            if !atts.is_empty() {
                let mut attachments = vec![];
                for (i, (_id, fname, mime)) in atts.iter().enumerate() {
                    let att = format!(
                        "{}[{:>2}: {}  ({})]\n",
                        match i as isize == app.selected_att {
                            true => " > ",
                            false => "",
                        },
                        i,
                        fname,
                        mime
                    );
                    attachments.push(Text::styled(
                        att,
                        match i as isize == app.selected_att {
                            true => app.style_selected,
                            _ => app.style_attachment,
                        },
                    ));
                }
                f.render_widget(
                    Paragraph::new(attachments.iter())
                        .block(Block::default())
                        .alignment(Alignment::Left)
                        .scroll(scroll)
                        .wrap(true),
                    rects[2],
                );
            }
        })?;

        match io::stdin().keys().next().unwrap() {
            Ok(Key::Char('q')) | Ok(Key::Char('i')) => break,
            Ok(Key::Char('j')) | Ok(Key::Down) => {
                if scroll < scroll_max {
                    scroll += 1;
                } else if app.selected_att < atts.len() as isize - 1 {
                    app.selected_att += 1;
                }
            }
            Ok(Key::Char('k')) | Ok(Key::Up) => {
                if app.selected_att >= 0 {
                    app.selected_att -= 1;
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
                if app.selected_att >= 0 {
                    show_attachment(&msg.id, &atts[app.selected_att as usize])?;
                }
            }
            _ => {}
        }
    }

    Ok(())
}
fn show_attachment(
    id: &str,
    (part, fname, mime): &(usize, String, String),
) -> Result<(), failure::Error> {
    let mut tmp = "/tmp/".to_string();
    tmp.push_str(&fname);

    let out = Command::new("notmuch")
        .args(&["show", "--format=raw"])
        .arg(format!("--part={}", part))
        .arg(format!("id:{}", id))
        .output()?;

    std::fs::write(&tmp, out.stdout)?;

    let _show = Command::new("ristretto").arg(&tmp).output()?;
    std::fs::remove_file(tmp)?;

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
