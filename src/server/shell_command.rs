//! Builds shell commands for Open With rules and Custom Actions.
//!
//! Commands may contain the tokens `{file}` (or `%1`), `{dir}`, `{target_pane}` and
//! `{selection}`. File names must never become part of the command text, because the
//! shell would read characters such as `$(...)` or backticks in them as instructions.
//!
//! On Unix the tokens are replaced by references to environment variables
//! (`BRUM_FILE`, `BRUM_DIR`, `BRUM_TARGET_PANE`) and the selection is passed as
//! positional arguments (`"$@"`). The shell expands those values as plain text.
//! A token's surrounding quotes are taken into account, so `"{file}"`, `'{file}'`,
//! `{file}` and `"{dir}/out.txt"` all keep working.
//!
//! On Windows (`cmd /C`) values are quoted into the text; paths containing a double
//! quote or control characters are refused, since they could end the quoted section.

use std::process::Command;

/// Values available to a command
pub struct CommandValues<'a> {
    pub file: &'a str,
    pub dir: &'a str,
    pub target_pane: &'a str,
    pub selection: &'a [String],
}

#[derive(Clone, Copy, PartialEq)]
enum Token {
    File,
    Dir,
    TargetPane,
    Selection,
}

const TOKENS: &[(&str, Token)] = &[
    ("{file}", Token::File),
    ("%1", Token::File),
    ("{dir}", Token::Dir),
    ("{target_pane}", Token::TargetPane),
    ("{selection}", Token::Selection),
];

fn token_at(s: &str) -> Option<(&'static str, Token)> {
    TOKENS.iter().find(|(t, _)| s.starts_with(t)).map(|(t, k)| (*t, *k))
}

fn env_name(t: Token) -> &'static str {
    match t {
        Token::File => "BRUM_FILE",
        Token::Dir => "BRUM_DIR",
        Token::TargetPane => "BRUM_TARGET_PANE",
        Token::Selection => "",
    }
}

/// A token written outside any quotes, or as a whole quoted word: one safe quoted value
fn quoted_ref(t: Token) -> String {
    match t {
        Token::Selection => "\"$@\"".to_string(),
        _ => format!("\"${{{}}}\"", env_name(t)),
    }
}

/// Rewrites a command template for `sh -c`, so values arrive only through the environment
pub fn unix_script(template: &str) -> String {
    #[derive(PartialEq)]
    enum Q {
        None,
        Single,
        Double,
    }
    let mut out = String::with_capacity(template.len() + 32);
    let mut q = Q::None;
    let mut i = 0;
    while i < template.len() {
        let rest = &template[i..];
        let c = rest.chars().next().unwrap();

        // A token that is the whole of a quoted word: "{file}" or '{file}'
        if q == Q::None && (c == '"' || c == '\'') {
            if let Some((tok, kind)) = token_at(&rest[1..]) {
                if rest[1 + tok.len()..].starts_with(c) {
                    out.push_str(&quoted_ref(kind));
                    i += tok.len() + 2;
                    continue;
                }
            }
        }

        if let Some((tok, kind)) = token_at(rest) {
            match q {
                Q::None => out.push_str(&quoted_ref(kind)),
                Q::Double => out.push_str(&match kind {
                    Token::Selection => "$*".to_string(),
                    _ => format!("${{{}}}", env_name(kind)),
                }),
                // Close the single quotes, insert the value, reopen them
                Q::Single => {
                    out.push('\'');
                    out.push_str(&quoted_ref(kind));
                    out.push('\'');
                }
            }
            i += tok.len();
            continue;
        }

        match (&q, c) {
            (Q::None, '\'') => q = Q::Single,
            (Q::Single, '\'') => q = Q::None,
            (Q::None, '"') => q = Q::Double,
            (Q::Double, '"') => q = Q::None,
            (Q::None | Q::Double, '\\') => {
                // Keep an escaped character as written
                out.push(c);
                i += 1;
                if let Some(n) = template[i..].chars().next() {
                    out.push(n);
                    i += n.len_utf8();
                }
                continue;
            }
            _ => {}
        }
        out.push(c);
        i += c.len_utf8();
    }
    out
}

fn windows_safe(v: &str) -> bool {
    !v.contains('"') && !v.chars().any(|c| c.is_control())
}

/// Readable form of the command, for messages only (never executed)
pub fn display_command(template: &str, v: &CommandValues) -> String {
    let sel = v.selection.iter().map(|s| format!("\"{}\"", s)).collect::<Vec<_>>().join(" ");
    let mut out = String::new();
    let mut i = 0;
    while i < template.len() {
        if let Some((tok, kind)) = token_at(&template[i..]) {
            match kind {
                Token::File => out.push_str(v.file),
                Token::Dir => out.push_str(v.dir),
                Token::TargetPane => out.push_str(v.target_pane),
                Token::Selection => out.push_str(&sel),
            }
            i += tok.len();
        } else {
            let c = template[i..].chars().next().unwrap();
            out.push(c);
            i += c.len_utf8();
        }
    }
    out
}

