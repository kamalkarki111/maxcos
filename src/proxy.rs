//! Safari web proxy — fetch remote pages and rewrite links for iframe use.
use regex::Regex;
use reqwest::header::{CONTENT_TYPE, USER_AGENT};
use std::sync::OnceLock;
use url::Url;

static HREF_RE: OnceLock<Regex> = OnceLock::new();
static SRC_RE: OnceLock<Regex> = OnceLock::new();
static ACTION_RE: OnceLock<Regex> = OnceLock::new();
static CSS_URL_RE: OnceLock<Regex> = OnceLock::new();
static SRCSET_RE: OnceLock<Regex> = OnceLock::new();

fn href_re() -> &'static Regex {
    HREF_RE.get_or_init(|| Regex::new(r#"(?i)(\bhref\s*=\s*)(["'])([^"']*)(["'])"#).unwrap())
}
fn src_re() -> &'static Regex {
    SRC_RE.get_or_init(|| Regex::new(r#"(?i)(\bsrc\s*=\s*)(["'])([^"']*)(["'])"#).unwrap())
}
fn action_re() -> &'static Regex {
    ACTION_RE.get_or_init(|| Regex::new(r#"(?i)(\baction\s*=\s*)(["'])([^"']*)(["'])"#).unwrap())
}
fn css_url_re() -> &'static Regex {
    CSS_URL_RE.get_or_init(|| Regex::new(r#"url\(\s*(['"]?)([^'")]+)\1\s*\)"#).unwrap())
}
fn srcset_re() -> &'static Regex {
    SRCSET_RE.get_or_init(|| Regex::new(r#"(?i)(\bsrcset\s*=\s*)(["'])([^"']*)(["'])"#).unwrap())
}

pub struct Proxied {
    pub bytes: Vec<u8>,
    pub content_type: String,
    pub final_url: String,
    pub title: Option<String>,
}

pub async fn fetch_proxied(raw_url: &str) -> Result<Proxied, String> {
    let url = normalize_url(raw_url)?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err("only http/https allowed".into());
    }
    // Block obvious local/private targets for safety
    if let Some(host) = url.host_str() {
        let h = host.to_lowercase();
        if h == "localhost" || h == "127.0.0.1" || h == "0.0.0.0" || h == "[::1]" || h.ends_with(".local") {
            return Err("local addresses are blocked".into());
        }
    }

    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::limited(8))
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|e| e.to_string())?;

    let resp = client
        .get(url.clone())
        .header(USER_AGENT, "Mozilla/5.0 (Macintosh; Intel Mac OS X 14_0) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Safari/605.1.15 Maxcos/1.0")
        .send()
        .await
        .map_err(|e| format!("fetch failed: {e}"))?;

    let final_url = resp.url().to_string();
    let status = resp.status();
    if !status.is_success() && !status.is_redirection() {
        // still try body for error pages
    }
    let ct = resp
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/octet-stream")
        .to_string();
    let bytes = resp.bytes().await.map_err(|e| e.to_string())?.to_vec();

    let base = Url::parse(&final_url).map_err(|e| e.to_string())?;
    let content_type = ct.clone();
    let is_html = ct.to_lowercase().contains("text/html")
        || looks_like_html(&bytes);
    let is_css = ct.to_lowercase().contains("text/css") || final_url.ends_with(".css");

    if is_html {
        let html = String::from_utf8_lossy(&bytes).into_owned();
        let title = extract_title(&html);
        let rewritten = rewrite_html(&html, &base);
        Ok(Proxied {
            bytes: rewritten.into_bytes(),
            content_type: "text/html; charset=utf-8".into(),
            final_url,
            title,
        })
    } else if is_css {
        let css = String::from_utf8_lossy(&bytes).into_owned();
        let rewritten = rewrite_css(&css, &base);
        Ok(Proxied {
            bytes: rewritten.into_bytes(),
            content_type: "text/css; charset=utf-8".into(),
            final_url,
            title: None,
        })
    } else {
        Ok(Proxied {
            bytes,
            content_type,
            final_url,
            title: None,
        })
    }
}

pub fn normalize_url(raw: &str) -> Result<Url, String> {
    let t = raw.trim();
    if t.is_empty() {
        return Err("empty url".into());
    }
    if t == "about:start" || t == "about:blank" {
        return Err("about page".into());
    }
    let candidate = if t.starts_with("http://") || t.starts_with("https://") {
        t.to_string()
    } else if t.contains(' ') || !t.contains('.') {
        format!("https://duckduckgo.com/?q={}", urlencoding_lite(t))
    } else {
        format!("https://{t}")
    };
    Url::parse(&candidate).map_err(|e| e.to_string())
}

fn urlencoding_lite(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            b' ' => out.push_str("%20"),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn looks_like_html(bytes: &[u8]) -> bool {
    let head = String::from_utf8_lossy(&bytes[..bytes.len().min(256)]).to_lowercase();
    head.contains("<html") || head.contains("<!doctype")
}

fn extract_title(html: &str) -> Option<String> {
    let re = Regex::new(r"(?is)<title[^>]*>(.*?)</title>").ok()?;
    re.captures(html).map(|c| {
        html_decode(&c[1]).trim().chars().take(120).collect()
    })
}

fn html_decode(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
}

fn proxy_url(absolute: &str) -> String {
    format!("/proxy?url={}", urlencoding_lite(absolute))
}

fn resolve_attr(base: &Url, value: &str) -> Option<String> {
    let v = value.trim();
    if v.is_empty() || v.starts_with('#') || v.starts_with("javascript:") || v.starts_with("data:") || v.starts_with("mailto:") || v.starts_with("tel:") {
        return None;
    }
    if v.starts_with("/proxy?") {
        return None; // already proxied
    }
    base.join(v).ok().map(|u| u.to_string())
}

fn rewrite_html(html: &str, base: &Url) -> String {
    let mut out = href_re().replace_all(html, |caps: &regex::Captures| {
        let prefix = &caps[1];
        let q1 = &caps[2];
        let val = &caps[3];
        let q2 = &caps[4];
        if let Some(abs) = resolve_attr(base, val) {
            format!("{prefix}{q1}{}{q2}", proxy_url(&abs))
        } else {
            caps[0].to_string()
        }
    }).into_owned();

    out = src_re().replace_all(&out, |caps: &regex::Captures| {
        let prefix = &caps[1];
        let q1 = &caps[2];
        let val = &caps[3];
        let q2 = &caps[4];
        if let Some(abs) = resolve_attr(base, val) {
            format!("{prefix}{q1}{}{q2}", proxy_url(&abs))
        } else {
            caps[0].to_string()
        }
    }).into_owned();

    out = action_re().replace_all(&out, |caps: &regex::Captures| {
        let prefix = &caps[1];
        let q1 = &caps[2];
        let val = &caps[3];
        let q2 = &caps[4];
        if let Some(abs) = resolve_attr(base, val) {
            format!("{prefix}{q1}{}{q2}", proxy_url(&abs))
        } else {
            caps[0].to_string()
        }
    }).into_owned();

    out = srcset_re().replace_all(&out, |caps: &regex::Captures| {
        let prefix = &caps[1];
        let q1 = &caps[2];
        let val = &caps[3];
        let q2 = &caps[4];
        let rewritten = val
            .split(',')
            .map(|part| {
                let part = part.trim();
                let mut bits = part.split_whitespace();
                let u = bits.next().unwrap_or("");
                let rest: Vec<&str> = bits.collect();
                if let Some(abs) = resolve_attr(base, u) {
                    let mut s = proxy_url(&abs);
                    if !rest.is_empty() {
                        s.push(' ');
                        s.push_str(&rest.join(" "));
                    }
                    s
                } else {
                    part.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join(", ");
        format!("{prefix}{q1}{rewritten}{q2}")
    }).into_owned();

    // Strip CSP / frame-blocking meta and X-Frame related is server header (we don't forward)
    let csp_re = Regex::new(r#"(?is)<meta[^>]+http-equiv\s*=\s*["']?content-security-policy["']?[^>]*>"#).unwrap();
    out = csp_re.replace_all(&out, "").into_owned();

    // Inject base + helper so top-level navigations still work somewhat
    let inject = format!(
        r#"<base href="{base}">
<script>
(function(){{
  document.addEventListener('click', function(e) {{
    var a = e.target.closest && e.target.closest('a[href]');
    if (!a) return;
    var href = a.getAttribute('href') || '';
    if (href.startsWith('#') || href.startsWith('javascript:') || href.startsWith('mailto:')) return;
    // ensure same-frame
    a.setAttribute('target', '_self');
  }}, true);
}})();
</script>"#,
        base = base
    );

    if let Some(idx) = out.to_lowercase().find("<head") {
        if let Some(end) = out[idx..].find('>') {
            let at = idx + end + 1;
            out.insert_str(at, &inject);
        }
    } else {
        out = inject + &out;
    }
    out
}

fn rewrite_css(css: &str, base: &Url) -> String {
    css_url_re()
        .replace_all(css, |caps: &regex::Captures| {
            let quote = &caps[1];
            let val = &caps[2];
            if let Some(abs) = resolve_attr(base, val) {
                format!("url({quote}{}{quote})", proxy_url(&abs))
            } else {
                caps[0].to_string()
            }
        })
        .into_owned()
}
