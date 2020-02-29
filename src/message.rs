use failure::bail;
use log::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::process::Command;

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
    pub matches: bool,
    pub excluded: bool,
    pub filename: Vec<String>,
    pub timestamp: u64,
    pub date_relative: String,
    pub tags: Vec<String>,
    pub body: Vec<Body>,
    #[serde(skip)]
    pub crypto: HashMap<String, String>,
    pub headers: HashMap<String, String>,
    #[serde(skip)]
    pub replys: Vec<Message>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(untagged)]
enum Node {
    Msg(Message),
    Children(Vec<Message>),
}

pub fn body_attachments(
    bodys: &Vec<Body>,
) -> Result<(String, Vec<(usize, String, String)>), failure::Error> {
    debug!("body_attachments: {:?}", &bodys);

    let mut body = String::from("");
    let mut attachments = vec![];

    for b in bodys {
        match &b.content {
            Some(Content::Str(s)) => body.push_str(s),
            Some(Content::Array(bs)) => {
                let (b, atts) = body_attachments(bs)?;
                body.push_str(&b);
                attachments.extend(atts);
            }
            _ => {}
        }

        if let Some(filename) = &b.filename {
            attachments.push((b.id, filename.to_string(), b.content_type.to_string()));
        }
    }

    debug!(
        "Parsed: lines: {:?}, attachments: {}",
        &body,
        &attachments.len()
    );
    Ok((body.into(), attachments))
}

pub fn parse_messages(search_term: &str) -> Result<Vec<Message>, failure::Error> {
    debug!("Parsing messages: {}", search_term);

    let mut result = vec![];

    let output = Command::new("/usr/bin/notmuch")
        .arg("show")
        .arg("--format=json")
        .arg(search_term)
        .output()?;

    let threadset: Vec<Vec<Vec<Node>>> = serde_json::from_slice(&output.stdout)?;

    for threads in threadset.iter() {
        for thread in threads.iter() {
            if let Some(Node::Msg(msg)) = thread.iter().cloned().next() {
                let mut message = msg.clone();
                for reply in thread.iter().skip(1) {
                    match reply {
                        Node::Children(msgs) => {
                            for m in msgs {
                                message.replys.push(m.clone());
                            }
                        }
                        _ => bail!("Parse Error: expected children."),
                    }
                }
                result.push(message);
            } else {
                bail!("Parse Error: expected message, but got something else.")
            }
        }
    }

    Ok(result)
}