/// Builds the shell process for a command template. Errors name the refused value.
pub fn build_command(template: &str, v: &CommandValues) -> Result<Command, String> {
    if cfg!(target_os = "windows") {
        let all = [v.file, v.dir, v.target_pane].into_iter().chain(v.selection.iter().map(|s| s.as_str()));
        if let Some(bad) = all.into_iter().find(|s| !windows_safe(s)) {
            return Err(format!("Refusing to run: path contains a double quote or control character: {}", bad));
        }
        let mut text = String::new();
        let mut i = 0;
        while i < template.len() {
            if let Some((tok, kind)) = token_at(&template[i..]) {
                // Quotes the user already wrote around the token are reused
                let already_quoted = text.ends_with('"') && template[i + tok.len()..].starts_with('"');
                let value = match kind {
                    Token::File => v.file.to_string(),
                    Token::Dir => v.dir.to_string(),
                    Token::TargetPane => v.target_pane.to_string(),
                    Token::Selection => {
                        let joined = v.selection.iter().map(|s| format!("\"{}\"", s)).collect::<Vec<_>>().join(" ");
                        if already_quoted {
                            // "{selection}": drop the outer quotes, each path carries its own
                            text.pop();
                            text.push_str(&joined);
                            i += tok.len() + 1;
                            continue;
                        }
                        text.push_str(&joined);
                        i += tok.len();
                        continue;
                    }
                };
                if already_quoted {
                    text.push_str(&value);
                } else {
                    text.push_str(&format!("\"{}\"", value));
                }
                i += tok.len();
            } else {
                let c = template[i..].chars().next().unwrap();
                text.push(c);
                i += c.len_utf8();
            }
        }
        let mut cmd = Command::new("cmd");
        cmd.args(["/C", &text]).current_dir(v.dir);
        Ok(cmd)
    } else {
        let mut cmd = Command::new("sh");
        cmd.arg("-c")
            .arg(unix_script(template))
            .arg("brum") // $0
            .args(v.selection)
            .env("BRUM_FILE", v.file)
            .env("BRUM_DIR", v.dir)
            .env("BRUM_TARGET_PANE", v.target_pane)
            .current_dir(v.dir);
        Ok(cmd)
    }
}

/// What a command printed while Brum waited for it
pub struct RunOutcome {
    pub output: String,
    /// Exit code, or `None` while the command is still running (or was ended by a signal)
    pub exit_code: Option<i32>,
    pub still_running: bool,
    pub truncated: bool,
}

