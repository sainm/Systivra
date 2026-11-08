use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    Java,
    Jsp,
    Xml,
    Sql,
}

impl Language {
    pub fn detect(path: &Path) -> Option<Self> {
        let ext = path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        match ext.as_str() {
            "java" => Some(Self::Java),
            "jsp" | "jspx" | "tag" | "tagx" => Some(Self::Jsp),
            "xml" | "dicon" | "tld" => Some(Self::Xml),
            "sql" => Some(Self::Sql),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Java => "JAVA",
            Self::Jsp => "JSP",
            Self::Xml => "XML",
            Self::Sql => "SQL",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_case_insensitive_extensions() {
        assert_eq!(Language::detect(Path::new("A.JAVA")), Some(Language::Java));
        assert_eq!(
            Language::detect(Path::new("view.jspx")),
            Some(Language::Jsp)
        );
        assert_eq!(
            Language::detect(Path::new("component.dicon")),
            Some(Language::Xml)
        );
        assert_eq!(
            Language::detect(Path::new("schema.sql")),
            Some(Language::Sql)
        );
        assert_eq!(Language::detect(Path::new("README.md")), None);
    }
}
