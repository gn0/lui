use serde::Deserialize;

use crate::server::{Message, MessageContent};

#[derive(Debug, Deserialize, Clone, PartialEq)]
pub struct Prompt {
    pub label: String,
    pub history: Option<Vec<Message>>,
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

        if let Some(ref xs) = self.history {
            result.extend_from_slice(xs);
        }

        if let Some(ref x) = self.system {
            result.push(Message {
                role: "system".to_string(),
                content: MessageContent::Text(x.to_string()),
            });
        }

        result.push(Message {
            role: "user".to_string(),
            content: MessageContent::Text(format!(
                "#Prompt\n\n{}",
                self.question
            )),
        });

        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_as_messages_with_only_question() {
        assert_eq!(
            Prompt {
                label: String::new(),
                history: None,
                system: None,
                question: "foo bar".to_string(),
                model: None,
            }
            .as_messages(),
            vec![Message {
                role: "user".to_string(),
                content: MessageContent::Text(
                    "#Prompt\n\nfoo bar".to_string()
                )
            }]
        );
    }

    #[test]
    fn prompt_as_messages_with_question_and_system_prompt() {
        assert_eq!(
            Prompt {
                label: String::new(),
                history: None,
                system: Some("baz".to_string()),
                question: "foo bar".to_string(),
                model: None,
            }
            .as_messages(),
            vec![
                Message {
                    role: "system".to_string(),
                    content: MessageContent::Text("baz".to_string())
                },
                Message {
                    role: "user".to_string(),
                    content: MessageContent::Text(
                        "#Prompt\n\nfoo bar".to_string()
                    )
                }
            ]
        );
    }

    #[test]
    fn prompt_as_messages_with_question_and_history() {
        assert_eq!(
            Prompt {
                label: String::new(),
                history: Some(vec![
                    Message {
                        role: "user".to_string(),
                        content: MessageContent::Text(
                            "lorem ipsum".to_string()
                        )
                    },
                    Message {
                        role: "assistant".to_string(),
                        content: MessageContent::Text(
                            "dolor sit amet".to_string()
                        )
                    }
                ]),
                system: None,
                question: "foo bar".to_string(),
                model: None,
            }
            .as_messages(),
            vec![
                Message {
                    role: "user".to_string(),
                    content: MessageContent::Text(
                        "lorem ipsum".to_string()
                    )
                },
                Message {
                    role: "assistant".to_string(),
                    content: MessageContent::Text(
                        "dolor sit amet".to_string()
                    )
                },
                Message {
                    role: "user".to_string(),
                    content: MessageContent::Text(
                        "#Prompt\n\nfoo bar".to_string()
                    )
                }
            ]
        );
    }

    #[test]
    fn prompt_as_messages_with_history_and_system_prompt() {
        // Order must be: history first, then the system prompt, then
        // the user's question.
        assert_eq!(
            Prompt {
                label: String::new(),
                history: Some(vec![Message {
                    role: "user".to_string(),
                    content: MessageContent::Text(
                        "lorem ipsum".to_string()
                    )
                }]),
                system: Some("baz".to_string()),
                question: "foo bar".to_string(),
                model: None,
            }
            .as_messages(),
            vec![
                Message {
                    role: "user".to_string(),
                    content: MessageContent::Text(
                        "lorem ipsum".to_string()
                    )
                },
                Message {
                    role: "system".to_string(),
                    content: MessageContent::Text("baz".to_string())
                },
                Message {
                    role: "user".to_string(),
                    content: MessageContent::Text(
                        "#Prompt\n\nfoo bar".to_string()
                    )
                }
            ]
        );
    }
}
