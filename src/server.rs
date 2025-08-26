use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::{BufRead, BufReader};
use ureq::BodyReader;

/// Access details for open-webui.
#[derive(Debug, Deserialize)]
pub struct Server {
    pub host: String,
    pub port: u16,

    #[serde(rename = "api-key")]
    pub api_key: String,
}

impl Server {
    pub fn send(
        &self,
        model: &str,
        message: &str,
        stream: bool,
    ) -> Result<OutputReader<'static>, String> {
        let uri = format!(
            "http://{}:{}/api/chat/completions",
            self.host, self.port
        );

        let request = Request {
            model: model.to_string(),
            messages: vec![Message {
                role: String::from("user"),
                content: message.to_string(),
            }],
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
                eprintln!("error: server sent bad line: {line:?}");
                return None;
            };

            if json == "[DONE]" {
                return None;
            }

            let Ok(value): Result<Value, _> =
                serde_json::from_str(json)
            else {
                eprintln!("error: server sent bad JSON: {json:?}");
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

pub fn remove_think_block(message: &str) -> String {
    if message.starts_with("<think>")
        && let Some(pos) = message.find("</think>")
    {
        message
            .chars()
            .skip(pos + 8)
            .collect::<String>()
            .trim_start_matches(['\r', '\n'])
            .to_string()
    } else {
        message.to_string()
    }
}
