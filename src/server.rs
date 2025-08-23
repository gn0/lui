use serde::{Deserialize, Serialize};
use serde_json::Value;

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
    ) -> Result<Output, String> {
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
        };

        let response: Value = ureq::post(&uri)
            .header(
                "Authorization",
                &format!("Bearer {}", self.api_key),
            )
            .send_json(&request)
            .map_err(|x| format!("{x}"))?
            .body_mut()
            .read_json()
            .map_err(|x| format!("{x}"))?;

        Ok(Output {
            message: response["choices"][0]["message"]["content"]
                .as_str()
                .ok_or_else(|| "malformed response".to_string())?
                .to_string(),
        })
    }
}

#[derive(Debug, Serialize)]
struct Request {
    model: String,
    messages: Vec<Message>,
}

#[derive(Debug, Serialize)]
struct Message {
    role: String,
    content: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Output {
    pub message: String,
}
