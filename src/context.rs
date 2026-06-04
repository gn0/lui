use base64::prelude::{BASE64_STANDARD, Engine as _};
use glob::glob;
use std::io::IsTerminal;
use std::path::PathBuf;

pub type Label = String;
pub type Content = String;

/// Holds context data for the application.
///
/// This includes both anonymous context (from stdin) and named files.
/// It is used to provide additional context for the request sent to the
/// model.
#[derive(Debug)]
pub struct Context {
    pub anonymous: Option<String>,
    pub named: Vec<(Label, Content)>,

    /// Image files matched by `-i`, stored as ready-to-send `data:`
    /// URLs (`data:<mime>;base64,<...>`).  Sent to vision-capable models
    /// as `image_url` content parts rather than inlined as text.
    pub images: Vec<(Label, String)>,
}

impl Context {
    /// Creates an empty context.
    pub fn new() -> Self {
        Self {
            anonymous: None,
            named: Vec::new(),
            images: Vec::new(),
        }
    }

    /// Loads anonymous context from stdin.
    ///
    /// # Errors
    ///
    /// This method returns an error if
    ///
    /// - reading from stdin fails or
    /// - the input is not valid UTF-8.
    pub fn load_anonymous(&mut self) -> Result<(), String> {
        let content = std::io::read_to_string(std::io::stdin())
            .map_err(|x| format!("stdin: {x}"))?;

        self.anonymous = Some(content);

        Ok(())
    }

    /// Loads named context from files matching the given glob pattern.
    ///
    /// Each matched file is classified by its content (see [`sniff`]).
    /// A supported image (png/jpeg/gif/webp) is base64-encoded into an
    /// `image_url` for a vision model.  Document formats return an Err.
    /// Anything else is read as UTF-8 text.
    ///
    /// # Errors
    ///
    /// This method returns an error if
    ///
    /// - the glob pattern is invalid,
    /// - there was an error while traversing the filesystem to find
    ///   files that match the glob pattern,
    /// - a matched file is a recognized document (use `-r`/`--rag`), or
    /// - a matched file is neither a supported image nor valid UTF-8.
    pub fn load_named(&mut self, pattern: &str) -> Result<(), String> {
        glob_each(pattern, |path| {
            let label = String::from(path.to_string_lossy());
            let bytes = std::fs::read(&path)
                .map_err(|x| format!("{label}: {x}"))?;

            match sniff(&bytes) {
                Sniff::Image(mime) => {
                    let data = BASE64_STANDARD.encode(&bytes);
                    self.images.push((
                        label,
                        format!("data:{mime};base64,{data}"),
                    ));
                }
                Sniff::Document(kind) => {
                    return Err(format!(
                        "{label}: looks like {kind}; send documents \
                         with -r/--rag, not -i"
                    ));
                }
                Sniff::Unknown => {
                    let content =
                        String::from_utf8(bytes).map_err(|_| {
                            format!(
                                "{label}: not valid UTF-8 and not a \
                                 supported image (png/jpeg/gif/webp); \
                                 if it's a document, use -r/--rag"
                            )
                        })?;

                    self.named.push((label, content));
                }
            }

            Ok(())
        })
    }

    /// Creates an empty context and loads each file that is matched by
    /// a pattern in `include`.
    ///
    /// # Errors
    ///
    /// This method returns an error if
    ///
    /// - any of the specified glob patterns are invalid,
    /// - there was an error while traversing the filesystem to find
    ///   files that match the glob pattern, or
    /// - either stdin or the content of one of the matched files is not
    ///   valid UTF-8.
    pub fn load(include: Option<&[String]>) -> Result<Self, String> {
        let mut context = Self::new();

        if let Some(patterns) = include {
            for pattern in patterns {
                if pattern == "-" {
                    context.load_anonymous()?;
                } else {
                    context.load_named(pattern)?;
                }
            }
        }

        if context.anonymous.is_none()
            && !std::io::stdin().is_terminal()
        {
            // The user didn't specify `--include -` but we are running
            // in non-interactive mode, so the user may be sending
            // anonymous context to us via a pipe.
            context.load_anonymous()?;
        }

        Ok(context)
    }

    /// Converts each file in the context into a Markdown representation
    /// that can be sent to the model.
    pub fn as_markdown(&self) -> Vec<String> {
        let mut result = Vec::new();

        if let Some(ref content) = self.anonymous {
            result.push(format!(
                "## Unnamed input\n\n```\n{}\n```\n",
                content.trim_end_matches(['\r', '\n'])
            ));
        }

        for (label, content) in self.named.iter() {
            result.push(format!(
                "## File `{label}`\n\n```\n{}\n```\n",
                content.trim_end_matches(['\r', '\n'])
            ));
        }

        result
    }
}

