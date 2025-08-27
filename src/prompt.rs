use serde::Deserialize;

#[derive(Debug, Deserialize, Clone, PartialEq)]
pub struct Prompt {
    pub label: String,
    pub system: Option<String>,
    pub question: String,
    pub model: Option<String>,
}

impl Prompt {
    /// Converts the prompt into a Markdown representation that can be
    /// sent to the model.
    pub fn as_message(&self) -> String {
        let mut message = "# Prompt\n\n".to_string();

        if let Some(ref x) = self.system {
            message.push_str(x);
            message.push_str("\n\n");
        }

        message.push_str(&self.question);

        message
    }
}
