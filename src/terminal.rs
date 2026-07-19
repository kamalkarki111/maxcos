//! Sandboxed terminal — virtual FS under data/cache only; host paths blocked.
use crate::store::Store;
use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::process::Command;
use tokio::time::timeout;

const HOST_ALLOW: &[&str] = &[
    "uname", "date", "whoami", "hostname", "uptime", "df", "id", "arch",
    "pwd", "echo", "printf", "true", "false", "yes", "cal", "env", "printenv",
    "which", "type", "file", "wc", "head", "tail", "sort", "uniq", "grep",
    "egrep", "fgrep", "find", "stat", "ls", "cat", "mkdir", "touch", "rm",
    "cp", "mv", "clear", "help", "neofetch", "history",
];

/// Commands that accept filesystem path arguments and must stay inside the sandbox.
const PATH_CMDS: &[&str] = &[
    "cat", "ls", "file", "wc", "head", "tail", "stat", "rm", "cp", "mv",
    "mkdir", "touch", "find", "grep", "egrep", "fgrep", "sort", "uniq",
];

const BLOCKED_PATTERNS: &[&str] = &[
    "rm -rf /", "rm -rf/*", ":(){", "mkfs", "dd if=", "shutdown", "reboot",
    "halt", "poweroff", "launchctl", "sudo", "su ", "chmod 777 /", "> /dev/",
    "curl ", "wget ", "nc ", "ncat ", "ssh ", "scp ", "ftp ", "telnet ",
    "python -c", "python3 -c", "perl -e", "ruby -e", "osascript", "defaults ",
    "/etc/passwd", "/etc/shadow", "/etc/hosts", "/private/etc/",
];

pub struct TermResult {
    pub output: String,
    pub cwd: String,
    pub exit_code: i32,
}

