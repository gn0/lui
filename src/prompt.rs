use serde::Deserialize;

#[derive(Debug, Deserialize, Clone, PartialEq)]
pub struct Prompt {
    pub label: String,
    pub question: String,
    pub model: Option<String>,
}

impl Prompt {
    /// Converts the prompt into a Markdown representation that can be
    /// sent to the model.
    pub fn as_message(&self) -> String {
        format!("# Prompt\n\n{}", self.question)
    }
}
