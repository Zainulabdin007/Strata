//! Lightweight path glob matching for ingest discovery (`*` and `**` only).

/// Returns true when `path` (slash-separated relative path) matches `pattern`.
///
/// Supported:
/// - `*` — any characters except `/`
/// - `**` — any path segments (including `/`)
/// - other characters match literally (case-sensitive)
pub fn glob_matches(pattern: &str, path: &str) -> bool {
    let pattern = pattern.strip_prefix("./").unwrap_or(pattern);
    let path = path.strip_prefix("./").unwrap_or(path);
    // Two DP rows: O(pattern * path) work, O(path) space, no recursion.
    let p = pattern.as_bytes();
    let text = path.as_bytes();
    let mut next = vec![false; text.len() + 1];
    next[text.len()] = true;
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < p.len() {
        if p[i..].starts_with(b"**") {
            tokens.push((b'*', true));
            i += 2;
            if p.get(i) == Some(&b'/') {
                i += 1;
            }
        } else {
            tokens.push((p[i], false));
            i += 1;
        }
    }
    for (token, recursive) in tokens.into_iter().rev() {
        let mut row = vec![false; text.len() + 1];
        if token == b'*' {
            row[text.len()] = next[text.len()];
        }
        for j in (0..text.len()).rev() {
            row[j] = if token == b'*' {
                next[j] || ((recursive || text[j] != b'/') && row[j + 1])
            } else {
                (token == text[j] || (token == b'?' && text[j] != b'/')) && next[j + 1]
            };
        }
        next = row;
    }
    next[0]
}

/// True if `relative` is accepted by include/exclude lists.
pub fn path_allowed(relative: &str, include: &[String], exclude: &[String]) -> bool {
    let included = if include.is_empty() {
        true
    } else {
        include.iter().any(|g| glob_matches(g, relative))
    };
    if !included {
        return false;
    }
    !exclude.iter().any(|g| glob_matches(g, relative))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn star_and_recursive() {
        assert!(glob_matches("*.csv", "a.csv"));
        assert!(!glob_matches("*.csv", "dir/a.csv"));
        assert!(glob_matches("**/*.csv", "dir/a.csv"));
        assert!(glob_matches("**/market/*.csv", "data/market/x.csv"));
        assert!(!glob_matches("**/market/*.csv", "data/other/x.csv"));
    }

    #[test]
    fn include_exclude() {
        let include = vec!["**/*.csv".into()];
        let exclude = vec!["**/skip/**".into()];
        assert!(path_allowed("a/b.csv", &include, &exclude));
        assert!(!path_allowed("a/skip/b.csv", &include, &exclude));
    }
}
