//! Image link helpers and external viewer launch for GitHub detail.
//!
//! Images are text rows in the TUI; click/`o` downloads into XDG cache and
//! opens with `CORRAL_GITHUB_IMAGE_VIEWER` (default: imv).

use crate::ui::Palette;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

pub(crate) const MAX_IMAGE_BYTES: usize = 8 * 1024 * 1024;
const IMAGE_FETCH_TIMEOUT: Duration = Duration::from_secs(15);

/// One image discovered while rendering markdown, keyed by its final line index.
/// Click / `o` opens the URL with the external viewer (imv by default).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ImagePlacement {
    pub(crate) line: usize,
    pub(crate) url: String,
    pub(crate) alt: String,
}

pub(crate) fn image_viewer() -> String {
    std::env::var("CORRAL_GITHUB_IMAGE_VIEWER")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "imv".into())
}

/// Download a remote image into the XDG cache and open it with the configured
/// external viewer (default: imv). Network I/O runs off the UI thread via
/// `DetailApp::open_image`; this helper still uses an explicit ureq timeout so
/// a hung CDN cannot stall the worker forever.
pub(crate) fn open_image_externally(url: &str, _alt: &str) -> Result<String, String> {
    let path = cache_image(url)?;
    let viewer = image_viewer();
    let mut child = Command::new(&viewer)
        .arg(&path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|error| format!("image viewer `{viewer}`: {error}"))?;
    // Reap the GUI process so repeated opens do not leave zombies.
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(format!("opened in {viewer}"))
}

pub(crate) fn cache_image(url: &str) -> Result<PathBuf, String> {
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err("unsupported image url".into());
    }
    let dir = image_cache_dir()?;
    let digest = stable_url_digest(url);
    // Prefer an already-resolved file (with or without extension).
    if let Some(existing) = find_cached_image(&dir, digest) {
        return Ok(existing);
    }
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(IMAGE_FETCH_TIMEOUT))
        .build()
        .into();
    // GitHub user-attachments redirects to S3; follow redirects with a normal
    // browser-ish UA so private CDN edges do not 403 bare clients.
    let mut response = agent
        .get(url)
        .header(
            "User-Agent",
            "corral-github/0.1 (+https://github.com/xifan2333/herdr-corral)",
        )
        .header("Accept", "image/*,*/*;q=0.8")
        .call()
        .map_err(|error| error.to_string())?;
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_string();
    let mut bytes = Vec::new();
    let mut reader = response.body_mut().as_reader().take((MAX_IMAGE_BYTES as u64) + 1);
    reader
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.is_empty() {
        return Err("empty image body".into());
    }
    if bytes.len() > MAX_IMAGE_BYTES {
        return Err(format!("image larger than {MAX_IMAGE_BYTES} bytes"));
    }
    let ext = extension_from_url(url)
        .or_else(|| extension_from_content_type(&content_type))
        .or_else(|| extension_from_magic(&bytes))
        .unwrap_or("bin");
    let path = dir.join(format!("{digest:016x}.{ext}"));
    if path.is_file() {
        return Ok(path);
    }
    let temp = dir.join(format!("{digest:016x}.part-{}", std::process::id()));
    {
        let mut file = std::fs::File::create(&temp).map_err(|error| error.to_string())?;
        file.write_all(&bytes).map_err(|error| error.to_string())?;
    }
    std::fs::rename(&temp, &path).map_err(|error| error.to_string())?;
    Ok(path)
}

pub(crate) fn image_cache_dir() -> Result<PathBuf, String> {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    let dir = base.join("corral").join("github-images");
    std::fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
    Ok(dir)
}

pub(crate) fn stable_url_digest(url: &str) -> u64 {
    // DefaultHasher is process-stable enough for a local cache key; we only
    // need collisions to be rare within one machine's cache dir.
    let mut hasher = DefaultHasher::new();
    url.hash(&mut hasher);
    hasher.finish()
}