/// Runs a command and collects its combined output for up to `wait`. A command that takes
/// longer (for example one that opens a program) keeps running in the background.
pub async fn run_collecting_output(cmd: Command, wait: std::time::Duration, cap: usize) -> std::io::Result<RunOutcome> {
    use std::sync::{Arc, Mutex};
    use tokio::io::AsyncReadExt;

    let mut cmd = tokio::process::Command::from(cmd);
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let mut child = cmd.spawn()?;

    let buf = Arc::new(Mutex::new((Vec::<u8>::new(), false)));
    let mut readers = Vec::new();
    let pipes: Vec<Box<dyn tokio::io::AsyncRead + Unpin + Send>> = [
        child.stdout.take().map(|p| Box::new(p) as Box<dyn tokio::io::AsyncRead + Unpin + Send>),
        child.stderr.take().map(|p| Box::new(p) as Box<dyn tokio::io::AsyncRead + Unpin + Send>),
    ]
    .into_iter()
    .flatten()
    .collect();
    for mut pipe in pipes {
        let buf = buf.clone();
        // Keep reading past the cap, so a chatty command never stalls on a full pipe
        readers.push(tokio::spawn(async move {
            let mut chunk = [0u8; 8192];
            while let Ok(n) = pipe.read(&mut chunk).await {
                if n == 0 {
                    break;
                }
                let mut b = buf.lock().unwrap();
                let room = cap.saturating_sub(b.0.len());
                if n > room {
                    b.1 = true;
                }
                let take = n.min(room);
                b.0.extend_from_slice(&chunk[..take]);
            }
        }));
    }

    let finished = tokio::time::timeout(wait, child.wait()).await;
    if finished.is_ok() {
        // Programs started by the command may keep the output open; collect what is
        // already there, without waiting for them
        let _ = tokio::time::timeout(std::time::Duration::from_millis(300), async {
            for r in readers {
                let _ = r.await;
            }
        })
        .await;
    }

    let (exit_code, still_running) = match finished {
        Ok(status) => (status?.code(), false),
        Err(_) => (None, true),
    };
    let b = buf.lock().unwrap();
    Ok(RunOutcome {
        output: String::from_utf8_lossy(&b.0).to_string(),
        exit_code,
        still_running,
        truncated: b.1,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_become_quoted_variable_references() {
        assert_eq!(unix_script("wc -l {file}"), "wc -l \"${BRUM_FILE}\"");
        assert_eq!(unix_script("wc -l \"{file}\""), "wc -l \"${BRUM_FILE}\"");
        assert_eq!(unix_script("echo '{file}'"), "echo \"${BRUM_FILE}\"");
        assert_eq!(unix_script("code \"%1\""), "code \"${BRUM_FILE}\"");
        assert_eq!(unix_script("cp {selection} {target_pane}"), "cp \"$@\" \"${BRUM_TARGET_PANE}\"");
        assert_eq!(unix_script("git -C \"{dir}\" log"), "git -C \"${BRUM_DIR}\" log");
    }

    #[test]
    fn tokens_inside_longer_quoted_strings() {
        assert_eq!(unix_script("cat > \"{dir}/out.txt\""), "cat > \"${BRUM_DIR}/out.txt\"");
        assert_eq!(unix_script("echo 'name: {file} done'"), "echo 'name: '\"${BRUM_FILE}\"' done'");
    }

    #[test]
    fn display_shows_real_paths() {
        let sel = vec!["/a b".to_string()];
        let v = CommandValues { file: "/x/y.txt", dir: "/x", target_pane: "/t", selection: &sel };
        assert_eq!(display_command("wc -l \"{file}\" {selection}", &v), "wc -l \"/x/y.txt\" \"/a b\"");
    }

    #[cfg(unix)]
    fn run_with_hostile_names(template: &str) -> (String, bool) {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("INJECTED");
        // Every special shell construct a file name could carry
        // Commands run in `dir`, so a bare `touch INJECTED` would create the marker there
        let hostile = "a$(touch INJECTED)`touch INJECTED`\"; touch INJECTED; \"$((1))'; touch INJECTED; ' b.txt";
        let file = dir.path().join(hostile);
        std::fs::write(&file, "one\ntwo\n").unwrap();
        let file_s = file.to_string_lossy().to_string();
        let dir_s = dir.path().to_string_lossy().to_string();
        let sel = vec![file_s.clone(), file_s.clone()];
        let v = CommandValues { file: &file_s, dir: &dir_s, target_pane: &file_s, selection: &sel };
        let out = build_command(template, &v).unwrap().output().unwrap();
        (String::from_utf8_lossy(&out.stdout).to_string(), marker.exists())
    }

    #[cfg(unix)]
    #[test]
    fn hostile_file_names_never_run_as_commands() {
        for template in [
            "wc -l \"{file}\"",
            "wc -l {file}",
            "cat '{file}'",
            "cat {selection}",
            "cat \"{selection}\"",
            "echo \"{target_pane}\" >/dev/null; cat \"%1\"",
            "echo 'x {file} y'",
            "ls \"{dir}\" >/dev/null",
        ] {
            let (stdout, injected) = run_with_hostile_names(template);
            assert!(!injected, "file name ran as a command with template: {template}");
            assert!(!stdout.is_empty() || template.starts_with("ls"), "no output for template: {template}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn file_with_spaces_is_one_argument() {
        let (stdout, _) = run_with_hostile_names("wc -l \"{file}\"");
        assert!(stdout.trim_start().starts_with('2'), "unexpected: {stdout}");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn output_is_collected_and_long_commands_keep_running() {
        let quick = build_command("echo hello; echo oops >&2; exit 3", &CommandValues { file: "", dir: "/tmp", target_pane: "", selection: &[] }).unwrap();
        let r = run_collecting_output(quick, std::time::Duration::from_secs(5), 1024).await.unwrap();
        assert!(r.output.contains("hello") && r.output.contains("oops"), "output: {}", r.output);
        assert_eq!(r.exit_code, Some(3));
        assert!(!r.still_running && !r.truncated);

        let slow = build_command("echo started; sleep 3", &CommandValues { file: "", dir: "/tmp", target_pane: "", selection: &[] }).unwrap();
        let r = run_collecting_output(slow, std::time::Duration::from_millis(500), 1024).await.unwrap();
        assert!(r.still_running);
        assert!(r.output.contains("started"));

        let chatty = build_command("yes | head -c 100000", &CommandValues { file: "", dir: "/tmp", target_pane: "", selection: &[] }).unwrap();
        let r = run_collecting_output(chatty, std::time::Duration::from_secs(5), 1000).await.unwrap();
        assert_eq!(r.output.len(), 1000);
        assert!(r.truncated && !r.still_running);

        // A command that starts a long-lived program returns without waiting for it
        let launcher = build_command("echo launched; sleep 5 &", &CommandValues { file: "", dir: "/tmp", target_pane: "", selection: &[] }).unwrap();
        let t = std::time::Instant::now();
        let r = run_collecting_output(launcher, std::time::Duration::from_secs(4), 1024).await.unwrap();
        assert!(t.elapsed() < std::time::Duration::from_secs(2), "waited for the started program");
        assert!(r.output.contains("launched") && !r.still_running);
    }

    #[test]
    fn windows_refuses_quotes_in_paths() {
        assert!(!windows_safe("C:\\a\"b"));
        assert!(windows_safe("C:\\100% done.txt"));
    }
}