pub async fn run_command(store: &Store, user_id: &str, cmd: &str, cwd_virt: &str) -> TermResult {
    let cmd = cmd.trim();
    if cmd.is_empty() {
        return TermResult { output: String::new(), cwd: cwd_virt.into(), exit_code: 0 };
    }
    if cmd == "clear" || cmd == "cls" {
        return TermResult { output: "__CLEAR__".into(), cwd: cwd_virt.into(), exit_code: 0 };
    }
    if cmd == "exit" || cmd == "logout" {
        return TermResult { output: "__EXIT__".into(), cwd: cwd_virt.into(), exit_code: 0 };
    }
    if cmd == "help" || cmd == "?" {
        return TermResult {
            output: "\x1b[1;36mMaxcos Terminal\x1b[0m — sandboxed shell\n\
\x1b[33mVirtual FS:\x1b[0m ls, cd, pwd, cat, mkdir, touch, rm, cp, mv, echo\n\
\x1b[33mHost (read-mostly):\x1b[0m uname, date, whoami, hostname, df, uptime, env, which\n\
Working directory is locked under data/cache/<user>/fs. Absolute host paths are blocked."
                .into(),
            cwd: cwd_virt.into(),
            exit_code: 0,
        };
    }

    for p in BLOCKED_PATTERNS {
        if cmd.to_lowercase().contains(&p.to_lowercase()) {
            return TermResult {
                output: format!("\x1b[31mzsh: blocked for safety: pattern `{p}`\x1b[0m"),
                cwd: cwd_virt.into(),
                exit_code: 126,
            };
        }
    }

    // Reject obvious absolute host paths in the raw command line
    if let Some(bad) = first_absolute_host_path(cmd) {
        return TermResult {
            output: format!(
                "\x1b[31mzsh: sandbox: absolute path outside data/cache denied: {bad}\x1b[0m"
            ),
            cwd: cwd_virt.into(),
            exit_code: 126,
        };
    }

    let parts: Vec<&str> = shell_split(cmd);
    if parts.is_empty() {
        return TermResult { output: String::new(), cwd: cwd_virt.into(), exit_code: 0 };
    }
    let bin = parts[0];
    let args = &parts[1..];

    match bin {
        "cd" => {
            let target = args.first().copied().unwrap_or("~");
            if looks_like_host_absolute(target) {
                return TermResult {
                    output: format!("\x1b[31mcd: sandbox: absolute path denied: {target}\x1b[0m"),
                    cwd: cwd_virt.into(),
                    exit_code: 1,
                };
            }
            let new_cwd = resolve_cd(cwd_virt, target);
            match store.resolve_virtual(user_id, &virt_from_cwd(&new_cwd)) {
                Ok(p) if p.is_dir() => {
                    TermResult { output: String::new(), cwd: new_cwd, exit_code: 0 }
                }
                Ok(_) => TermResult {
                    output: format!("cd: not a directory: {target}"),
                    cwd: cwd_virt.into(),
                    exit_code: 1,
                },
                Err(e) => TermResult {
                    output: format!("cd: {e}"),
                    cwd: cwd_virt.into(),
                    exit_code: 1,
                },
            }
        }
        "pwd" => {
            let display = if cwd_virt == "~" || cwd_virt == "/Users/maxcos" {
                "/Users/maxcos".into()
            } else {
                cwd_virt.to_string()
            };
            TermResult { output: display, cwd: cwd_virt.into(), exit_code: 0 }
        }
        "neofetch" => TermResult {
            output: "\x1b[36m                    'c.\n                 ,xNMM.\n               .OMMMMo\n               OMMM0,\n     .;loddo:' loolloddol;.\n   cKMMMMMMMMMMNWMMMMMMMMMM0:\x1b[0m\n\n \x1b[1mmaxcos@Maxcos-MacBook-Pro\x1b[0m\n \x1b[36m─────────────────────────\x1b[0m\n \x1b[33mOS\x1b[0m: macOS Sequoia 15.0 (Maxcos)\n \x1b[33mHost\x1b[0m: MacBook Pro (Sim)\n \x1b[33mShell\x1b[0m: zsh (sandboxed)\n \x1b[33mTerminal\x1b[0m: Maxcos Terminal\n \x1b[33mFS\x1b[0m: data/cache sandbox".into(),
            cwd: cwd_virt.into(),
            exit_code: 0,
        },
        _ => {
            let base_bin = PathBuf::from(bin)
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| bin.to_string());
            let allow: HashSet<&str> = HOST_ALLOW.iter().copied().collect();
            if !allow.contains(base_bin.as_str()) && !allow.contains(bin) {
                return TermResult {
                    output: format!(
                        "\x1b[31mzsh: command not found: {bin}\x1b[0m\n(type \x1b[33mhelp\x1b[0m for allowed commands)"
                    ),
                    cwd: cwd_virt.into(),
                    exit_code: 127,
                };
            }

            // Resolve sandbox roots
            let virt = virt_from_cwd(cwd_virt);
            let real_cwd = match store.resolve_virtual(user_id, &virt) {
                Ok(p) => p,
                Err(e) => {
                    return TermResult {
                        output: format!("cwd error: {e}"),
                        cwd: cwd_virt.into(),
                        exit_code: 1,
                    };
                }
            };
            if !real_cwd.is_dir() {
                let _ = store.fs_mkdir(user_id, &virt);
            }
            let real_cwd = store
                .resolve_virtual(user_id, &virt)
                .unwrap_or(real_cwd);
            let sandbox_root = match store.resolve_virtual(user_id, "~") {
                Ok(p) => p,
                Err(e) => {
                    return TermResult {
                        output: format!("sandbox error: {e}"),
                        cwd: cwd_virt.into(),
                        exit_code: 1,
                    };
                }
            };

            // Path-sensitive commands: validate every non-flag arg stays in sandbox
            let path_sensitive = PATH_CMDS.contains(&base_bin.as_str());
            let mut safe_args: Vec<String> = Vec::new();
            if path_sensitive {
                for a in args {
                    if a.starts_with('-') && *a != "-" {
                        safe_args.push((*a).to_string());
                        continue;
                    }
                    // grep pattern (first non-flag) is not always a path — for grep/egrep/fgrep,
                    // treat only args that look path-like as paths
                    if matches!(base_bin.as_str(), "grep" | "egrep" | "fgrep")
                        && !looks_path_like(a)
                    {
                        safe_args.push((*a).to_string());
                        continue;
                    }
                    match resolve_in_sandbox(&sandbox_root, &real_cwd, a) {
                        Ok(p) => safe_args.push(p.to_string_lossy().into_owned()),
                        Err(e) => {
                            return TermResult {
                                output: format!("\x1b[31mzsh: sandbox: {e}\x1b[0m"),
                                cwd: cwd_virt.into(),
                                exit_code: 126,
                            };
                        }
                    }
                }
            } else {
                // Non-path commands still cannot pass absolute host paths as args
                for a in args {
                    if looks_like_host_absolute(a) || a.contains("..") {
                        // allow ".." only if not path-like for echo etc.
                        if looks_path_like(a) {
                            return TermResult {
                                output: format!(
                                    "\x1b[31mzsh: sandbox: path outside data/cache denied: {a}\x1b[0m"
                                ),
                                cwd: cwd_virt.into(),
                                exit_code: 126,
                            };
                        }
                    }
                    safe_args.push((*a).to_string());
                }
            }

            let output =
                run_host_binary(&base_bin, &safe_args, &real_cwd, &sandbox_root).await;
            TermResult {
                output,
                cwd: cwd_virt.into(),
                exit_code: 0,
            }
        }
    }
}

