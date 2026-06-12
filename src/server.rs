use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use std::borrow::Cow;
use std::io::{BufRead, BufReader, Read};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};
use ureq::BodyReader;

use crate::context::Context;
use crate::prompt::Prompt;

/// Access details for open-webui.
#[derive(Debug, Deserialize)]
pub struct Server {
    pub host: String,
    pub port: u16,

    #[serde(rename = "api-key")]
    pub api_key: String,
}

impl Server {
    /// Send a prompt and a context to open-webui.
    ///
    /// Returns an `OutputReader::TokenIter` if `stream` is true and an
    /// `OutputReader::OutputIter` otherwise.
    ///
    /// # Errors
    ///
    /// This method returns an error if
    ///
    /// - the HTTP request to the server fails or
    /// - the server's response is
    ///
    ///   * not valid JSON,
    ///   * doesn't contain a message field,
    ///   * contains a non-integer prompt token count, or
    ///   * contains a message or an approximate duration that is not
    ///     valid UTF-8.
    pub fn send(
        &self,
        prompt: &Prompt,
        context: &Context,
        file_ids: &[String],
        stream: bool,
    ) -> Result<OutputReader<BodyReader<'static>>, String> {
        let uri = self.url("/api/chat/completions");

        let request = Request {
            model: prompt
                .model
                .as_deref()
                .ok_or_else(|| "no model specified".to_string())?
                .to_string(),
            messages: assemble_messages(context, prompt),
            stream,
            files: file_ids
                .iter()
                .map(|id| FileRef {
                    kind: "file".to_string(),
                    id: id.clone(),
                })
                .collect(),
        };

        let response = ureq::post(&uri)
            .header("Authorization", &self.bearer())
            .send_json(&request)
            .map_err(|x| format!("{x}"))?;

        if stream {
            let body_reader = response.into_body().into_reader();

            Ok(OutputReader::Streamed(TokenIter {
                reader: BufReader::new(body_reader),
                sources: Vec::new(),
            }))
        } else {
            let (output, sources) = get_complete_output(response)?;

            Ok(OutputReader::Complete(OutputIter {
                output: Some(output),
                sources,
            }))
        }
    }

    /// Builds a full request URL from a path beginning with `/`.
    fn url(&self, path: &str) -> String {
        format!("http://{}:{}{}", self.host, self.port, path)
    }

    /// Builds the `Authorization` header value.
    fn bearer(&self) -> String {
        format!("Bearer {}", self.api_key)
    }

    /// Uploads `path` to open-webui's RAG file store and returns the ID
    /// the server assigned to it.
    ///
    /// The request blocks until the file has been indexed
    /// (`process_in_background=false`), so the returned ID can be
    /// referenced in a chat request right away without polling.
    ///
    /// ureq 3.1 has no multipart support, so the `multipart/form-data`
    /// body is assembled by hand.
    ///
    /// # Errors
    ///
    /// This method returns an error if the file cannot be read, the HTTP
    /// request fails, or the response is not JSON containing a string,
    /// safe `id`.
    pub fn upload_file(&self, path: &Path) -> Result<String, String> {
        let uri =
            self.url("/api/v1/files/?process_in_background=false");

        // The filename is interpolated into a Content-Disposition
        // header, so quotes and control characters (which a Unix
        // filename may legally contain) must be removed to avoid
        // producing a malformed or injectable multipart body.
        let filename = sanitize_filename(path);
        let content_type = content_type_for(path);

        let boundary = multipart_boundary();
        let mut body = Vec::new();
        body.extend_from_slice(
            format!(
                "--{boundary}\r\n\
                 Content-Disposition: form-data; \
                 name=\"file\"; filename=\"{filename}\"\r\n\
                 Content-Type: {content_type}\r\n\r\n"
            )
            .as_bytes(),
        );
        // Append the file's bytes straight into `body` so the file is
        // held in memory only once.
        std::fs::File::open(path)
            .and_then(|mut file| file.read_to_end(&mut body))
            .map_err(|x| format!("{}: {x}", path.to_string_lossy()))?;
        body.extend_from_slice(
            format!("\r\n--{boundary}--\r\n").as_bytes(),
        );

        let response = ureq::post(&uri)
            .header("Authorization", &self.bearer())
            .header(
                "Content-Type",
                &format!("multipart/form-data; boundary={boundary}"),
            )
            .send(body)
            .map_err(|x| format!("{}: {x}", path.to_string_lossy()))?;

        let value: Value = response
            .into_body()
            .read_json()
            .map_err(|x| format!("{x}"))?;

        let id = value["id"].as_str().ok_or_else(|| {
            format!(
                "{}: upload response has no file id",
                path.to_string_lossy()
            )
        })?;

        // The ID is later used as a path component (in the journal) and
        // a URL segment (in delete_file), so reject anything that isn't
        // a plain token before trusting it.
        if !is_safe_id(id) {
            return Err(format!(
                "{}: server returned an unsafe file id {id:?}",
                path.to_string_lossy()
            ));
        }

        Ok(id.to_string())
    }

    /// Deletes the file with the given ID from open-webui.
    ///
    /// A `404` response is treated as success: the file is already
    /// gone, which is the desired end state.
    ///
    /// # Errors
    ///
    /// This method returns an error if `id` is not a safe token or if
    /// the HTTP request fails with any status other than `404`.
    pub fn delete_file(&self, id: &str) -> Result<(), String> {
        if !is_safe_id(id) {
            return Err(format!("unsafe file id {id:?}"));
        }

        let uri = self.url(&format!("/api/v1/files/{id}"));

        match ureq::delete(&uri)
            .header("Authorization", &self.bearer())
            .call()
        {
            Ok(_) => Ok(()),
            Err(ureq::Error::StatusCode(404)) => Ok(()),
            Err(x) => Err(format!("{id}: {x}")),
        }
    }

    /// Lists the IDs of every file the authenticated user can access on
    /// the server.  Entries whose ID is missing or unsafe to use as a
    /// path/URL segment are skipped with a warning rather than aborting
    /// the whole listing.
    ///
    /// # Errors
    ///
    /// This method returns an error if the HTTP request fails or the
    /// response is not a JSON array.
    pub fn list_files(&self) -> Result<Vec<String>, String> {
        let value: Value = ureq::get(&self.url("/api/v1/files/"))
            .header("Authorization", &self.bearer())
            .call()
            .map_err(|x| format!("{x}"))?
            .into_body()
            .read_json()
            .map_err(|x| format!("{x}"))?;

        let array = value
            .as_array()
            .ok_or_else(|| "malformed file list".to_string())?;

        let mut ids = Vec::new();

        for file in array {
            match file["id"].as_str() {
                Some(id) if is_safe_id(id) => ids.push(id.to_string()),
                Some(id) => {
                    log::warn!("skipping file with unsafe id {id:?}")
                }
                None => log::warn!("skipping file with no id"),
            }
        }

        Ok(ids)
    }
}

