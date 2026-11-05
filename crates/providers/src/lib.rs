//! Provider trait と登録簿。
//!
//! 境界：`providers` は**業務ロジックを持たない**（`02-アーキテクチャ` §3）。
//! 能力名は `&'static str` で持つ。`runtime` の型付き `Capability` を参照すると
//! `runtime → providers → runtime` の循環になるため、語彙は文字列で受け渡す。

#![forbid(unsafe_code)]

/// Provider の健全性。`/readyz` と管理画面の表示に使う。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Health {
    Up,
    /// 動いてはいるが劣化している。理由を 1 行で持つ（画面に出す）。
    Degraded(&'static str),
    Down,
}

impl Health {
    pub fn is_up(&self) -> bool {
        matches!(self, Self::Up | Self::Degraded(_))
    }
}

/// 外部能力の提供者。LLM、埋め込み、rerank、グラフ、コード解析など。
pub trait Provider: Send + Sync + 'static {
    /// 設定上の識別子。ログと監査に出す。
    fn name(&self) -> &'static str;

    /// 提供できる能力名（`knowledge.search` 等）。
    /// **Tool 名ではない**（`04-コアエンジン` §4.7）。
    fn capabilities(&self) -> &'static [&'static str];

    fn health(&self) -> Health;
}

/// Provider の登録簿。起動時に 1 度だけ構築し、以降は読み取り専用。
#[derive(Default)]
pub struct Registry {
    providers: Vec<Box<dyn Provider>>,
}

impl Registry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, p: Box<dyn Provider>) {
        self.providers.push(p);
    }

    pub fn iter(&self) -> impl Iterator<Item = &dyn Provider> {
        self.providers.iter().map(|p| p.as_ref())
    }

    /// 指定能力を持つ Provider を探す。複数あれば先勝ち（切替は管理画面から）。
    pub fn find_by_capability(&self, capability: &str) -> Option<&dyn Provider> {
        self.iter().find(|p| p.capabilities().contains(&capability))
    }

    /// 全体の健全性。1 つでも `Down` なら `Down`。
    pub fn health(&self) -> Health {
        let mut degraded = None;
        for p in self.iter() {
            match p.health() {
                Health::Down => return Health::Down,
                Health::Degraded(r) => degraded = degraded.or(Some(r)),
                Health::Up => {}
            }
        }
        match degraded {
            Some(r) => Health::Degraded(r),
            None => Health::Up,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fake(&'static [&'static str], Health);

    impl Provider for Fake {
        fn name(&self) -> &'static str {
            "fake"
        }
        fn capabilities(&self) -> &'static [&'static str] {
            self.0
        }
        fn health(&self) -> Health {
            self.1.clone()
        }
    }

    #[test]
    fn finds_provider_by_capability() {
        let mut r = Registry::new();
        r.register(Box::new(Fake(&["knowledge.search"], Health::Up)));
        assert!(r.find_by_capability("knowledge.search").is_some());
        assert!(r.find_by_capability("graph.neighbor").is_none());
    }

    #[test]
    fn any_down_makes_registry_down() {
        let mut r = Registry::new();
        r.register(Box::new(Fake(&["a"], Health::Up)));
        r.register(Box::new(Fake(&["b"], Health::Down)));
        assert_eq!(r.health(), Health::Down);
    }
}
