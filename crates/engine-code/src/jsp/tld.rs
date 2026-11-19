//! TLD（Tag Library Descriptor）の最小パーサ。
//!
//! JSP のカスタムタグは、TLD が無いと**何者なのか決められない**。
//! ここでは `uri` / `short-name` / `tag` 名だけを拾う（属性の型までは見ない）。
//! 推測はしない — TLD に無いタグは「無い」として扱い、呼び出し側が UNRESOLVED にする。

use quick_xml::events::Event;
use quick_xml::Reader;

/// TLD 1 ファイル分。
#[derive(Debug, Clone, Default)]
pub struct TldDoc {
    /// `<uri>`。taglib 指令の uri と突き合わせる鍵。
    pub uri: Option<String>,
    /// `<short-name>`。uri が無いときの手掛かり。
    pub short_name: Option<String>,
    pub tags: Vec<TldTag>,
}

#[derive(Debug, Clone)]
pub struct TldTag {
    pub name: String,
    /// `<attribute><name>` を並べたもの（使用箇所の検証にはまだ使っていない）。
    pub attributes: Vec<String>,
}

impl TldDoc {
    pub fn has_tag(&self, name: &str) -> bool {
        self.tags.iter().any(|tag| tag.name == name)
    }
}

fn local(name: &[u8]) -> String {
    let text = String::from_utf8_lossy(name);
    text.rsplit(':').next().unwrap_or(&text).to_string()
}

/// TLD を解析する。`<taglib>` が無い / XML として壊れている場合は `None`。
pub fn parse(text: &str) -> Option<TldDoc> {
    let mut reader = Reader::from_str(text);
    reader.config_mut().trim_text(true);

    let mut doc = TldDoc::default();
    let mut stack: Vec<String> = Vec::new();
    let mut current: Option<TldTag> = None;
    let mut saw_taglib = false;
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(event)) => {
                let name = local(event.name().as_ref());
                if name == "taglib" {
                    saw_taglib = true;
                }
                if name == "tag" {
                    current = Some(TldTag {
                        name: String::new(),
                        attributes: Vec::new(),
                    });
                }
                stack.push(name);
            }
            Ok(Event::Text(event)) => {
                let parent = stack.last().map(String::as_str).unwrap_or("");
                let grand = stack.iter().rev().nth(1).map(String::as_str).unwrap_or("");
                let raw = String::from_utf8_lossy(event.as_ref());
                let unescaped = quick_xml::escape::unescape(&raw)
                    .map(|text| text.into_owned())
                    .unwrap_or_else(|_| raw.into_owned());
                let value = unescaped.trim().to_string();
                if value.is_empty() {
                    continue;
                }
                match (grand, parent) {
                    ("taglib", "uri") => doc.uri = Some(value),
                    ("taglib", "short-name") => doc.short_name = Some(value),
                    ("tag", "name") => {
                        if let Some(tag) = current.as_mut() {
                            if tag.name.is_empty() {
                                tag.name = value;
                            }
                        }
                    }
                    ("attribute", "name") => {
                        if let Some(tag) = current.as_mut() {
                            tag.attributes.push(value);
                        }
                    }
                    _ => {}
                }
            }
            Ok(Event::End(event)) => {
                let name = local(event.name().as_ref());
                if name == "tag" {
                    if let Some(tag) = current.take() {
                        if !tag.name.is_empty() {
                            doc.tags.push(tag);
                        }
                    }
                }
                stack.pop();
            }
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(_) => return None,
        }
        buf.clear();
    }

    if !saw_taglib {
        return None;
    }
    Some(doc)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<taglib xmlns="http://java.sun.com/xml/ns/javaee" version="2.1">
  <short-name>fmt</short-name>
  <uri>http://java.sun.com/jsp/jstl/fmt</uri>
  <tag>
    <name>formatDate</name>
    <tag-class>org.apache.taglibs.standard.tag.FormatDateTag</tag-class>
    <attribute><name>value</name></attribute>
    <attribute><name>pattern</name></attribute>
  </tag>
  <tag>
    <name>message</name>
  </tag>
</taglib>"#;

    #[test]
    fn parses_uri_and_tags() {
        let doc = parse(SAMPLE).expect("TLD");
        assert_eq!(doc.uri.as_deref(), Some("http://java.sun.com/jsp/jstl/fmt"));
        assert_eq!(doc.short_name.as_deref(), Some("fmt"));
        assert_eq!(doc.tags.len(), 2);
        assert!(doc.has_tag("formatDate"));
        assert!(doc.has_tag("message"));
        assert!(!doc.has_tag("forEach"));
        assert_eq!(doc.tags[0].attributes, vec!["value", "pattern"]);
    }

    #[test]
    fn rejects_non_tld() {
        assert!(parse("<html><body/></html>").is_none());
        assert!(parse("not xml at all").is_none());
    }
}
