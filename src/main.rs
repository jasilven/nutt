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
use termion::input::MouseTerminal;
use termion::input::TermRead;
use termion::raw::IntoRawMode;
use termion::raw::RawTerminal;
use termion::screen::AlternateScreen;
use tui::backend::TermionBackend;
use tui::layout::{Alignment, Constraint, Direction, Layout};
use tui::style::{Color, Modifier, Style};
use tui::widgets::{Block, Borders, Paragraph, Row, Table, Text};
use tui::Terminal;

mod message;

struct Messages {
    list: Vec<(String, message::Message)>,
    selected: u16,
}

impl Messages {
    fn new(messages: &Vec<message::Message>) -> Self {
        let msgs = Messages::build_messages(messages, "");
        Messages {
            list: msgs,
            selected: 0,
        }
    }

    fn build_messages(
        messages: &Vec<message::Message>,
        prefix: &str,
    ) -> Vec<(String, message::Message)> {
        let mut result = vec![];
        let title_len = 45;
        for m in messages {
            let title = format!(
                "{}{}",
                prefix,
                &m.headers
                    .get("Subject")
                    .unwrap_or(&"<no subject>".to_string()),
            )
            .chars()
            .take(title_len)
            .collect::<String>();
            result.push((
                format!(
                    " . {: >19}  {1:-<2$}    {3}\n",
                    &m.date_relative,
                    &title,
                    title_len,
                    Tags(&m.tags)
                ),
                m.clone(),
            ));
            for (title, reply) in
                Messages::build_messages(&m.replys, &format!("{}{}", prefix, "  ")).iter()
            {
                result.push((title.clone(), reply.clone()));
            }
        }
        result
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

    fn get_selected(&self) -> Option<&(String, message::Message)> {
        if !self.list.is_empty() {
            self.list.get(self.selected as usize)
        } else {
            None
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
    messages: Messages,
    styles: Styles,
    selected_att: isize,
    search_term: String,
}

impl App {
    fn new() -> App {
        App {
            state: AppState::Refresh,
            search_term: "tag:inbox".to_string(),
            messages: Messages::new(&vec![]),
            selected_att: -1,
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
    app.messages = Messages::new(&messages);
    app.state = AppState::Index;

    Ok(())
}

// TODO: refactor/split to smaller functions
// TODO: simplify scrolling logic which is awful now
fn show_index(
    app: &mut App,
    terminal: &mut Terminal<TermionBackend<AlternateScreen<MouseTerminal<RawTerminal<Stdout>>>>>,
) -> Result<(), failure::Error> {
    use Color::*;
    debug!("Showing index: {} messages", &app.messages.len());

    let mut is_input = false;
    let input = &mut String::new();
    let (mut scroll, mut scroll_max) = (0, 0);

    loop {
        terminal.hide_cursor()?;
        terminal.draw(|mut f| {
            let view_height = f.size().height - 5;
            if app.messages.len() > view_height {
                scroll_max = app.messages.len() - view_height;
            }

            if app.messages.selected >= view_height && scroll < scroll_max {
                scroll += 1;
            }

            let rects = Layout::default()
                .direction(Direction::Vertical)
                .horizontal_margin(1)
                .constraints([Constraint::Length(3), Constraint::Min(1)].as_ref())
                .split(f.size());

            let (input_color, index_color, search_text) = match is_input {
                true => (Yellow, White, Text::Raw(input.to_string().into())),
                _ => (White, Yellow, Text::Raw(app.search_term.to_string().into())),
            };

            // render input
            f.render_widget(
                Paragraph::new([search_text].iter())
                    .style(Style::default().fg(input_color))
                    .block(
                        Block::default()
                            .borders(Borders::ALL)
                            .border_style(Style::default().fg(input_color)),
                    )
                    .alignment(Alignment::Left)
                    .wrap(true),
                rects[0],
            );

            let lines: Vec<Text> = app
                .messages
                .list
                .iter()
                .enumerate()
                .map(|(i, (s, _m))| {
                    Text::Styled(
                        s.into(),
                        match (is_input, i == app.messages.selected as usize) {
                            (true, _) | (false, false) => app.styles.normal,
                            (false, true) => app.styles.selected,
                        },
                    )
                })
                .collect();

            // render message list/index
            f.render_widget(
                Paragraph::new(lines.iter())
                    .block(
                        Block::default()
                            .title(&format!("Found: {}", app.messages.len()))
                            .border_style(Style::default().fg(index_color))
                            .borders(Borders::BOTTOM | Borders::LEFT | Borders::RIGHT),
                    )
                    .scroll(scroll)
                    .alignment(Alignment::Left)
                    .wrap(true),
                rects[1],
            );
            // let lines: Vec<Vec<String>> = app
            //     .messages
            //     .list
            //     .iter()
            //     .enumerate()
            //     .map(|(i, (_s, m))| {
            //         vec![
            //             m.date_relative.clone(),
            //             m.headers
            //                 .get("Subject")
            //                 .unwrap_or(&"<no subject>".to_string())
            //                 .clone(),
            //             Tags(&m.tags).to_string(),
            //         ]
            //     })
            //     .collect();
            // let rows = lines.iter().enumerate().map(|(i, item)| {
            //     match (is_input, i == app.messages.selected as usize) {
            //         (true, _) | (false, false) => {
            //             Row::StyledData(item.into_iter(), app.styles.normal)
            //         }
            //         (false, true) => Row::StyledData(item.into_iter(), app.styles.selected),
            //     }
            // });
            // let table = Table::new([""].iter(), rows)
            //     .header_style(Style::default().modifier(Modifier::HIDDEN))
            //     .scroll(scroll)
            //     .block(Block::default().borders(Borders::BOTTOM | Borders::LEFT | Borders::RIGHT))
            //     .widths(&[
            //         Constraint::Length(13),
            //         Constraint::Length(45),
            //         Constraint::Min(10),
            //     ]);
            // f.render_widget(table, rects[1]);
        })?;

        if is_input {
            terminal.show_cursor()?;
            write!(
                terminal.backend_mut(),
                "{}",
                Goto(3 + input.len() as u16, 2)
            )
            .unwrap();
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
                    if app.messages.selected as u16 <= scroll && scroll > 0 {
                        scroll -= 1
                    }
                }

                Ok(Key::Char('g')) => match io::stdin().keys().next().unwrap() {
                    Ok(Key::Char('g')) => {
                        scroll = 0;
                        app.messages.select_first();
                    }
                    _ => {}
                },
                Ok(Key::Char('G')) => {
                    app.messages.select_last();
                    scroll = scroll_max;
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

// TODO: refactor/split to smaller functions
fn view_selected(
    app: &mut App,
    terminal: &mut Terminal<TermionBackend<AlternateScreen<MouseTerminal<RawTerminal<Stdout>>>>>,
) -> Result<(), failure::Error> {
    app.state = AppState::Index;

    let (_, msg) = app
        .messages
        .get_selected()
        .ok_or(failure::format_err!("Selected message missing!"))?;

    let (body, atts) = message::body_attachments(&msg.body)?;

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
                            true => app.styles.selected,
                            _ => app.styles.attachment,
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
    (part, fname, _mime): &(usize, String, String),
) -> Result<(), failure::Error> {
    let mut tmp_file = std::env::temp_dir();
    tmp_file.push(fname);

    let out = Command::new("notmuch")
        .args(&["show", "--format=raw"])
        .arg(format!("--part={}", part))
        .arg(format!("id:{}", id))
        .output()?;

    std::fs::write(&tmp_file, out.stdout)?;

    let _show = Command::new("ristretto").arg(&tmp_file).output()?;
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
