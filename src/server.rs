use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::borrow::Cow;
use std::io::{BufRead, BufReader};
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
        stream: bool,
    ) -> Result<OutputReader<'static>, String> {
        let uri = format!(
            "http://{}:{}/api/chat/completions",
            self.host, self.port
        );

        let mut messages: Vec<_> = context
            .as_messages()
            .into_iter()
            .map(|content| Message {
                role: "user".to_string(),
                content,
            })
            .collect();

        messages.push(Message {
            role: "user".to_string(),
            content: prompt.as_message(),
        });

        let request = Request {
            model: prompt
                .model
                .as_deref()
                .ok_or_else(|| "no model specified".to_string())?
                .to_string(),
            messages,
            stream,
        };

        let response = ureq::post(&uri)
            .header(
                "Authorization",
                &format!("Bearer {}", self.api_key),
            )
            .send_json(&request)
            .map_err(|x| format!("{x}"))?;

        if stream {
            Ok(OutputReader::Streamed(TokenIter {
                reader: BufReader::new(
                    response.into_body().into_reader(),
                ),
            }))
        } else {
            let output = get_complete_output(response)?;

            Ok(OutputReader::Complete(OutputIter {
                output: Some(output),
            }))
        }
    }
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
) -> Result<Output, String> {
    let value: Value = response
        .into_body()
        .read_json()
        .map_err(|x| format!("{x}"))?;

    Ok(Output {
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
    })
}

#[derive(Debug, Serialize)]
struct Request {
    model: String,
    messages: Vec<Message>,
    stream: bool,
}

#[derive(Debug, Serialize)]
struct Message {
    role: String,
    content: String,
}

pub enum OutputReader<'a> {
    Complete(OutputIter),
    Streamed(TokenIter<'a>),
}

impl<'a> Iterator for OutputReader<'a> {
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
}

impl Iterator for OutputIter {
    type Item = Output;

    fn next(&mut self) -> Option<Self::Item> {
        let output = self.output.clone()?;

        self.output = None;

        Some(output)
    }
}

pub struct TokenIter<'a> {
    reader: BufReader<BodyReader<'a>>,
}

impl<'a> Iterator for TokenIter<'a> {
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

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Output {
    pub message: String,
    pub prompt_tokens: Option<u64>,
    pub approximate_total: Option<String>,
}

/// Removes the leading `<think></think>` block from a complete
/// response.
pub fn remove_think_block(message: &str) -> Cow<'_, str> {
    if message.starts_with("<think>")
        && let Some(pos) = message.find("</think>")
    {
        let clean = message
            .chars()
            .skip(pos + 8)
            .collect::<String>()
            .trim_start_matches(['\r', '\n'])
            .to_string();

        Cow::Owned(clean)
    } else {
        Cow::Borrowed(message)
    }
}
