use clap::{ArgAction, Parser};
use std::borrow::Cow;
use std::io::Write;

mod config;
mod context;
mod logger;
mod prompt;
mod server;

use crate::config::Config;
use crate::context::Context;
use crate::server::{Output, OutputReader, remove_think_block};

/// Command-line interface to open-webui.
#[derive(Debug, Parser)]
#[command(version, about)]
struct Args {
    /// Files to feed to open-webui's RAG API for use with the prompt.
    /// (Can be glob patterns.)
    #[arg(long, short, num_args = 1..)]
    rag: Option<Vec<String>>,

    /// Files to include in the prompt sent to the model.  (Can be glob
    /// patterns, or '-' for stdin.)
    #[arg(long, short, num_args = 1..)]
    include: Option<Vec<String>>,

    /// Use this model, even if the prompt is configured with a
    /// different one.
    #[arg(long, short)]
    model: Option<String>,

    /// Specify system prompt.
    #[arg(long, short)]
    system: Option<String>,

    /// Print the model's response in JSON form.
    #[arg(long, short = 'j')]
    output_json: bool,

    /// Keep the <think></think> block if the model's response has it.
    #[arg(long, short = 't')]
    keep_think_block: bool,

    /// Don't stream the response, only print when complete.
    #[arg(long, short = 'S')]
    no_stream: bool,

    /// Set log level (-v for info, -vv for debug, -vvv for trace).
    #[arg(long, short, action = ArgAction::Count)]
    verbose: u8,

    /// Either plain text or '@' + prompt label to use a prompt from the
    /// configuration.  If no question is given, the default prompt will
    /// be used.  May be augmented with context.
    question: Option<String>,
}

struct OutputNormalizer<T>
where
    T: std::io::Read,
{
    output_reader: OutputReader<T>,
    ever_read: bool,
    prev_returned_output: Option<Output>,
    keep_think_block: bool,
    no_stream: bool,
    inside_think_block: bool,
}

impl<T> OutputNormalizer<T>
where
    T: std::io::Read,
{
    fn new(
        output_reader: OutputReader<T>,
        keep_think_block: bool,
        no_stream: bool,
    ) -> Self {
        Self {
            output_reader,
            ever_read: false,
            prev_returned_output: None,
            keep_think_block,
            no_stream,
            inside_think_block: false,
        }
    }
}

impl<T> Iterator for OutputNormalizer<T>
where
    T: std::io::Read,
{
    type Item = Output;

    fn next(&mut self) -> Option<Self::Item> {
        for mut output in self.output_reader.by_ref() {
            log::debug!("server sent {output:?}");

            let skip_this_output;

            if self.no_stream {
                // The output we've received should contain the server's
                // complete response.
                //

                let clean = if self.keep_think_block {
                    Cow::from(output.message)
                } else {
                    remove_think_block(&output.message)
                };

                // Normalize trailing whitespace.
                //
                output.message = format!("{}\n", clean.trim_end());

                skip_this_output = false;
            } else {
                // The output we've received is one token in the
                // server's response stream.
                //

                if !self.keep_think_block
                    && !self.ever_read
                    && output.message == "<think>"
                {
                    // We want to drop the <think></think> block and the
                    // first token (this one!) is <think>.
                    //
                    skip_this_output = true;
                    self.inside_think_block = true;
                } else if !self.keep_think_block
                    && self.inside_think_block
                {
                    // We want to drop the <think></think> block.  The
                    // first token (previously) was <think> and we
                    // haven't seen </think> before.
                    //

                    skip_this_output = true;

                    if output.message == "</think>" {
                        // Current token closes <think></think> block.
                        //
                        self.inside_think_block = false;
                    }
                } else if self.prev_returned_output.is_none() {
                    // This is the first token that would be printed.
                    // Normalize leading newlines.
                    //

                    output.message =
                        output.message.trim_start().to_string();

                    skip_this_output = output.message.is_empty()
                        && output.prompt_tokens.is_none()
                        && output.approximate_total.is_none();
                } else {
                    if output.message.is_empty() {
                        // An empty message indicates the end of the
                        // token stream.  Change the message to `\n` to
                        // make sure that we print a closing newline.
                        //
                        output.message = "\n".to_string();
                    }

                    skip_this_output = false;
                }
            }

            self.ever_read = true;

            if !skip_this_output {
                self.prev_returned_output = Some(output.clone());

                return Some(output);
            }
        }

        None
    }
}

fn process(args: &Args) -> Result<(), String> {
    let config = Config::load()?;

    let prompt = config.resolve_prompt(
        args.system.as_deref(),
        args.question.as_deref(),
        args.model.as_deref(),
    )?;

    let context = Context::load(args.include.as_deref())?;

    if args.rag.is_some() {
        // TODO
        panic!("RAG support is not yet implemented");
    }

    if log::log_enabled!(log::Level::Info) {
        log::info!(
            "querying model {:?}",
            prompt.model.as_deref().unwrap_or("")
        );

        if let Some(ref x) = prompt.system {
            log::info!("using system prompt {x:?}");
        }

        match (context.anonymous.is_some(), context.named.len()) {
            (true, 0) => log::info!("sending stdin as context"),
            (true, x) => {
                log::info!("sending stdin and {x} files as context")
            }
            (false, x) if x > 0 => {
                log::info!("sending {x} files as context")
            }
            _ => (),
        }
    }

    let response =
        config.server.send(&prompt, &context, !args.no_stream)?;

    let normalizer = OutputNormalizer::new(
        response,
        args.keep_think_block,
        args.no_stream,
    );

    for output in normalizer {
        if args.output_json {
            let output_json = serde_json::to_string(&output)
                .map_err(|x| x.to_string())?;

            println!("{output_json}");
        } else {
            print!("{}", output.message);

            let _ = std::io::stdout().flush();
        }

        if log::log_enabled!(log::Level::Info) {
            if let Some(x) = output.prompt_tokens {
                log::info!("prompt tokens: {x}");
            }

            if let Some(x) = output.approximate_total {
                log::info!("total time: {x}");
            }
        }
    }

    Ok(())
}

