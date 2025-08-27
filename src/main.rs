use clap::ArgAction;
use clap::Parser;
use std::io::Write;

mod config;
mod context;
mod logger;
mod prompt;
mod server;

use crate::config::Config;
use crate::context::Context;
use crate::server::remove_think_block;

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

fn process() -> Result<(), String> {
    let args = Args::parse();

    let max_level = match args.verbose {
        0 => log::Level::Error,
        1 => log::Level::Info,
        2 => log::Level::Debug,
        3 => log::Level::Trace,
        _ => {
            return Err(
                "too many occurrences of --verbose/-v".to_string()
            )
        }
    };

    logger::init(max_level).map_err(|x| x.to_string())?;

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

    let mut prev_message: Option<String> = None;
    let mut prev_printed: Option<String> = None;
    let mut inside_think_block = false;

    for output in response {
        let mut output = output;
        let skip_this_output;

        if args.keep_think_block {
            skip_this_output = false;
        } else if !args.keep_think_block && args.no_stream {
            // Remove <think></think> block from complete output.  Also
            // normalize trailing newlines.
            //

            skip_this_output = false;

            let clean = remove_think_block(&output.message);
            output.message = format!("{}\n", clean.trim_end());
        } else {
            // Remove <think></think> block from token stream.
            //
            if prev_message.is_none() && output.message == "<think>" {
                // First token is <think>.
                //
                skip_this_output = true;
                inside_think_block = true;
            } else if inside_think_block {
                // First token was <think>.
                //

                skip_this_output = true;

                if output.message == "</think>" {
                    // Current token closes <think></think> block.
                    //
                    inside_think_block = false;
                }
            } else {
                // Normalize leading newlines.
                //
                if prev_printed.is_none() {
                    output.message =
                        output.message.trim_start().to_string();
                }

                skip_this_output = output.message.is_empty();
            }
        }

        if !skip_this_output {
            if args.output_json {
                let output_json = serde_json::to_string(&output)
                    .map_err(|x| x.to_string())?;

                println!("{output_json}");
            } else {
                print!("{}", output.message);

                let _ = std::io::stdout().flush();

                if output.message.is_empty()
                    && let Some(x) = prev_message
                    && !x.ends_with('\n')
                {
                    println!();
                }
            }

            prev_printed = Some(output.message.clone());
        }

        prev_message = Some(output.message);

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
    match process() {
        Ok(_) => std::process::exit(0),
        Err(x) => {
            eprintln!("error: {x}");
            std::process::exit(1);
        }
    }
}