/// Returns true if `id` is safe to use both as a single file path
/// component and as a URL path segment.
///
/// The function enforces the following conditions:
///
/// - Only ASCII alphanumerics and `.`, `-`, `_` are permitted, with the
///   goal of covering the UUIDs Open WebUI ought to return while
///   preventing path separators (`/`) or URL-reserved characters (`?`,
///   `#`, `%`, etc.) from slipping through.
/// - The whole-string values `.` and `..` are rejected.
/// - The ID cannot be longer than 255 bytes, to prevent issues on file
///   systems that don't support longer filenames.
fn is_safe_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 255
        && id != "."
        && id != ".."
        && id.chars().all(|c| {
            c.is_ascii_alphanumeric()
                || c == '.'
                || c == '-'
                || c == '_'
        })
}

/// Derives the filename open-webui sees, with any character that would
/// break the Content-Disposition header (quotes, backslashes, CR/LF and
/// other control characters) removed.
fn sanitize_filename(path: &Path) -> String {
    let raw = path
        .file_name()
        .map(|x| x.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".to_string());

    let cleaned: String = raw
        .chars()
        .filter(|c| *c != '"' && *c != '\\' && !c.is_control())
        .collect();

    if cleaned.is_empty() {
        "file".to_string()
    } else {
        cleaned
    }
}

/// Guesses a MIME type from the file extension so that open-webui can
/// pick the right document loader.  Falls back to a generic binary type
/// for unknown extensions.
fn content_type_for(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|x| x.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("csv") => "text/csv",
        Some("doc") => "application/msword",
        Some("docx") => {
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
        }
        Some("epub") => "application/epub+zip",
        Some("html") | Some("htm") => "text/html",
        Some("json") => "application/json",
        Some("md") | Some("markdown") => "text/markdown",
        Some("pdf") => "application/pdf",
        Some("ppt") => "application/vnd.ms-powerpoint",
        Some("pptx") => {
            "application/vnd.openxmlformats-officedocument.presentationml.presentation"
        }
        Some("rst") => "text/x-rst",
        Some("tsv") => "text/tab-separated-values",
        Some("txt") | Some("text") | Some("log") => "text/plain",
        Some("xml") => "application/xml",
        Some("xls") => "application/vnd.ms-excel",
        Some("xlsx") => {
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
        }
        _ => "application/octet-stream",
    }
}

