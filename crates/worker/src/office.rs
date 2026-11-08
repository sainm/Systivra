use std::collections::HashMap;
use std::io::{Cursor, Read};

use quick_xml::events::Event;
use quick_xml::Reader;

#[derive(Debug, thiserror::Error)]
pub enum OfficeError {
    #[error("ZIP として壊れている: {0}")]
    Zip(String),
    #[error("XML の解析に失敗: {0}")]
    Xml(String),
    #[error("必要なパーツが無い: {0}")]
    Missing(String),
    #[error("関係（rels）が解決できない: {0}")]
    Rel(String),
}

pub struct Package {
    pub(crate) parts: HashMap<String, Vec<u8>>,
}

pub fn open(bytes: &[u8]) -> Result<Package, OfficeError> {
    let mut zip =
        zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| OfficeError::Zip(e.to_string()))?;
    let mut parts = HashMap::new();

    for i in 0..zip.len() {
        let mut file = zip
            .by_index(i)
            .map_err(|e| OfficeError::Zip(e.to_string()))?;
        if !file.is_file() {
            continue;
        }
        let mut buf = Vec::new();
        file.read_to_end(&mut buf)
            .map_err(|e| OfficeError::Zip(e.to_string()))?;
        parts.insert(file.name().to_string(), buf);
    }

    Ok(Package { parts })
}

impl Package {
    pub(crate) fn has(&self, part: &str) -> bool {
        self.parts.contains_key(part)
    }

    pub(crate) fn part(&self, part: &str) -> Option<&[u8]> {
        self.parts.get(part).map(Vec::as_slice)
    }

    pub(crate) fn xml(&self, part: &str) -> Result<Node, OfficeError> {
        let bytes = self
            .parts
            .get(part)
            .ok_or_else(|| OfficeError::Missing(part.to_string()))?;
        parse_tree(bytes)
    }

    pub(crate) fn rels_for(&self, part: &str) -> Result<HashMap<String, String>, OfficeError> {
        let (dir, file) = match part.rsplit_once('/') {
            Some((d, f)) => (d, f),
            None => ("", part),
        };
        let rels_path = if dir.is_empty() {
            format!("_rels/{file}.rels")
        } else {
            format!("{dir}/_rels/{file}.rels")
        };

        if !self.has(&rels_path) {
            return Ok(HashMap::new());
        }

        let root = self.xml(&rels_path)?;
        let mut map = HashMap::new();
        for r in root.descendants("Relationship") {
            let (Some(id), Some(target)) = (r.attr("Id"), r.attr("Target")) else {
                continue;
            };
            map.insert(id.to_string(), target.to_string());
        }
        Ok(map)
    }
}

#[derive(Debug, Default)]
pub(crate) struct Node {
    pub(crate) name: String,
    pub(crate) attrs: Vec<(String, String)>,
    pub(crate) text: String,
    pub(crate) children: Vec<Node>,
}

impl Node {
    pub(crate) fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }

    pub(crate) fn child(&self, name: &str) -> Option<&Node> {
        self.children.iter().find(|c| c.name == name)
    }

    pub(crate) fn child_text(&self, name: &str) -> Option<&str> {
        self.child(name)
            .map(|c| c.text.trim())
            .filter(|t| !t.is_empty())
    }

    pub(crate) fn descendants<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Node> + 'a {
        let mut stack: Vec<&Node> = vec![self];
        std::iter::from_fn(move || {
            while let Some(n) = stack.pop() {
                for c in n.children.iter().rev() {
                    stack.push(c);
                }
                if n.name == name {
                    return Some(n);
                }
            }
            None
        })
    }
}

fn local(name: &[u8]) -> String {
    let s = String::from_utf8_lossy(name);
    match s.rsplit_once(':') {
        Some((_, local)) => local.to_string(),
        None => s.to_string(),
    }
}

pub(crate) fn parse_tree(xml: &[u8]) -> Result<Node, OfficeError> {
    let mut reader = Reader::from_reader(xml);
    reader.config_mut().trim_text(false);

    let mut root = Node::default();
    let mut stack: Vec<Node> = Vec::new();
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let mut node = Node {
                    name: local(e.local_name().as_ref()),
                    ..Default::default()
                };
                for a in e.attributes().flatten() {
                    let key = local(a.key.local_name().as_ref());
                    let raw = String::from_utf8_lossy(&a.value);
                    let val = quick_xml::escape::unescape(&raw)
                        .map(|v| v.into_owned())
                        .unwrap_or_else(|_| raw.to_string());
                    node.attrs.push((key, val));
                }
                stack.push(node);
            }
            Ok(Event::Empty(e)) => {
                let mut node = Node {
                    name: local(e.local_name().as_ref()),
                    ..Default::default()
                };
                for a in e.attributes().flatten() {
                    let key = local(a.key.local_name().as_ref());
                    let raw = String::from_utf8_lossy(&a.value);
                    let val = quick_xml::escape::unescape(&raw)
                        .map(|v| v.into_owned())
                        .unwrap_or_else(|_| raw.to_string());
                    node.attrs.push((key, val));
                }
                match stack.last_mut() {
                    Some(parent) => parent.children.push(node),
                    None => root.children.push(node),
                }
            }
            Ok(Event::Text(e)) => {
                if let Some(top) = stack.last_mut() {
                    let raw = String::from_utf8_lossy(&e);
                    let text = quick_xml::escape::unescape(&raw)
                        .map(|v| v.into_owned())
                        .unwrap_or_else(|_| raw.to_string());
                    top.text.push_str(&text);
                }
            }
            Ok(Event::End(_)) => {
                let Some(node) = stack.pop() else { continue };
                match stack.last_mut() {
                    Some(parent) => parent.children.push(node),
                    None => root.children.push(node),
                }
            }
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(e) => return Err(OfficeError::Xml(e.to_string())),
        }
        buf.clear();
    }

    Ok(root)
}

pub(crate) fn parent_of(part: &str) -> &str {
    match part.rsplit_once('/') {
        Some((d, _)) => d,
        None => "",
    }
}

pub(crate) fn normalize_part(base_dir: &str, target: &str) -> String {
    if let Some(rest) = target.strip_prefix('/') {
        return rest.to_string();
    }
    let mut parts: Vec<&str> = base_dir.split('/').filter(|s| !s.is_empty()).collect();
    for seg in target.split('/') {
        match seg {
            "." | "" => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    parts.join("/")
}

pub(crate) fn resolve_target(part: &str, target: &str) -> String {
    normalize_part(parent_of(part), target)
}
