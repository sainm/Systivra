use std::sync::{Mutex, OnceLock};

use lindera::dictionary::load_dictionary;
use lindera::mode::Mode;
use lindera::segmenter::Segmenter;
use lindera_analysis::tokenizer::Tokenizer;

static TOKENIZER: OnceLock<Option<Mutex<Tokenizer>>> = OnceLock::new();

fn tokenizer() -> Option<&'static Mutex<Tokenizer>> {
    TOKENIZER
        .get_or_init(|| {
            let dictionary = load_dictionary("embedded://ipadic").ok()?;
            let segmenter = Segmenter::new(Mode::Normal, dictionary, None);
            Some(Mutex::new(Tokenizer::new(segmenter)))
        })
        .as_ref()
}

pub fn words(text: &str) -> Vec<String> {
    let Some(tokenizer) = tokenizer() else {
        return Vec::new();
    };
    let Ok(tokenizer) = tokenizer.lock() else {
        return Vec::new();
    };
    tokenizer
        .tokenize(text)
        .map(|tokens| {
            tokens
                .into_iter()
                .map(|token| token.surface.to_string())
                .filter(|word| word.chars().any(|c| c.is_alphanumeric()))
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_japanese_words_with_ipadic() {
        let words = words("住所チェックの桁数");
        assert!(words.contains(&"住所".to_string()), "{words:?}");
        assert!(words.contains(&"チェック".to_string()), "{words:?}");
    }
}
