use regex::Regex;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

/// Maximum number of patterns kept in cache before clearing everything.
const CACHE_CAPACITY: usize = 256;

fn cache() -> &'static Mutex<HashMap<String, Regex>> {
    static CACHE: OnceLock<Mutex<HashMap<String, Regex>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Compiles (or reuses) a regex, avoiding recompiling the same pattern on each
/// record/validation. The cloned `Regex` shares the program internally.
pub fn compiled(pattern: &str) -> Result<Regex, regex::Error> {
    if let Ok(guard) = cache().lock()
        && let Some(regex) = guard.get(pattern)
    {
        return Ok(regex.clone());
    }
    let regex = Regex::new(pattern)?;
    if let Ok(mut guard) = cache().lock() {
        if guard.len() >= CACHE_CAPACITY {
            guard.clear();
        }
        guard.insert(pattern.to_string(), regex.clone());
    }
    Ok(regex)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compiles_and_reuses_patterns() {
        let first = compiled("^a.*z$").unwrap();
        let second = compiled("^a.*z$").unwrap();
        assert!(first.is_match("abcz"));
        assert!(second.is_match("abcz"));
    }

    #[test]
    fn invalid_pattern_errors() {
        assert!(compiled("(unclosed").is_err());
    }
}
