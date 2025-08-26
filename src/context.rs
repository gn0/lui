use glob::glob;

pub type Label = String;
pub type Content = String;

/// Holds context data for the application.
///
/// This includes both anonymous context (from stdin) and named files.
/// It is used to provide additional context for the request sent to the
/// model.
#[derive(Debug)]
pub struct Context {
    anonymous: Option<String>,
    named: Vec<(Label, Content)>,
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
    /// - the glob pattern is invalid or
    /// - there was an error while traversing the filesystem to find
    ///   files that match the glob pattern.
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
