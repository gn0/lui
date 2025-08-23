use serde::Deserialize;
use std::path::PathBuf;

use crate::prompt::Prompt;
use crate::server::Server;

#[derive(Debug, Deserialize)]
pub struct Config {
    pub server: Server,

    #[serde(rename = "default-prompt")]
    pub default_prompt: Option<String>,

    #[serde(rename = "default-model")]
    pub default_model: Option<String>,

    pub prompt: Vec<Prompt>,
}

impl Config {
    /// Loads the user's configuration from the location given by
    /// [`get_config_path`].
    ///
    /// # Errors
    ///
    /// This function returns an error if:
    ///
    /// - the path to the user's configuration file cannot be
    ///   determined,
    /// - the configuration file doesn't exist, or
    /// - the configuration file contains a parse error.
    pub fn load() -> Result<Self, String> {
        let path = get_config_path().ok_or_else(|| {
            String::from("Home directory cannot be determined")
        })?;

        let config: Config = toml::from_str(
            &std::fs::read_to_string(path.clone())
                .map_err(|error| format!("{path:?}: {error}"))?,
        )
        .map_err(|error| error.message().to_string())?;

        Ok(config)
    }

    pub fn resolve_prompt(
        &self,
        question: Option<&str>,
        model: Option<&str>,
    ) -> Result<Prompt, String> {
        match question {
            None => {
                let label =
                    self.default_prompt.as_ref().ok_or_else(|| {
                        "no default prompt specified".to_string()
                    })?;

                self.find_prompt(label).ok_or_else(|| {
                    format!("default prompt '{label}' not found")
                })
            }
            Some(x) => {
                if x.starts_with('@') {
                    let label: String = x.chars().skip(1).collect();

                    self.find_prompt(&label).ok_or_else(|| {
                        format!("prompt '{label}' not found")
                    })
                } else {
                    let model = model
                        .or(self.default_model.as_deref())
                        .ok_or_else(|| {
                            "no default model specified".to_string()
                        })?;

                    Ok(Prompt {
                        label: String::new(),
                        question: x.to_string(),
                        model: model.to_string(),
                    })
                }
            }
        }
    }

    fn find_prompt(&self, label: &str) -> Option<Prompt> {
        for prompt in self.prompt.iter() {
            if prompt.label == label {
                return Some(prompt.clone());
            }
        }

        None
    }
}

/// Constructs the path to the user's configuration file
/// (`$XDG_CONFIG_HOME/lui/config.toml`).
///
/// Returns `None` if the user's home directory cannot be determined.
fn get_config_path() -> Option<PathBuf> {
    let mut path = std::env::home_dir()?;

    path.push(".config");
    path.push("lui");
    path.push("config.toml");

    Some(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prompt::Prompt;

    fn make_prompts() -> Vec<Prompt> {
        vec![
            Prompt {
                label: "foo".to_string(),
                model: "foo".to_string(),
                question: "foo bar baz".to_string(),
            },
            Prompt {
                label: "bar".to_string(),
                model: "bar".to_string(),
                question: "bar baz foo".to_string(),
            },
        ]
    }

    fn make_config() -> Config {
        Config {
            server: Server {
                host: "".to_string(),
                port: 5000,
                api_key: "".to_string(),
            },
            default_prompt: None,
            default_model: None,
            prompt: make_prompts(),
        }
    }

    #[test]
    fn resolve_prompt_falls_back_on_default_prompt() {
        let mut config = make_config();

        assert_eq!(
            config.resolve_prompt(None, None),
            Err("no default prompt specified".to_string())
        );

        config.default_prompt = Some("foo".to_string());

        assert_eq!(
            config.resolve_prompt(None, None),
            Ok(config.prompt[0].clone())
        );
    }

    #[test]
    fn resolve_prompt_TODO_NAME_THIS_TODO_model_without_question() {
        // XXX Does this case make any sense?  A model without a
        // question?  This is a total bonkers scenario and probably no
        // prompt should be returned by Config::resolve_prompt.
        // However, I am not sure because it is 2:13am and I am tired.
        let config = make_config();

        assert_eq!(
            config.resolve_prompt(None, Some("foo")),
            Ok(config.prompt[0].clone())
        );

        assert_eq!(
            config.resolve_prompt(None, Some("bar")),
            Ok(config.prompt[1].clone())
        );

        assert_eq!(
            config.resolve_prompt(None, Some("asdf")),
            Err("prompt 'asdf' not found".to_string())
        );
    }

    #[test]
    fn resolve_prompt_returns_correct_prompt_if_question_is_label() {
        let config = make_config();

        assert_eq!(
            config.resolve_prompt(Some("@foo"), None),
            Ok(config.prompt[0].clone())
        );

        assert_eq!(
            config.resolve_prompt(Some("@bar"), None),
            Ok(config.prompt[1].clone())
        );

        assert_eq!(
            config.resolve_prompt(Some("@asdf"), None),
            Err("prompt 'asdf' not found".to_string())
        );
    }

    #[test]
    fn resolve_prompt_falls_back_on_default_model_if_question_is_text()
    {
        let question = "foo bar baz";
        let mut config = make_config();

        assert_eq!(
            config.resolve_prompt(Some(question), None),
            Err("no default model specified".to_string())
        );

        config.default_model = Some("foo".to_string());

        assert_eq!(
            config.resolve_prompt(Some(question), None),
            Ok(Prompt {
                label: String::new(),
                model: "foo".to_string(),
                question: question.to_string(),
            })
        );
    }

    #[test]
    fn resolve_prompt_ignores_model_if_question_is_label() {
        let model = "lorem-ipsum";
        let config = make_config();

        assert_eq!(
            config.resolve_prompt(Some("@foo"), Some(model)),
            Ok(config.prompt[0].clone())
        );

        assert_eq!(
            config.resolve_prompt(Some("@bar"), Some(model)),
            Ok(config.prompt[1].clone())
        );

        assert_eq!(
            config.resolve_prompt(Some("@asdf"), Some(model)),
            Err("prompt 'asdf' not found".to_string())
        );
    }

    #[test]
    fn resolve_prompt_uses_the_given_model_if_question_is_text() {
        let question = "foo bar baz";
        let mut config = make_config();

        // The assertions should hold whether or not a default model is
        // specified.  They are tested in a loop to make it clear that
        // it's the exact same assertions that are made.
        loop {
            assert_eq!(
                config.resolve_prompt(Some(question), Some("foo")),
                Ok(Prompt {
                    label: String::new(),
                    model: "foo".to_string(),
                    question: question.to_string(),
                })
            );

            assert_eq!(
                config.resolve_prompt(Some(question), Some("bar")),
                Ok(Prompt {
                    label: String::new(),
                    model: "bar".to_string(),
                    question: question.to_string(),
                })
            );

            if config.default_model.is_some() {
                break;
            }

            config.default_model = Some("foo".to_string());
        }
    }
}