/// Expands the glob patterns in `patterns` into a flat list of file
/// paths, deduplicated while preserving first-seen order.
///
/// Unlike [`Context::load_named`], this does not read the matched files
/// as UTF-8 text.  RAG files are uploaded to open-webui as raw bytes,
/// so only their paths are needed here.  There is also no `-`/stdin
/// case: a multipart upload needs a real file on disk.  Deduplication
/// avoids uploading the same file twice when patterns overlap (e.g.
/// `-r '*.txt' a.txt`).
///
/// # Errors
///
/// This function returns an error if
///
/// - a glob pattern is invalid,
/// - there was an error while traversing the filesystem to find files
///   that match a glob pattern, or
/// - a pattern matches no files.
pub fn expand_rag_paths(
    patterns: &[String],
) -> Result<Vec<PathBuf>, String> {
    let mut paths = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for pattern in patterns {
        glob_each(pattern, |path| {
            if seen.insert(path.clone()) {
                paths.push(path);
            }
            Ok(())
        })?;
    }

    Ok(paths)
}

/// Expands the glob `pattern` and invokes `f` once per matched path.
///
/// # Errors
///
/// This function returns an error if the pattern is invalid, the
/// filesystem traversal fails, `f` fails, or the pattern matches no
/// files.
fn glob_each(
    pattern: &str,
    mut f: impl FnMut(PathBuf) -> Result<(), String>,
) -> Result<(), String> {
    let mut matched_file = false;

    for maybe_path in
        glob(pattern).map_err(|x| format!("{pattern}: {x}"))?
    {
        let path = maybe_path.map_err(|x| format!("{pattern}: {x}"))?;

        matched_file = true;

        f(path)?;
    }

    if !matched_file {
        return Err(format!("{pattern}: no files matched"));
    }

    Ok(())
}

/// The classification of a file's bytes for `-i` handling.
enum Sniff {
    /// A supported image.  Carries its MIME type.
    Image(&'static str),
    /// A recognized document format.  Carries a human-readable name for
    /// the "use -r/--rag" error message.
    Document(&'static str),
    /// Anything else.  Treated as (attempted) UTF-8 text.
    Unknown,
}

/// Classifies a file by its leading bytes (magic numbers).
fn sniff(bytes: &[u8]) -> Sniff {
    // Supported images.
    if bytes
        .starts_with(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A])
    {
        return Sniff::Image("image/png");
    }
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Sniff::Image("image/jpeg");
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return Sniff::Image("image/gif");
    }
    if bytes.len() >= 12
        && &bytes[0..4] == b"RIFF"
        && &bytes[8..12] == b"WEBP"
    {
        return Sniff::Image("image/webp");
    }

    // Recognized document formats.  Return human-readable format name.
    if bytes.starts_with(b"%PDF-") {
        return Sniff::Document("a PDF");
    }
    if bytes.starts_with(b"PK\x03\x04")
        || bytes.starts_with(b"PK\x05\x06")
        || bytes.starts_with(b"PK\x07\x08")
    {
        return Sniff::Document(
            "a Zip/Office document (docx, xlsx, pptx, epub)",
        );
    }
    if bytes
        .starts_with(&[0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1])
    {
        return Sniff::Document("a legacy Office document");
    }
    if bytes.starts_with(b"{\\rtf") {
        return Sniff::Document("an RTF document");
    }

    Sniff::Unknown
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expand_rag_paths_matches_glob() {
        let paths =
            expand_rag_paths(&["src/*.rs".to_string()]).unwrap();

        assert!(
            paths.iter().any(|p| p.ends_with("context.rs")),
            "expected context.rs among {paths:?}"
        );
    }

    #[test]
    fn expand_rag_paths_errors_when_no_files_match() {
        let result =
            expand_rag_paths(&["src/does-not-exist-*.zzz".to_string()]);

        assert_eq!(
            result.unwrap_err(),
            "src/does-not-exist-*.zzz: no files matched"
        );
    }

    #[test]
    fn expand_rag_paths_deduplicates_overlapping_patterns() {
        // The same file is matched by both patterns; it must appear
        // only once.
        let paths = expand_rag_paths(&[
            "src/*.rs".to_string(),
            "src/context.rs".to_string(),
        ])
        .unwrap();

        let count =
            paths.iter().filter(|p| p.ends_with("context.rs")).count();

        assert_eq!(count, 1, "context.rs duplicated in {paths:?}");
    }

