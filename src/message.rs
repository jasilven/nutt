use failure::bail;
use log::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::Write;
use std::process::{Command, Stdio};

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(untagged)]
pub enum Content {
    Str(String),
    Array(Vec<Body>),
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Body {
    id: usize,
    #[serde(rename = "content-type")]
    content_type: String,
    #[serde(rename = "content-transfer-encoding")]
    content_transfer_encoding: Option<String>,
    #[serde(rename = "content-charset")]
    content_charset: Option<String>,
    content: Option<Content>,
    filename: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Message {
    pub id: String,
    #[serde(rename = "match", skip)]
    pub filename: Vec<String>,
    pub timestamp: u64,
    pub date_relative: String,
    pub tags: Vec<String>,
    pub body: Vec<Body>,
    pub headers: HashMap<String, String>,
    #[serde(skip)]
    pub depth: usize,
    #[serde(skip)]
    pub replys: Vec<Message>,
    // pub matches: bool,
    // pub excluded: bool,
    // #[serde(skip)]
    // pub crypto: HashMap<String, String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(untagged)]
pub enum Node {
    Msg(Message),
    Children(Vec<Vec<Node>>),
}

fn html_to_text(html: &str) -> Result<String, failure::Error> {
    let mut child = Command::new("lynx")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .arg("-stdin")
        .arg("-dump")
        .arg("-width")
        .arg("80")
        .arg("-display_charset=UTF-8")
        .spawn()?;

    let stdin = child
        .stdin
        .as_mut()
        .ok_or(failure::format_err!("Failed to run lynx"))?;
    stdin.write_all(html.as_bytes())?;

    let output = child.wait_with_output()?;
    let result = std::str::from_utf8(&output.stdout)?.to_string();

    Ok(result)
}

pub enum Attachment {
    Html(String),
    File(usize, String, String),
}

pub fn body_attachments(bodys: &Vec<Body>) -> Result<(String, Vec<Attachment>), failure::Error> {
    debug!("body_attachments: {:?}", &bodys);

    let mut body = String::from("");
    let mut body_html = String::from("");

    let mut attachments: Vec<Attachment> = vec![];

    for b in bodys {
        match &b.content {
            Some(Content::Str(s)) => match b.content_type.as_str() {
                "text/html" => {
                    body_html.push_str(s);
                }
                _ => body.push_str(s),
            },
            Some(Content::Array(bs)) => {
                let (b, atts) = body_attachments(bs)?;
                body.push_str(&b);
                attachments.extend(atts);
            }
            _ => {}
        }

        if let Some(filename) = &b.filename {
            attachments.push(Attachment::File(
                b.id,
                filename.to_string(),
                b.content_type.to_string(),
            ));
        }
    }
    if body.is_empty() {
        body = html_to_text(&body_html)?;
    }
    if !body_html.is_empty() {
        attachments.push(Attachment::Html(body_html));
    }

    debug!(
        "Parsed: lines: {:?}, attachments: {}",
        &body,
        &attachments.len()
    );

    Ok((body.into(), attachments))
}

pub fn parse_thread(
    thread: &Vec<Node>,
    depth: usize,
    messages: &mut Vec<Message>,
) -> Result<(), failure::Error> {
    // let mut result = vec![];

    if let Some(Node::Msg(msg)) = thread.iter().cloned().next() {
        let mut message = msg.clone();
        message.depth = depth;
        messages.push(message);
        for reply in thread.iter().skip(1) {
            match reply {
                Node::Children(childs) => {
                    for child in childs {
                        // messages.push(child);
                        parse_thread(&child, depth + 1, messages)?;
                    }
                }
                _ => bail!("Parse Error: expected children."),
            }
        }
    } else {
        bail!("Parse Error: expected message, but got something else.")
    }

    Ok(())
}

pub fn parse_messages(search_term: &str) -> Result<Vec<Message>, failure::Error> {
    debug!("Parsing search result: {}", search_term);

    let mut result: Vec<Message> = vec![];

    // TODO: remove path (e.g. use env)
    let output = Command::new("notmuch")
        .arg("show")
        .arg("--format=json")
        .arg("--include-html")
        .arg(search_term)
        .output()?;

    let threadset: Vec<Vec<Vec<Node>>> = serde_json::from_slice(&output.stdout)?;

    for threads in threadset.iter() {
        for thread in threads.iter() {
            parse_thread(thread, 0, &mut result)?;
            // result.append(&mut t);
        }
    }

    Ok(result)
}
