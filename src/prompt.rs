use serde::Deserialize;

use crate::server::Message;

#[derive(Debug, Deserialize, Clone, PartialEq)]
pub struct Prompt {
    pub label: String,
    pub system: Option<String>,
    pub question: String,
    pub model: Option<String>,
}

impl Prompt {
    /// Converts the prompt into messages that [`Server::send`] can send
    /// to the model.
    ///
    /// If a system prompt is present in `self`, the corresponding
    /// message role is set to `system`.  The user prompt has role
    /// `user`.
    pub fn as_messages(&self) -> Vec<Message> {
        let mut result = Vec::new();

        if let Some(ref x) = self.system {
            result.push(Message {
                role: "system".to_string(),
                content: x.to_string(),
            });
        }

        result.push(Message {
            role: "user".to_string(),
            content: format!("#Prompt\n\n{}", self.question),
        });

        result
    }
}
