use serde::Deserialize;

use crate::context::Context;

#[derive(Debug, Deserialize, Clone, PartialEq)]
pub struct Prompt {
    pub label: String,
    pub question: String,
    pub model: String,
}

impl Prompt {
    /// Formats the prompt as Markdown for the model.
    pub fn render(&self, context: &Context) -> String {
        format!("{context}\n\n# Prompt\n\n{}", self.question)
    }
}
