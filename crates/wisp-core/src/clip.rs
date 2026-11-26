//! Clip types, kind detection and small formatting helpers.

use std::path::PathBuf;

/// What a clip holds. Url, color and code are text detected on capture so the
/// UI can show the right preview and filters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Text,
    Url,
    Color,
    Code,
    Files,
    Image,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Text => "text",
            Kind::Url => "url",
            Kind::Color => "color",
            Kind::Code => "code",
            Kind::Files => "files",
            Kind::Image => "image",
        }
    }

    pub fn parse(s: &str) -> Kind {
        match s {
            "url" => Kind::Url,
            "color" => Kind::Color,
            "code" => Kind::Code,
            "files" => Kind::Files,
            "image" => Kind::Image,
            _ => Kind::Text,
        }
    }
}

/// Content read from the clipboard, before it is stored.
#[derive(Clone, Debug, PartialEq)]
pub enum ClipContent {
    Text(String),
    /// File and folder paths from a copy in File Explorer.
    Files(Vec<PathBuf>),
    /// A PNG-encoded image.
    Image { png: Vec<u8>, width: u32, height: u32 },
}

/// One clipboard capture: the content and the app it came from.
#[derive(Clone, Debug, PartialEq)]
pub struct Captured {
    pub content: ClipContent,
    pub source_app: Option<String>,
}

/// A stored clip, as the UI lists it.
#[derive(Clone, Debug, PartialEq)]
pub struct Clip {
    pub id: i64,
    pub kind: Kind,
    /// Text for text kinds, newline-separated paths for files, empty for images.
    pub text: String,
    pub source_app: Option<String>,
    /// Unix seconds.
    pub created_at: i64,
    pub pinned: bool,
    pub size_bytes: i64,
    /// Image dimensions, for image clips.
    pub image_size: Option<(u32, u32)>,
    /// Thumbnail file name inside the images folder, for image clips.
    pub thumb: Option<String>,
}

impl Clip {
    /// One-line label for lists.
    pub fn title(&self) -> String {
        match self.kind {
            Kind::Image => match self.image_size {
                Some((w, h)) => format!("Image {w} × {h}"),
                None => "Image".into(),
            },
            Kind::Files => {
                let paths: Vec<&str> = self.text.lines().collect();
                let name = |p: &str| p.rsplit(['\\', '/']).next().unwrap_or(p).to_string();
                match paths.len() {
                    0 => "Files".into(),
                    1 => name(paths[0]),
                    n => format!("{} and {} more", name(paths[0]), n - 1),
                }
            }
            _ => {
                let first = self.text.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
                first.trim().chars().take(200).collect()
            }
        }
    }
}

/// Classify copied text.
pub fn classify(text: &str) -> Kind {
    let t = text.trim();
    if t.is_empty() {
        return Kind::Text;
    }
    let single = !t.contains('\n');
    if single && !t.contains(' ') && (t.starts_with("http://") || t.starts_with("https://") || t.starts_with("www.")) {
        return Kind::Url;
    }
    if single && is_hex_color(t) {
        return Kind::Color;
    }
    if looks_like_code(t) {
        return Kind::Code;
    }
    Kind::Text
}

fn is_hex_color(t: &str) -> bool {
    let Some(hex) = t.strip_prefix('#') else { return false };
    matches!(hex.len(), 3 | 4 | 6 | 8) && hex.chars().all(|c| c.is_ascii_hexdigit())
}

fn looks_like_code(t: &str) -> bool {
    let lines: Vec<&str> = t.lines().collect();
    let mut score = 0;
    let markers = [
        "fn ", "let ", "const ", "function ", "return ", "import ", "#include", "def ", "class ", "public ",
        "SELECT ", "INSERT ", "UPDATE ", "=> ", "==", "!=", "&&", "||", "();", "git ", "cargo ", "npm ",
    ];
    for m in markers {
        if t.contains(m) {
            score += 1;
        }
    }
    let ends = lines.iter().filter(|l| {
        let l = l.trim_end();
        l.ends_with(';') || l.ends_with('{') || l.ends_with('}')
    });
    score += ends.count().min(3);
    if lines.len() > 1 && lines.iter().filter(|l| l.starts_with("    ") || l.starts_with('\t')).count() >= 1 {
        score += 1;
    }
    score >= 2
}

/// Stable 64-bit FNV-1a hash, used to spot repeat copies across restarts.
pub fn content_hash(kind: &str, bytes: &[u8]) -> i64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in kind.as_bytes().iter().chain([0u8].iter()).chain(bytes) {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h as i64
}

/// "now", "5 min", "3 h", "Yesterday", "4 days", "12 Mar"-style labels.
pub fn relative_time(then: i64, now: i64) -> String {
    let d = (now - then).max(0);
    match d {
        0..=59 => "now".into(),
        60..=3599 => format!("{} min", d / 60),
        3600..=86_399 => format!("{} h", d / 3600),
        86_400..=172_799 => "Yesterday".into(),
        _ => format!("{} days", d / 86_400),
    }
}

pub fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_text_kinds() {
        assert_eq!(classify("https://example.com/a?b=1"), Kind::Url);
        assert_eq!(classify("#2F6BFF"), Kind::Color);
        assert_eq!(classify("#fff"), Kind::Color);
        assert_eq!(classify("#hashtag"), Kind::Text);
        assert_eq!(classify("git rebase -i HEAD~3"), Kind::Text);
        assert_eq!(classify("SELECT id FROM clips\nWHERE pinned = 1;"), Kind::Code);
        assert_eq!(classify("fn main() {\n    println!(\"hi\");\n}"), Kind::Code);
        assert_eq!(classify("Meeting moved to Thursday, same room."), Kind::Text);
        assert_eq!(classify("see https://example.com for details"), Kind::Text);
    }

    #[test]
    fn hash_is_stable_and_kind_sensitive() {
        assert_eq!(content_hash("text", b"abc"), content_hash("text", b"abc"));
        assert_ne!(content_hash("text", b"abc"), content_hash("files", b"abc"));
    }

    #[test]
    fn relative_time_labels() {
        assert_eq!(relative_time(1000, 1030), "now");
        assert_eq!(relative_time(0, 300), "5 min");
        assert_eq!(relative_time(0, 7200), "2 h");
        assert_eq!(relative_time(0, 90_000), "Yesterday");
        assert_eq!(relative_time(0, 4 * 86_400), "4 days");
    }

    fn clip(kind: Kind, text: &str) -> Clip {
        Clip {
            id: 1,
            kind,
            text: text.into(),
            source_app: None,
            created_at: 0,
            pinned: false,
            size_bytes: 0,
            image_size: None,
            thumb: None,
        }
    }

    #[test]
    fn titles() {
        assert_eq!(clip(Kind::Text, "\n  first line \nsecond").title(), "first line");
        assert_eq!(clip(Kind::Files, "C:\\Work\\Starfall.pdf").title(), "Starfall.pdf");
        assert_eq!(clip(Kind::Files, "C:\\a\\one.txt\nC:\\a\\two.txt\nC:\\a\\b").title(), "one.txt and 2 more");
        let mut img = clip(Kind::Image, "");
        img.image_size = Some((1280, 720));
        assert_eq!(img.title(), "Image 1280 × 720");
    }
}
