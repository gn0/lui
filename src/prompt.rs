use serde::Deserialize;

#[derive(Debug, Deserialize, Clone, PartialEq)]
pub struct Prompt {
    pub label: String,
    pub question: String,
    pub model: String,
}

impl Prompt {
    /// Formats the prompt as Markdown for the model.
    pub fn as_message(&self) -> String {
        format!("# Prompt\n\n{}", self.question)
    }
}
