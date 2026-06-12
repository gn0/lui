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
    /// The order is the conventional one for a chat request: the
    /// `system` prompt (if any) first, then the conversation `history`
    /// (if any), then the user's question.  Putting the system prompt
    /// first matters for two reasons:
    ///
    /// 1. Many chat templates only honor a system prompt if it leads
    ///    the conversation.
    /// 2. This is the right layout for few-shot prompting (instruction,
    ///    then examples, then query).
    pub fn as_messages(&self) -> Vec<Message> {
        let mut result = Vec::new();

        if let Some(ref x) = self.system {
            result.push(Message {
                role: "system".to_string(),
                content: MessageContent::Text(x.to_string()),
            });
        }

        if let Some(ref xs) = self.history {
            result.extend_from_slice(xs);
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
        // Order must be: system prompt first, then history, then the
        // user's question.
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
                    role: "system".to_string(),
                    content: MessageContent::Text("baz".to_string())
                },
                Message {
                    role: "user".to_string(),
                    content: MessageContent::Text(
                        "lorem ipsum".to_string()
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
}