fn looks_path_like(s: &str) -> bool {
    s.starts_with('/')
        || s.starts_with("./")
        || s.starts_with("../")
        || s.starts_with('~')
        || s.contains('/')
}

fn looks_like_host_absolute(s: &str) -> bool {
    if !s.starts_with('/') {
        return false;
    }
    // Virtual mac paths under /Users/maxcos are mapped by resolve_virtual — treat others as host
    if s == "/Users/maxcos" || s.starts_with("/Users/maxcos/") {
        return false;
    }
    true
}

fn first_absolute_host_path(cmd: &str) -> Option<String> {
    for tok in shell_split(cmd) {
        if looks_like_host_absolute(tok) {
            return Some(tok.to_string());
        }
        // bare ../../etc/passwd style
        if tok.contains("..") && (tok.contains("etc") || tok.starts_with("../")) {
            // not definitive; still check at resolve time
        }
    }
    None
}

/// Resolve a user-supplied path so it is strictly under sandbox_root (data/cache/.../fs).
fn resolve_in_sandbox(
    sandbox_root: &Path,
    cwd: &Path,
    arg: &str,
) -> Result<PathBuf, String> {
    if arg.is_empty() || arg == "-" {
        return Err("empty path".into());
    }
    if looks_like_host_absolute(arg) {
        return Err(format!("absolute path outside data/cache denied: {arg}"));
    }

    // Map virtual home
    let raw = if arg == "~" || arg == "/Users/maxcos" {
        sandbox_root.to_path_buf()
    } else if let Some(rest) = arg
        .strip_prefix("~/")
        .or_else(|| arg.strip_prefix("/Users/maxcos/"))
    {
        sandbox_root.join(rest)
    } else if arg.starts_with('/') {
        return Err(format!("absolute path outside data/cache denied: {arg}"));
    } else {
        cwd.join(arg)
    };

    // Normalize .. without leaving root
    let mut out = PathBuf::new();
    let abs = if raw.is_absolute() {
        raw
    } else {
        cwd.join(arg)
    };
    for comp in abs.components() {
        match comp {
            Component::RootDir => out.push("/"),
            Component::Normal(s) => out.push(s),
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    return Err("path escapes sandbox (..)".into());
                }
            }
            Component::Prefix(_) => return Err("invalid path".into()),
        }
    }

    let root_canon = sandbox_root
        .canonicalize()
        .unwrap_or_else(|_| sandbox_root.to_path_buf());
    // If target doesn't exist yet (touch/mkdir), check parent chain
    if out.exists() {
        let c = out.canonicalize().map_err(|e| e.to_string())?;
        if !c.starts_with(&root_canon) {
            return Err(format!("path escapes sandbox: {arg}"));
        }
        return Ok(c);
    }

    // Non-existent path (touch/mkdir): ensure an existing ancestor stays in sandbox
    let mut ancestor = out.clone();
    while !ancestor.exists() {
        if !ancestor.pop() {
            break;
        }
    }
    let anc = ancestor
        .canonicalize()
        .unwrap_or_else(|_| ancestor.clone());
    if !anc.starts_with(&root_canon) && anc != root_canon {
        return Err(format!("path escapes sandbox: {arg}"));
    }
    if out.starts_with(&root_canon) || out.starts_with(sandbox_root) {
        Ok(out)
    } else {
        let root_str = root_canon.to_string_lossy();
        let out_str = out.to_string_lossy();
        if out_str.starts_with(root_str.as_ref()) {
            Ok(out)
        } else {
            Err(format!("path escapes sandbox: {arg}"))
        }
    }
}