pub(crate) fn find_cached_image(dir: &Path, digest: u64) -> Option<PathBuf> {
    let prefix = format!("{digest:016x}");
    let exact = dir.join(&prefix);
    if exact.is_file() {
        return Some(exact);
    }
    // Common extensions first so we avoid a directory scan on the hot path.
    for ext in ["png", "jpg", "jpeg", "gif", "webp", "bmp", "svg", "bin"] {
        let candidate = dir.join(format!("{prefix}.{ext}"));
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

pub(crate) fn extension_from_url(url: &str) -> Option<&'static str> {
    let path = url.split('?').next().unwrap_or(url);
    let ext = Path::new(path).extension()?.to_str()?.to_ascii_lowercase();
    match ext.as_str() {
        "png" => Some("png"),
        "jpg" | "jpeg" => Some("jpg"),
        "gif" => Some("gif"),
        "webp" => Some("webp"),
        "bmp" => Some("bmp"),
        "svg" => Some("svg"),
        _ => None,
    }
}

pub(crate) fn extension_from_content_type(content_type: &str) -> Option<&'static str> {
    let mime = content_type.split(';').next().unwrap_or("").trim().to_ascii_lowercase();
    match mime.as_str() {
        "image/png" => Some("png"),
        "image/jpeg" | "image/jpg" => Some("jpg"),
        "image/gif" => Some("gif"),
        "image/webp" => Some("webp"),
        "image/bmp" | "image/x-ms-bmp" => Some("bmp"),
        "image/svg+xml" => Some("svg"),
        _ => None,
    }
}

pub(crate) fn extension_from_magic(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n']) {
        Some("png")
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Some("jpg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("gif")
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("webp")
    } else {
        None
    }
}

/// Pull `src` / `alt` from a raw HTML `<img ...>` snippet (GitHub comments).
pub(crate) fn extract_html_img(html: &str) -> Option<(String, String)> {
    let lower = html.to_ascii_lowercase();
    let start = lower.find("<img")?;
    let rest = &html[start..];
    let end = rest.find('>').map(|index| index + 1).unwrap_or(rest.len());
    let tag = &rest[..end];
    let src = html_attr(tag, "src")?;
    if !(src.starts_with("http://") || src.starts_with("https://")) {
        return None;
    }
    let alt = html_attr(tag, "alt").unwrap_or_default();
    Some((src, alt))
}

pub(crate) fn html_attr(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let key = format!("{name}=");
    let index = lower.find(&key)?;
    let value = tag[index + key.len()..].trim_start();
    let quote = value.chars().next()?;
    if quote == '"' || quote == '\'' {
        let body = &value[1..];
        let end = body.find(quote)?;
        return Some(body[..end].to_string());
    }
    // Unquoted attribute: read until whitespace or tag end.
    let end = value
        .find(|ch: char| ch.is_whitespace() || ch == '>')
        .unwrap_or(value.len());
    Some(value[..end].to_string())
}

pub(crate) fn looks_like_image_url(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    if !(lower.starts_with("http://") || lower.starts_with("https://")) {
        return false;
    }
    // Autolink / bare-URL promotion is intentionally narrow: only hosts that
    // GitHub uses for issue/PR image attachments. Markdown images and HTML
    // <img> still accept any http(s) URL via their own event paths.
    if lower.contains("github.com/user-attachments/assets/")
        || lower.contains("user-images.githubusercontent.com/")
        || lower.contains("private-user-images.githubusercontent.com/")
    {
        return true;
    }
    extension_from_url(url).is_some()
}

/// If `segs` is only a bare image URL (optionally surrounded by whitespace),
/// consume it and return the URL.
pub(crate) fn take_lone_image_url(segs: &mut Vec<(String, Style)>) -> Option<String> {
    let text: String = segs.iter().map(|(text, _)| text.as_str()).collect();
    let trimmed = text.trim();
    if trimmed.is_empty() || trimmed.chars().any(char::is_whitespace) {
        return None;
    }
    if !looks_like_image_url(trimmed) {
        return None;
    }
    segs.clear();
    Some(trimmed.to_string())
}

pub(crate) fn push_image_link(
    out: &mut Vec<Line<'static>>,
    images: &mut Vec<(usize, String, String)>,
    url: String,
    alt: &str,
    palette: &Palette,
) {
    let label = if alt.trim().is_empty() {
        "attachment"
    } else {
        alt.trim()
    };
    images.push((out.len(), url.clone(), label.to_string()));
    out.push(Line::from(vec![
        Span::styled(
            "[image] ".to_string(),
            Style::default()
                .fg(palette.accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            label.to_string(),
            Style::default()
                .fg(palette.blue)
                .add_modifier(Modifier::UNDERLINED),
        ),
    ]));
}
