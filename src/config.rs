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
            "Home directory cannot be determined".to_string()
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
        if let Some(x) = question
            && !x.starts_with('@')
        {
            // Question is text.

            if x.is_empty() {
                Err("prompt is empty".to_string())
            } else {
                let model = model
                    .or(self.default_model.as_deref())
                    .ok_or_else(|| {
                        "no default model specified".to_string()
                    })?;

                Ok(Prompt {
                    label: String::new(),
                    question: x.to_string(),
                    model: Some(model.to_string()),
                })
            }
        } else {
            let mut prompt = match question {
                None => {
                    // Question is missing.

                    let label = self
                        .default_prompt
                        .as_ref()
                        .ok_or_else(|| {
                            "no default prompt specified".to_string()
                        })?;

                    self.find_prompt(label).ok_or_else(|| {
                        format!("default prompt '{label}' not found")
                    })?
                }
                Some(x) => {
                    // Question starts with '@'.

                    let label: String = x.chars().skip(1).collect();

                    self.find_prompt(&label).ok_or_else(|| {
                        format!("prompt '{label}' not found")
                    })?
                }
            };

            prompt.model = model
                .map(str::to_string)
                .or_else(|| prompt.model.clone())
                .or_else(|| self.default_model.clone());

            Ok(prompt)
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
                model: Some("foo".to_string()),
                question: "foo bar baz".to_string(),
            },
            Prompt {
                label: "bar".to_string(),
                model: Some("bar".to_string()),
                question: "bar baz foo".to_string(),
            },
        ]
    }

    fn make_config_without_defaults() -> Config {
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
    fn resolve_prompt_handles_all_scenarios() {
        let err_nodefp =
            || Err("no default prompt specified".to_string());
        let err_nodefm =
            || Err("no default model specified".to_string());
        let err_emptyp = || Err("prompt is empty".to_string());
        let err_badp = || Err("prompt 'asdf' not found".to_string());
        let err_baddefp =
            || Err("default prompt 'asdf' not found".to_string());
        let ok_foo = || Ok(make_prompts().into_iter().next().unwrap());
        let ok_foo_um = || {
            Ok(Prompt {
                model: Some("um".to_string()),
                ..make_prompts().into_iter().next().unwrap()
            })
        };
        let ok_custom_m = || {
            Ok(Prompt {
                label: "".to_string(),
                model: Some("m".to_string()),
                question: "...".to_string(),
            })
        };
        let ok_custom_um = || {
            Ok(Prompt {
                label: "".to_string(),
                model: Some("um".to_string()),
                question: "...".to_string(),
            })
        };

        #[rustfmt::skip]
        let table = &[
            // Fields:
            //
            // 1. expected output
            // 2. default prompt
            // 3. default model
            // 4. user-specified question
            // 5. user-specified model
            //
            (err_nodefp(),   None, None, None, None),
            (err_nodefp(),   None, None, None, Some("um")),
            (err_emptyp(),   None, None, Some(""), None),
            (err_emptyp(),   None, None, Some(""), Some("um")),
            (ok_foo(),       None, None, Some("@foo"), None),
            (ok_foo_um(),    None, None, Some("@foo"), Some("um")),
            (err_badp(),     None, None, Some("@asdf"), None),
            (err_badp(),     None, None, Some("@asdf"), Some("um")),
            (err_nodefm(),   None, None, Some("..."), None),
            (ok_custom_um(), None, None, Some("..."), Some("um")),
            (err_nodefp(),   None, Some("m"), None, None),
            (err_nodefp(),   None, Some("m"), None, Some("um")),
            (err_emptyp(),   None, Some("m"), Some(""), None),
            (err_emptyp(),   None, Some("m"), Some(""), Some("um")),
            (ok_foo(),       None, Some("m"), Some("@foo"), None),
            (ok_foo_um(),    None, Some("m"), Some("@foo"), Some("um")),
            (err_badp(),     None, Some("m"), Some("@asdf"), None),
            (err_badp(),     None, Some("m"), Some("@asdf"), Some("um")),
            (ok_custom_m(),  None, Some("m"), Some("..."), None),
            (ok_custom_um(), None, Some("m"), Some("..."), Some("um")),
            (ok_foo(),       Some("foo"), None, None, None),
            (ok_foo_um(),    Some("foo"), None, None, Some("um")),
            (err_emptyp(),   Some("foo"), None, Some(""), None),
            (err_emptyp(),   Some("foo"), None, Some(""), Some("um")),
            (ok_foo(),       Some("foo"), None, Some("@foo"), None),
            (ok_foo_um(),    Some("foo"), None, Some("@foo"), Some("um")),
            (err_badp(),     Some("foo"), None, Some("@asdf"), None),
            (err_badp(),     Some("foo"), None, Some("@asdf"), Some("um")),
            (err_nodefm(),   Some("foo"), None, Some("..."), None),
            (ok_custom_um(), Some("foo"), None, Some("..."), Some("um")),
            (ok_foo(),       Some("foo"), Some("m"), None, None),
            (ok_foo_um(),    Some("foo"), Some("m"), None, Some("um")),
            (err_emptyp(),   Some("foo"), Some("m"), Some(""), None),
            (err_emptyp(),   Some("foo"), Some("m"), Some(""), Some("um")),
            (ok_foo(),       Some("foo"), Some("m"), Some("@foo"), None),
            (ok_foo_um(),    Some("foo"), Some("m"), Some("@foo"), Some("um")),
            (err_badp(),     Some("foo"), Some("m"), Some("@asdf"), None),
            (err_badp(),     Some("foo"), Some("m"), Some("@asdf"), Some("um")),
            (ok_custom_m(),  Some("foo"), Some("m"), Some("..."), None),
            (ok_custom_um(), Some("foo"), Some("m"), Some("..."), Some("um")),
            (err_baddefp(),  Some("asdf"), None, None, None),
            (err_baddefp(),  Some("asdf"), None, None, Some("um")),
            (err_emptyp(),   Some("asdf"), None, Some(""), None),
            (err_emptyp(),   Some("asdf"), None, Some(""), Some("um")),
            (ok_foo(),       Some("asdf"), None, Some("@foo"), None),
            (ok_foo_um(),    Some("asdf"), None, Some("@foo"), Some("um")),
            (err_badp(),     Some("asdf"), None, Some("@asdf"), None),
            (err_badp(),     Some("asdf"), None, Some("@asdf"), Some("um")),
            (err_nodefm(),   Some("asdf"), None, Some("..."), None),
            (ok_custom_um(), Some("asdf"), None, Some("..."), Some("um")),
            (err_baddefp(),  Some("asdf"), Some("m"), None, None),
            (err_baddefp(),  Some("asdf"), Some("m"), None, Some("um")),
            (err_emptyp(),   Some("asdf"), Some("m"), Some(""), None),
            (err_emptyp(),   Some("asdf"), Some("m"), Some(""), Some("um")),
            (ok_foo(),       Some("asdf"), Some("m"), Some("@foo"), None),
            (ok_foo_um(),    Some("asdf"), Some("m"), Some("@foo"), Some("um")),
            (err_badp(),     Some("asdf"), Some("m"), Some("@asdf"), None),
            (err_badp(),     Some("asdf"), Some("m"), Some("@asdf"), Some("um")),
            (ok_custom_m(),  Some("asdf"), Some("m"), Some("..."), None),
            (ok_custom_um(), Some("asdf"), Some("m"), Some("..."), Some("um")),
        ];

        for (expected, defp, defm, q, m) in table.iter() {
            dbg!((expected, defp, defm, q, m));

            let mut config = make_config_without_defaults();
            config.default_prompt = defp.map(|x| x.to_string());
            config.default_model = defm.map(|x| x.to_string());

            assert_eq!(config.resolve_prompt(*q, *m), *expected);
        }
    }
}