/// Builds a multipart boundary token unlikely to collide with file
/// contents.  Instead of pulling in a random number generator, use the
/// PID + current time in nanoseconds (unique enough in practice).
fn multipart_boundary() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);

    format!("----luiBoundary{}{}", std::process::id(), nanos)
}

/// Reads the complete output from open-webui for a non-streamed
/// request.
///
/// # Errors
///
/// This function returns an error if the server's response is
///
/// - not valid JSON,
/// - doesn't contain a message field,
/// - contains a non-integer prompt token count, or
/// - contains a message or an approximate duration that is not valid
///   UTF-8.
fn get_complete_output(
    response: http::response::Response<ureq::Body>,
) -> Result<(Output, Vec<Value>), String> {
    let value: Value = response
        .into_body()
        .read_json()
        .map_err(|x| format!("{x}"))?;

    let output = Output {
        message: value["choices"][0]["message"]["content"]
            .as_str()
            .ok_or_else(|| "malformed response".to_string())?
            .to_string(),
        prompt_tokens: Some(
            value["usage"]["prompt_tokens"].as_u64().ok_or_else(
                || "usage.prompt_tokens is not integer".to_string(),
            )?,
        ),
        approximate_total: Some(
            value["usage"]["approximate_total"]
                .as_str()
                .ok_or_else(|| "malformed response".to_string())?
                .to_string(),
        ),
    };

    let sources =
        value["sources"].as_array().cloned().unwrap_or_default();

    Ok((output, sources))
}

/// Builds the message list sent to the model: one `user` message per
/// text context piece, followed by the prompt's system/user messages.
///
/// If the context carries images, they are attached as `image_url`
/// parts to the prompt's user message (the last message), turning its
/// content from a plain string into a parts array.
fn assemble_messages(
    context: &Context,
    prompt: &Prompt,
) -> Vec<Message> {
    let mut messages: Vec<Message> = context
        .as_markdown()
        .into_iter()
        .enumerate()
        .inspect(|(index, content)| {
            log::debug!("sending context {}: {content:?}", index + 1)
        })
        .map(|(_, content)| Message {
            role: "user".to_string(),
            content: MessageContent::Text(content),
        })
        .collect();

    messages.extend(prompt.as_messages());

    if !context.images.is_empty()
        && let Some(last) = messages.last_mut()
    {
        let text = match &last.content {
            MessageContent::Text(s) => s.clone(),
            MessageContent::Parts(_) => String::new(),
        };

        let mut parts = vec![ContentPart::Text { text }];
        for (_label, url) in &context.images {
            parts.push(ContentPart::ImageUrl {
                image_url: ImageUrl { url: url.clone() },
            });
        }

        last.content = MessageContent::Parts(parts);
    }

    messages
}

#[derive(Debug, Serialize)]
struct Request {
    model: String,
    messages: Vec<Message>,
    stream: bool,

    /// RAG file references.  Skipped entirely when empty so that
    /// non-RAG requests serialize exactly as they did before.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    files: Vec<FileRef>,
}

/// A reference to a file that open-webui has already ingested, sent in
/// the chat request so the server retrieves from it.
#[derive(Debug, Serialize)]
struct FileRef {
    #[serde(rename = "type")]
    kind: String,
    id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Message {
    pub role: String,
    pub content: MessageContent,
}

/// A message body: either a plain string (the common case) or an array
/// of typed parts (used when images accompany the text).
///
/// `#[serde(untagged)]` makes the `Text` variant serialize as a bare
/// JSON string, so text-only requests are byte-for-byte identical to
/// before images were supported.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum MessageContent {
    Text(String),
    Parts(Vec<ContentPart>),
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type")]
pub enum ContentPart {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "image_url")]
    ImageUrl { image_url: ImageUrl },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ImageUrl {
    pub url: String,
}

impl<'de> Deserialize<'de> for Message {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;

