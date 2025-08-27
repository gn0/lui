use glob::glob;
use std::io::IsTerminal;

pub type Label = String;
pub type Content = String;

/// Holds context data for the application.
///
/// This includes both anonymous context (from stdin) and named files.
/// It is used to provide additional context for the request sent to the
/// model.
#[derive(Debug)]
pub struct Context {
    pub anonymous: Option<String>,
    pub named: Vec<(Label, Content)>,
}

impl Context {
    /// Creates an empty context.
    pub fn new() -> Self {
        Self {
            anonymous: None,
            named: Vec::new(),
        }
    }

    /// Loads anonymous context from stdin.
    ///
    /// # Errors
    ///
    /// This method returns an error if
    ///
    /// - reading from stdin fails or
    /// - the input is not valid UTF-8.
    pub fn load_anonymous(&mut self) -> Result<(), String> {
        let content = std::io::read_to_string(std::io::stdin())
            .map_err(|x| format!("stdin: {x}"))?;

        self.anonymous = Some(content);

        Ok(())
    }

    /// Loads named context from files matching the given glob pattern.
    ///
    /// # Errors
    ///
    /// This method returns an error if
    ///
    /// - the glob pattern is invalid,
    /// - there was an error while traversing the filesystem to find
    ///   files that match the glob pattern, or
    /// - the content of one of the matched files is not valid UTF-8.
    pub fn load_named(&mut self, pattern: &str) -> Result<(), String> {
        for maybe_path in
            glob(pattern).map_err(|x| format!("{pattern}: {x}"))?
        {
            let path =
                maybe_path.map_err(|x| format!("{pattern}: {x}"))?;
            let content =
                std::fs::read_to_string(&path).map_err(|x| {
                    format!("{}: {x}", path.to_string_lossy())
                })?;

            self.named
                .push((String::from(path.to_string_lossy()), content));
        }

        Ok(())
    }

    /// Creates an empty context and loads each file that is matched by
    /// a pattern in `include`.
    ///
    /// # Errors
    ///
    /// This method returns an error if
    ///
    /// - any of the specified glob patterns are invalid,
    /// - there was an error while traversing the filesystem to find
    ///   files that match the glob pattern, or
    /// - either stdin or the content of one of the matched files is not
    ///   valid UTF-8.
    pub fn load(include: Option<&[String]>) -> Result<Self, String> {
        let mut context = Self::new();

        if let Some(patterns) = include {
            for pattern in patterns {
                if pattern == "-" {
                    context.load_anonymous()?;
                } else {
                    context.load_named(pattern)?;
                }
            }
        }

        if context.anonymous.is_none()
            && !std::io::stdin().is_terminal()
        {
            // The user didn't specify `--include -` but we are running
            // in non-interactive mode, so the user may be sending
            // anonymous context to us via a pipe.
            context.load_anonymous()?;
        }

        Ok(context)
    }

    /// Converts each file in the context into a Markdown representation
    /// that can be sent to the model.
    pub fn as_messages(&self) -> Vec<String> {
        let mut result = Vec::new();

        if let Some(ref content) = self.anonymous {
            result.push(format!(
                "## Unnamed input\n\n```\n{}\n```",
                content.trim_end_matches(['\r', '\n'])
            ));
        }

        for (label, content) in self.named.iter() {
            result.push(format!(
                "## File `{label}`\n\n```\n{}\n```",
                content.trim_end_matches(['\r', '\n'])
            ));
        }

        result
    }
}