fn main() {
    let args = Args::parse();

    let max_level = match args.verbose {
        0 => log::Level::Error,
        1 => log::Level::Info,
        2 => log::Level::Debug,
        3 => log::Level::Trace,
        _ => {
            eprintln!("error: too many occurrences of --verbose/-v");
            std::process::exit(1);
        }
    };

    logger::init(max_level).unwrap_or_else(|x| {
        eprintln!("error: {x}");
        std::process::exit(1)
    });

    match process(&args) {
        Ok(_) => std::process::exit(0),
        Err(x) => {
            log::error!("{x}");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::TokenIter;
    use std::io::BufReader;

    fn new_streamed_output_reader(
        tokens: &[&str],
    ) -> OutputReader<&'static [u8]> {
        let mut responses: Vec<_> = tokens
            .iter()
            .map(|x| {
                format!(
                    "{}{}{}",
                    r#"data: {"choices":[{"delta":{"content":""#,
                    x.replace("\n", "\\n"),
                    r#""}}]}"#
                )
            })
            .collect();

        responses.push(
            r#"data: {"usage":{"prompt_tokens":2,"approximate_total":"foo bar"}}"#
                .to_string()
        );

        let buf_reader = BufReader::new(
            format!("{}\r\n", responses.join("\r\n")).leak().as_bytes(),
        );

        OutputReader::Streamed(TokenIter::new(buf_reader))
    }

    fn new_complete_output_reader(
        tokens: &[&str],
    ) -> OutputReader<&'static [u8]> {
        OutputReader::Complete(server::OutputIter::new(Output {
            message: tokens.join("").to_string(),
            prompt_tokens: Some(2),
            approximate_total: Some("foo bar".to_string()),
        }))
    }

    #[test]
    fn output_normalizer_keeps_complete_think_block() {
        let mut normalizer = OutputNormalizer::new(
            new_complete_output_reader(&[
                "<think>", "asdf", " qwerty", "</think>", "\n\n",
                "lorem", " ipsum",
            ]),
            true,
            true,
        );

        assert_eq!(
            normalizer.next(),
            Some(Output {
                message: "<think>asdf qwerty</think>\n\nlorem ipsum\n"
                    .to_string(),
                prompt_tokens: Some(2),
                approximate_total: Some("foo bar".to_string()),
            })
        );
    }

    #[test]
    fn output_normalizer_keeps_streamed_think_block() {
        let normalizer = OutputNormalizer::new(
            new_streamed_output_reader(&[
                "<think>", "asdf", " qwerty", "</think>", "\\n\\n",
                "lorem", " ipsum",
            ]),
            true,
            false,
        );

        assert_eq!(
            normalizer.collect::<Vec<_>>(),
            &[
                Output {
                    message: "<think>".to_string(),
                    prompt_tokens: None,
                    approximate_total: None,
                },
                Output {
                    message: "asdf".to_string(),
                    prompt_tokens: None,
                    approximate_total: None,
                },
                Output {
                    message: " qwerty".to_string(),
                    prompt_tokens: None,
                    approximate_total: None,
                },
                Output {
                    message: "</think>".to_string(),
                    prompt_tokens: None,
                    approximate_total: None,
                },
                Output {
                    message: "\n\n".to_string(),
                    prompt_tokens: None,
                    approximate_total: None,
                },
                Output {
                    message: "lorem".to_string(),
                    prompt_tokens: None,
                    approximate_total: None,
                },
                Output {
                    message: " ipsum".to_string(),
                    prompt_tokens: None,
                    approximate_total: None,
                },
                Output {
                    message: "\n".to_string(),
                    prompt_tokens: Some(2),
                    approximate_total: Some("foo bar".to_string()),
                },
            ]
        );
    }

    #[test]
    fn output_normalizer_removes_complete_think_block() {
        let mut normalizer = OutputNormalizer::new(
            new_complete_output_reader(&[
                "<think>", "asdf", " qwerty", "</think>", "\n\n",
                "lorem", " ipsum",
            ]),
            false,
            true,
        );

        assert_eq!(
            normalizer.next(),
            Some(Output {
                message: "lorem ipsum\n".to_string(),
                prompt_tokens: Some(2),
                approximate_total: Some("foo bar".to_string()),
            })
        );
    }

    #[test]
    fn output_normalizer_removes_streamed_think_block() {
        let normalizer = OutputNormalizer::new(
            new_streamed_output_reader(&[
                "<think>", "asdf", " qwerty", "</think>", "\\n\\n",
                "lorem", " ipsum",
            ]),
            false,
            false,
        );

        assert_eq!(
            normalizer.collect::<Vec<_>>(),
            &[
                Output {
                    message: "lorem".to_string(),
                    prompt_tokens: None,
                    approximate_total: None,
                },
                Output {
                    message: " ipsum".to_string(),
                    prompt_tokens: None,
                    approximate_total: None,
                },
                Output {
                    message: "\n".to_string(),
                    prompt_tokens: Some(2),
                    approximate_total: Some("foo bar".to_string()),
                },
            ]
        );
    }
}