async fn run_host_binary(
    bin: &str,
    args: &[String],
    cwd: &Path,
    sandbox_root: &Path,
) -> String {
    // Refuse cwd outside sandbox
    let root_canon = sandbox_root
        .canonicalize()
        .unwrap_or_else(|_| sandbox_root.to_path_buf());
    let cwd_canon = cwd.canonicalize().unwrap_or_else(|_| cwd.to_path_buf());
    if !cwd_canon.starts_with(&root_canon) {
        return "\x1b[31merror: working directory outside sandbox\x1b[0m".into();
    }

    let path = resolve_bin(bin);
    let mut command = Command::new(&path);
    command
        .args(args)
        .current_dir(&cwd_canon)
        .env_clear()
        .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
        .env("HOME", &cwd_canon)
        .env("USER", "maxcos")
        .env("LOGNAME", "maxcos")
        .env("TERM", "xterm-256color")
        .env("LANG", "en_US.UTF-8")
        .env("CLICOLOR", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    let fut = async {
        let child = command.spawn().map_err(|e| e.to_string())?;
        let out = child.wait_with_output().await.map_err(|e| e.to_string())?;
        let mut s = String::new();
        s.push_str(&String::from_utf8_lossy(&out.stdout));
        if !out.stderr.is_empty() {
            if !s.is_empty() && !s.ends_with('\n') {
                s.push('\n');
            }
            s.push_str(&String::from_utf8_lossy(&out.stderr));
        }
        if s.ends_with('\n') {
            s.pop();
            if s.ends_with('\r') {
                s.pop();
            }
        }
        Ok::<String, String>(s)
    };

    match timeout(Duration::from_secs(8), fut).await {
        Ok(Ok(s)) => s,
        Ok(Err(e)) => format!("\x1b[31merror: {e}\x1b[0m"),
        Err(_) => "\x1b[31merror: command timed out (8s)\x1b[0m".into(),
    }
}

fn resolve_bin(bin: &str) -> PathBuf {
    for root in ["/bin", "/usr/bin", "/usr/sbin", "/sbin"] {
        let p = PathBuf::from(root).join(bin);
        if p.is_file() {
            return p;
        }
    }
    PathBuf::from(bin)
}

fn virt_from_cwd(cwd: &str) -> String {
    if cwd == "~" || cwd == "/Users/maxcos" {
        "~".into()
    } else if let Some(rest) = cwd.strip_prefix("/Users/maxcos/") {
        format!("~/{rest}")
    } else if cwd.starts_with("~/") || cwd == "~" {
        cwd.into()
    } else if cwd == "/" || cwd == "/Users" {
        "~".into()
    } else {
        cwd.into()
    }
}

fn resolve_cd(cwd: &str, target: &str) -> String {
    match target {
        "~" | "~/" | "" => "~".into(),
        ".." => {
            if cwd == "~" || cwd == "/Users/maxcos" {
                "~".into()
            } else {
                let v = virt_from_cwd(cwd);
                let trimmed = v.trim_end_matches('/');
                match trimmed.rsplit_once('/') {
                    Some((p, _)) if p.is_empty() || p == "~" => "~".into(),
                    Some((p, _)) => p.to_string(),
                    None => "~".into(),
                }
            }
        }
        t if t.starts_with("~/") || t == "~" => t.to_string(),
        t if t.starts_with("/Users/maxcos") => t.to_string(),
        t if t.starts_with('/') => "~".into(), // clamp absolute host paths
        t => {
            let base = if cwd == "~" {
                "/Users/maxcos".to_string()
            } else {
                cwd.to_string()
            };
            // Prevent virtual .. escape
            let joined = if base.ends_with('/') {
                format!("{base}{t}")
            } else {
                format!("{base}/{t}")
            };
            // Normalize .. segments in virtual space
            let mut stack: Vec<&str> = Vec::new();
            let path = joined
                .trim_start_matches("/Users/maxcos/")
                .trim_start_matches("~/");
            for part in path.split('/') {
                if part.is_empty() || part == "." {
                    continue;
                }
                if part == ".." {
                    stack.pop();
                } else {
                    stack.push(part);
                }
            }
            if stack.is_empty() {
                "~".into()
            } else {
                format!("~/{}", stack.join("/"))
            }
        }
    }
}

fn shell_split(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = None;
    let mut chars = s.char_indices().peekable();
    let mut in_s = false;
    let mut in_d = false;
    while let Some((i, c)) = chars.next() {
        match c {
            '\'' if !in_d => in_s = !in_s,
            '"' if !in_s => in_d = !in_d,
            ' ' | '\t' if !in_s && !in_d => {
                if let Some(st) = start.take() {
                    out.push(&s[st..i]);
                }
            }
            _ => {
                if start.is_none() {
                    start = Some(i);
                }
            }
        }
    }
    if let Some(st) = start {
        out.push(&s[st..]);
    }
    out.into_iter()
        .map(|t| {
            if (t.starts_with('"') && t.ends_with('"') && t.len() >= 2)
                || (t.starts_with('\'') && t.ends_with('\'') && t.len() >= 2)
            {
                &t[1..t.len() - 1]
            } else {
                t
            }
        })
        .collect()
}