        parse_message(&raw).map_err(serde::de::Error::custom)
    }
}

/// Parses a `role:content` conversation-history argument into a
/// [`Message`].
///
/// The role must be `user` or `assistant`.  Leading spaces after the
/// colon are trimmed.  The content becomes a plain-text message body.
pub fn parse_message(raw: &str) -> Result<Message, String> {
    let colon_pos = raw
        .find(':')
        .filter(|&pos| ["user", "assistant"].contains(&&raw[..pos]))
        .ok_or_else(|| {
            "history is not of form 'user:...' or 'assistant:...'"
                .to_string()
        })?;

    let content_start_pos = (colon_pos + 1 < raw.len())
        .then(|| {
            raw[(colon_pos + 1)..]
                .find(|c| c != ' ')
                .map(|pos| pos + colon_pos + 1)
        })
        .flatten()
        .ok_or_else(|| "history contains empty content".to_string())?;

    let role = raw[..colon_pos].to_string();
    let content = raw[content_start_pos..].to_string();

    Ok(Message {
        role,
        content: MessageContent::Text(content),
    })
}

pub enum OutputReader<T>
where
    T: std::io::Read,
{
    Complete(OutputIter),
    Streamed(TokenIter<T>),
}

impl<T> OutputReader<T>
where
    T: std::io::Read,
{
    /// The `sources` (citation metadata) Open WebUI returned with the
    /// response, if any.
    ///
    /// Empty when the server sent none.  For a RAG request, this is the
    /// silent-failure signal that the uploaded file was not used.
    ///
    /// Only call this function if the response has already been
    /// consumed.
    pub fn sources(&self) -> &[Value] {
        match self {
            OutputReader::Complete(output_iter) => &output_iter.sources,
            OutputReader::Streamed(token_iter) => &token_iter.sources,
        }
    }
}

impl<T> Iterator for OutputReader<T>
where
    T: std::io::Read,
{
    type Item = Output;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            OutputReader::Complete(output_iter) => {
                OutputIter::next(output_iter)
            }
            OutputReader::Streamed(token_iter) => {
                TokenIter::next(token_iter)
            }
        }
    }
}

pub struct OutputIter {
    output: Option<Output>,
    sources: Vec<Value>,
}

impl OutputIter {
    #[allow(unused)]
    pub fn new(output: Output) -> Self {
        Self {
            output: Some(output),
            sources: Vec::new(),
        }
    }
}

impl Iterator for OutputIter {
    type Item = Output;

    fn next(&mut self) -> Option<Self::Item> {
        let output = self.output.clone()?;

        self.output = None;

        Some(output)
    }
}

pub struct TokenIter<T>
where
    T: std::io::Read,
{
    reader: BufReader<T>,
    sources: Vec<Value>,
}

impl<T> TokenIter<T>
where
    T: std::io::Read,
{
    #[allow(unused)]
    pub fn new(reader: BufReader<T>) -> Self {
        Self {
            reader,
            sources: Vec::new(),
        }
    }
}

impl<T: std::io::Read> Iterator for TokenIter<T> {
    type Item = Output;

    /// Iterates over tokens sent by open-webui in a streamed response.
    ///
    /// # Errors
    ///
    /// This method returns an error if
    ///
    /// - the server sends invalid JSON for any of the tokens,
    /// - the server sends a malformed line (missing the `data: `
    ///   prefix),
    /// - a prompt token count is present but not a valid integer,
    /// - an approximate duration is present but not valid UTF-8, or
    /// - the message is present but not valid UTF-8.
    fn next(&mut self) -> Option<Self::Item> {
        let mut buffer = String::new();

        while let Ok(length) = self.reader.read_line(&mut buffer) {
            if length == 0 {
                return None;
            }

            let line = buffer.trim_matches(['\r', '\n']);

            if line.is_empty() {
                continue;
            }

            let Some(json) = line.strip_prefix("data: ") else {
                log::error!("server sent bad line: {line:?}");
                return None;
            };

            if json == "[DONE]" {
                return None;
            }

            let Ok(value): Result<Value, _> =
                serde_json::from_str(json)
            else {
                log::error!("server sent bad JSON: {json:?}");
                return None;
            };

            // Open WebUI sends RAG citations in an object that carries
            // a top-level `sources` array.  Save it for the caller.
            if let Some(array) = value["sources"].as_array()
                && !array.is_empty()
            {
                self.sources = array.clone();
            }

            let content = &value["choices"][0]["delta"]["content"];

            return Some(Output {
                message: content.as_str().unwrap_or("").to_owned(),
                prompt_tokens: value["usage"]["prompt_tokens"].as_u64(),
                approximate_total: value["usage"]["approximate_total"]
                    .as_str()
                    .map(str::to_owned),
            });
        }

        None
    }
}

