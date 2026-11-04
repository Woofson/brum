//! Which files and folders a backup skips.
//!
//! Patterns, one per line (or comma separated), matched without regard to case:
//! - `node_modules`, `.git`, `Thumbs.db`: any file or folder with exactly this name
//! - `*.tmp`, `~*`, `cache?`: a name with wildcards (`*` any text, `?` one character)
//! - `Photos/Raw`, `build/*.o`: a path from the top of the backup (contains a `/`)

#[derive(Debug, Clone, Default)]
pub struct Exclusions {
    names: Vec<String>,
    paths: Vec<String>,
}

impl Exclusions {
    pub fn parse(text: &str) -> Self {
        let mut ex = Exclusions::default();
        for raw in text.split(['\n', ',', ';']) {
            let pat = raw.trim().trim_matches('/').to_lowercase();
            if pat.is_empty() {
                continue;
            }
            if pat.contains('/') {
                ex.paths.push(pat);
            } else {
                ex.names.push(pat);
            }
        }
        ex
    }

    pub fn from_list(list: &[String]) -> Self {
        Self::parse(&list.join("\n"))
    }

    pub fn is_empty(&self) -> bool {
        self.names.is_empty() && self.paths.is_empty()
    }

    /// `rel` is a path inside the backup with `/` between parts
    pub fn is_excluded(&self, rel: &str) -> bool {
        if self.is_empty() {
            return false;
        }
        let lower = rel.to_lowercase();
        let parts: Vec<&str> = lower.split('/').filter(|p| !p.is_empty()).collect();
        if self.names.iter().any(|pat| parts.iter().any(|part| glob_match(pat, part))) {
            return true;
        }
        // A path pattern matches the path itself or any folder above it
        self.paths.iter().any(|pat| (1..=parts.len()).any(|n| glob_match(pat, &parts[..n].join("/"))))
    }
}

/// `*` matches any text (not across `/` in path patterns), `?` one character
fn glob_match(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    let (mut pi, mut ti) = (0usize, 0usize);
    let (mut star, mut mark) = (None::<usize>, 0usize);
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' && t[ti] != '/' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ti;
            pi += 1;
        } else if let Some(s) = star {
            if t[mark] == '/' {
                return false;
            }
            pi = s + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_match_whole_parts_only() {
        let ex = Exclusions::parse(".git, node_modules");
        assert!(ex.is_excluded(".git"));
        assert!(ex.is_excluded("project/.git/config"));
        assert!(ex.is_excluded("web/node_modules/a/b.js"));
        assert!(!ex.is_excluded("project/.gitignore"));
        assert!(!ex.is_excluded("my.github.io/index.html"));
    }

    #[test]
    fn wildcards() {
        let ex = Exclusions::parse("*.tmp\n~*\ncache?");
        assert!(ex.is_excluded("a/b/file.TMP"));
        assert!(ex.is_excluded("~lock.docx"));
        assert!(ex.is_excluded("x/cache1/y"));
        assert!(!ex.is_excluded("attempt.txt"));
        assert!(!ex.is_excluded("x/cache12/y"));
    }

    #[test]
    fn paths_from_the_top() {
        let ex = Exclusions::parse("/Photos/Raw/, build/*.o");
        assert!(ex.is_excluded("photos/raw"));
        assert!(ex.is_excluded("Photos/Raw/img1.cr2"));
        assert!(!ex.is_excluded("Old/Photos/Raw/img1.cr2"));
        assert!(ex.is_excluded("build/main.o"));
        assert!(!ex.is_excluded("build/sub/main.o"));
    }

    #[test]
    fn empty_lines_ignored() {
        let ex = Exclusions::parse("\n , ;\n");
        assert!(ex.is_empty());
        assert!(!ex.is_excluded("anything"));
    }
}