    #[test]
    fn sniff_classifies_by_magic_bytes() {
        assert!(matches!(
            sniff(b"\x89PNG\r\n\x1a\nrest"),
            Sniff::Image("image/png")
        ));
        assert!(matches!(
            sniff(&[0xFF, 0xD8, 0xFF, 0x00]),
            Sniff::Image("image/jpeg")
        ));
        assert!(matches!(
            sniff(b"GIF89a..."),
            Sniff::Image("image/gif")
        ));

        let mut webp = b"RIFF".to_vec();
        webp.extend_from_slice(&[0, 0, 0, 0]);
        webp.extend_from_slice(b"WEBPmore");
        assert!(matches!(sniff(&webp), Sniff::Image("image/webp")));

        // GIF87a as well as GIF89a.
        assert!(matches!(sniff(b"GIF87a;"), Sniff::Image("image/gif")));

        assert!(matches!(sniff(b"%PDF-1.7"), Sniff::Document(_)));
        assert!(matches!(sniff(b"PK\x03\x04zip"), Sniff::Document(_)));
        assert!(matches!(sniff(b"{\\rtf1"), Sniff::Document(_)));
        // Legacy OLE compound document (.doc/.xls/.ppt).
        assert!(matches!(
            sniff(&[
                0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1, 0x00
            ]),
            Sniff::Document(_)
        ));

        assert!(matches!(sniff(b"plain text"), Sniff::Unknown));
        assert!(matches!(sniff(b""), Sniff::Unknown));
        // Truncated prefixes must not panic.
        assert!(matches!(sniff(&[0xFF]), Sniff::Unknown));
        assert!(matches!(sniff(b"RIFF"), Sniff::Unknown));

        // A RIFF container that isn't WebP (e.g., WAVE audio) must not
        // be taken for an image.
        let mut wave = b"RIFF".to_vec();
        wave.extend_from_slice(&[0, 0, 0, 0]);
        wave.extend_from_slice(b"WAVEfmt ");
        assert!(matches!(sniff(&wave), Sniff::Unknown));

        // Near-misses on the document prefixes stay text, not
        // misclassified as documents.
        assert!(matches!(sniff(b"PKZIP-ish text"), Sniff::Unknown));
        assert!(matches!(sniff(b"%PDFISH not a pdf"), Sniff::Unknown));
    }

    /// A unique scratch directory for one test, removed on drop.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            use std::time::{SystemTime, UNIX_EPOCH};

            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            let path = std::env::temp_dir().join(format!(
                "lui-context-{tag}-{}-{nanos}",
                std::process::id()
            ));
            std::fs::create_dir_all(&path).unwrap();

            Self(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn load_named_routes_images_text_and_documents() {
        let dir = TempDir::new("routes");

        let png_bytes = b"\x89PNG\r\n\x1a\nIMAGEDATA".to_vec();
        let png = dir.0.join("pic.png");
        std::fs::write(&png, &png_bytes).unwrap();

        let txt = dir.0.join("a.txt");
        std::fs::write(&txt, b"hello world").unwrap();

        let pdf = dir.0.join("doc.pdf");
        std::fs::write(&pdf, b"%PDF-1.7\nstuff").unwrap();

        let mut ctx = Context::new();
        ctx.load_named(png.to_str().unwrap()).unwrap();
        ctx.load_named(txt.to_str().unwrap()).unwrap();
        let doc_err =
            ctx.load_named(pdf.to_str().unwrap()).unwrap_err();

        // Text file is in .named, image is in .images.
        assert_eq!(ctx.named.len(), 1);
        assert_eq!(ctx.images.len(), 1);

        // Markdown excludes the image.
        assert_eq!(ctx.named[0].1, "hello world");

        let url = &ctx.images[0].1;
        let b64 = url
            .strip_prefix("data:image/png;base64,")
            .expect("png data URL");
        assert_eq!(BASE64_STANDARD.decode(b64).unwrap(), png_bytes);

        assert!(
            doc_err.contains("-r/--rag"),
            "PDF error should suggest -r/--rag: {doc_err}"
        );

        assert_eq!(ctx.as_markdown().len(), 1);
    }

    #[test]
    fn load_named_rejects_non_utf8_non_image() {
        let dir = TempDir::new("binary");
        let blob = dir.0.join("blob.bin");
        // Invalid UTF-8, no recognized magic.
        std::fs::write(&blob, [0x00, 0xFF, 0xFE, 0x01]).unwrap();

        let mut ctx = Context::new();
        let err = ctx.load_named(blob.to_str().unwrap()).unwrap_err();

        assert!(
            err.contains("not a supported image"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn load_named_treats_empty_file_as_text() {
        let dir = TempDir::new("empty");
        let empty = dir.0.join("empty.txt");
        std::fs::write(&empty, b"").unwrap();

        let mut ctx = Context::new();
        ctx.load_named(empty.to_str().unwrap()).unwrap();

        assert!(ctx.images.is_empty());
        assert_eq!(ctx.named.len(), 1);
        assert_eq!(ctx.named[0].1, "");
    }

    #[test]
    fn load_named_collects_multiple_images() {
        let dir = TempDir::new("multi-image");
        for name in ["one.png", "two.png"] {
            std::fs::write(dir.0.join(name), b"\x89PNG\r\n\x1a\ndata")
                .unwrap();
        }

        let mut ctx = Context::new();
        ctx.load_named(dir.0.join("*.png").to_str().unwrap())
            .unwrap();

        assert_eq!(ctx.images.len(), 2);
        assert!(ctx.named.is_empty());
    }
}