#[derive(Debug, Deserialize, Serialize, Clone, PartialEq)]
pub struct Output {
    pub message: String,
    pub prompt_tokens: Option<u64>,
    pub approximate_total: Option<String>,
}

/// Returns a best-effort human-readable label for one entry of
/// Open WebUI's `sources` array.
///
/// The exact structure of a source object varies by Open WebUI version,
/// so this function tries the fields most likely to hold a document
/// name and falls back to a placeholder.
pub fn source_label(source: &Value) -> String {
    for candidate in [
        &source["source"]["name"],
        &source["source"]["id"],
        &source["name"],
        &source["metadata"][0]["name"],
        &source["metadata"][0]["source"],
        &source["file"]["filename"],
    ] {
        if let Some(text) = candidate.as_str()
            && !text.is_empty()
        {
            return text.to_string();
        }
    }

    "(unknown source)".to_string()
}

/// Removes the leading `<think></think>` block from a complete
/// response.
pub fn remove_think_block(message: &str) -> Cow<'_, str> {
    if message.starts_with("<think>")
        && let Some(pos) = message.find("</think>")
    {
        let clean = message[(pos + 8)..]
            .trim_start_matches(['\r', '\n'])
            .to_string();

        Cow::Owned(clean)
    } else {
        Cow::Borrowed(message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remove_think_block_correctly_handles_utf8() {
        assert_eq!(
            remove_think_block(
                "<think>\nlorem ipsum 概括\n</think>\n\nfoo bar baz"
            ),
            "foo bar baz"
        );
    }

    #[test]
    fn request_omits_files_when_empty() {
        let request = Request {
            model: "m".to_string(),
            messages: Vec::new(),
            stream: false,
            files: Vec::new(),
        };

        let json = serde_json::to_string(&request).unwrap();

        assert!(
            !json.contains("files"),
            "empty files should not be serialized: {json}"
        );
    }

    #[test]
    fn request_serializes_files_with_type_and_id() {
        let request = Request {
            model: "m".to_string(),
            messages: Vec::new(),
            stream: false,
            files: vec![FileRef {
                kind: "file".to_string(),
                id: "abc123".to_string(),
            }],
        };

        let json = serde_json::to_string(&request).unwrap();

        assert!(
            json.contains(r#""files":[{"type":"file","id":"abc123"}]"#),
            "unexpected files serialization: {json}"
        );
    }

    #[test]
    fn is_safe_id_rejects_path_and_url_tricks() {
        // A real open-webui ID (UUID with hyphens) is accepted.
        assert!(is_safe_id("b9733e9c-0714-4425-8915-d0361bf66dfc"));
        assert!(is_safe_id("file-0a1b2c3d-uuid"));

        for bad in [
            "",
            ".",
            "..",
            "../etc/passwd",
            "a/b",
            "a\\b",
            "with space",
            "per%cent",
            "query?x=1",
            "frag#ment",
            "amp&ersand",
            "tab\tted",
            "new\nline",
            &"x".repeat(256),
        ] {
            assert!(!is_safe_id(bad), "{bad:?} should be unsafe");
        }
    }

    #[test]
    fn sanitize_filename_strips_header_breakers() {
        assert_eq!(
            sanitize_filename(Path::new("ev\"il\r\n.pdf")),
            "evil.pdf"
        );
        assert_eq!(
            sanitize_filename(Path::new("/tmp/report.pdf")),
            "report.pdf"
        );
        // A name made entirely of stripped characters falls back.
        assert_eq!(sanitize_filename(Path::new("\"\"")), "file");
    }

    #[test]
    fn content_type_for_maps_common_extensions() {
        assert_eq!(
            content_type_for(Path::new("a.pdf")),
            "application/pdf"
        );
        // Case-insensitive on the extension.
        assert_eq!(
            content_type_for(Path::new("A.PDF")),
            "application/pdf"
        );
        assert_eq!(
            content_type_for(Path::new("sheet.xlsx")),
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
        );
        assert_eq!(
            content_type_for(Path::new("deck.pptx")),
            "application/vnd.openxmlformats-officedocument.presentationml.presentation"
        );
        assert_eq!(
            content_type_for(Path::new("book.epub")),
            "application/epub+zip"
        );
        // Extensionless filename falls back to a generic binary type.
        assert_eq!(
            content_type_for(Path::new("notes")),
            "application/octet-stream"
        );
        assert_eq!(
            content_type_for(Path::new("archive.tar.gz")),
            "application/octet-stream"
        );
    }

    #[test]
    fn message_text_content_serializes_as_string() {
        let message = Message {
            role: "user".to_string(),
            content: MessageContent::Text("foo".to_string()),
        };

        assert_eq!(
            serde_json::to_string(&message).unwrap(),
            r#"{"role":"user","content":"foo"}"#
        );
    }

    #[test]
    fn message_parts_content_serializes_as_array() {
        let message = Message {
            role: "user".to_string(),
            content: MessageContent::Parts(vec![
                ContentPart::Text {
                    text: "foo".to_string(),
                },
                ContentPart::ImageUrl {
                    image_url: ImageUrl {
                        url: "data:image/png;base64,AAA".to_string(),
                    },
                },
            ]),
        };

        let json = serde_json::to_string(&message).unwrap();

        assert!(
            json.contains(
                r#""content":[{"type":"text","text":"foo"},{"type":"image_url","image_url":{"url":"data:image/png;base64,AAA"}}]"#
            ),
            "unexpected parts serialization: {json}"
        );
    }

    fn test_prompt() -> Prompt {
        Prompt {
            label: String::new(),
            history: None,
            system: None,
            question: "foo".to_string(),
            model: Some("bar".to_string()),
        }
    }

    #[test]
    fn assemble_messages_keeps_text_when_no_images() {
        let context = Context::new();

        let messages = assemble_messages(&context, &test_prompt());

        assert!(matches!(
            messages.last().unwrap().content,
            MessageContent::Text(_)
        ));
    }

    #[test]
    fn assemble_messages_attaches_images_to_last_message() {
        let mut context = Context::new();
        context.images.push((
            "pic.png".to_string(),
            "data:image/png;base64,AAA".to_string(),
        ));

        let messages = assemble_messages(&context, &test_prompt());

        match &messages.last().unwrap().content {
            MessageContent::Parts(parts) => {
                assert_eq!(parts.len(), 2);
                // The original prompt text must be preserved verbatim.
                match &parts[0] {
                    ContentPart::Text { text } => {
                        assert_eq!(text, "#Prompt\n\nfoo")
                    }
                    other => {
                        panic!("expected text part, got {other:?}")
                    }
                }
                match &parts[1] {
                    ContentPart::ImageUrl { image_url } => assert_eq!(
                        image_url.url,
                        "data:image/png;base64,AAA"
                    ),
                    other => {
                        panic!("expected image part, got {other:?}")
                    }
                }
            }
            other => panic!("expected parts, got {other:?}"),
        }
    }

    #[test]
    fn assemble_messages_handles_multiple_images() {
        let mut context = Context::new();
        context.images.push((
            "a.png".to_string(),
            "data:image/png;base64,A".to_string(),
        ));
        context.images.push((
            "b.png".to_string(),
            "data:image/png;base64,B".to_string(),
        ));

        let messages = assemble_messages(&context, &test_prompt());

        match &messages.last().unwrap().content {
            MessageContent::Parts(parts) => {
                // One text part plus one part per image.
                assert_eq!(parts.len(), 3);
                assert!(matches!(parts[0], ContentPart::Text { .. }));
                assert!(matches!(
                    parts[1],
                    ContentPart::ImageUrl { .. }
                ));
                assert!(matches!(
                    parts[2],
                    ContentPart::ImageUrl { .. }
                ));
            }
            other => panic!("expected parts, got {other:?}"),
        }
    }

    #[test]
    fn assemble_messages_attaches_image_to_user_not_system_or_context()
    {
        let mut context = Context::new();
        context.named.push(("a.txt".to_string(), "ctx".to_string()));
        context.images.push((
            "pic.png".to_string(),
            "data:image/png;base64,AAA".to_string(),
        ));

        let prompt = Prompt {
            label: String::new(),
            history: None,
            system: Some("be brief".to_string()),
            question: "q".to_string(),
            model: Some("m".to_string()),
        };

        let messages = assemble_messages(&context, &prompt);

        // [context-text user, system, prompt user]: only the last (the
        // prompt message) carries the image.  The rest stay text.
        assert_eq!(messages.len(), 3);
        assert!(matches!(messages[0].content, MessageContent::Text(_)));
        assert_eq!(messages[1].role, "system");
        assert!(matches!(messages[1].content, MessageContent::Text(_)));
        assert!(matches!(
            messages.last().unwrap().content,
            MessageContent::Parts(_)
        ));
    }

    #[test]
    fn parse_message_with_missing_role() {
        assert!(parse_message("foo bar").is_err());
    }

    #[test]
    fn parse_message_with_unrecognized_role() {
        assert!(parse_message("baz:foo bar").is_err());
    }

    #[test]
    fn parse_message_with_missing_message() {
        assert!(parse_message("user:").is_err());
        assert!(parse_message("user:   ").is_err());
        assert!(parse_message("assistant:").is_err());
        assert!(parse_message("assistant:   ").is_err());
    }

    #[test]
    fn parse_message_with_correct_formatting() {
        for role in ["user", "assistant"] {
            let expected = Ok(Message {
                role: role.to_string(),
                content: MessageContent::Text("foo bar".to_string()),
            });

            assert_eq!(
                parse_message(&dbg!(format!("{role}:foo bar"))),
                expected
            );
            assert_eq!(
                parse_message(&dbg!(format!("{role}: foo bar"))),
                expected
            );
            assert_eq!(
                parse_message(&dbg!(format!("{role}:  foo bar"))),
                expected
            );
        }
    }

    #[test]
    fn parse_message_handles_multiline_content() {
        for role in ["user", "assistant"] {
            let expected = Ok(Message {
                role: role.to_string(),
                content: MessageContent::Text("foo\n\nbar".to_string()),
            });

            assert_eq!(
                parse_message(&dbg!(format!("{role}:foo\n\nbar"))),
                expected
            );
            assert_eq!(
                parse_message(&dbg!(format!("{role}: foo\n\nbar"))),
                expected
            );
            assert_eq!(
                parse_message(&dbg!(format!("{role}:  foo\n\nbar"))),
                expected
            );
        }
    }

    #[test]
    fn parse_message_splits_on_first_colon_only() {
        // A colon inside the content is preserved; only the first colon
        // (separating the role) splits.
        assert_eq!(
            parse_message("user:see: this"),
            Ok(Message {
                role: "user".to_string(),
                content: MessageContent::Text("see: this".to_string()),
            })
        );

        // A role that merely starts with a valid role is rejected.
        assert!(parse_message("users:hi").is_err());
    }

    #[test]
    fn message_deserializes_from_role_content_string() {
        // The custom Deserialize is what config's `default-history` and
        // a `[[prompt]]` history use, so exercise it (not just
        // parse_message directly).
        let message: Message =
            serde_json::from_str(r#""assistant:hello there""#).unwrap();
        assert_eq!(
            message,
            Message {
                role: "assistant".to_string(),
                content: MessageContent::Text(
                    "hello there".to_string()
                ),
            }
        );

        // A malformed entry surfaces as a deserialization error.
        assert!(serde_json::from_str::<Message>(r#""nope""#).is_err());
    }

    #[test]
    fn source_label_extracts_a_name_or_falls_back() {
        use serde_json::json;

        assert_eq!(
            source_label(&json!({"source": {"name": "notes.txt"}})),
            "notes.txt"
        );
        assert_eq!(
            source_label(&json!({"metadata": [{"name": "doc.pdf"}]})),
            "doc.pdf"
        );
        assert_eq!(source_label(&json!({"name": "x"})), "x");
        assert_eq!(
            source_label(&json!({"unrecognized": true})),
            "(unknown source)"
        );
    }

    #[test]
    fn token_iter_captures_sources_from_stream() {
        // A trailing chunk carrying a top-level `sources` array should
        // be stashed (not emitted as content) and readable afterwards.
        let stream = concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"hi\"}}]}\n",
            "data: {\"sources\":[{\"source\":{\"name\":\"notes.txt\"}}]}\n",
            "data: [DONE]\n",
        );

        let mut iter =
            TokenIter::new(BufReader::new(stream.as_bytes()));
        while iter.next().is_some() {}

        assert_eq!(iter.sources.len(), 1);
        assert_eq!(source_label(&iter.sources[0]), "notes.txt");
    }
}
