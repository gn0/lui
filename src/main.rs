use clap::Parser;
use std::io::Write;

mod config;
mod context;
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

    /// Print the model's response in JSON form.
    #[arg(long, short = 'j')]
    output_json: bool,

    /// Keep the <think></think> block if the model's response has it.
    #[arg(long, short = 't')]
    keep_think_block: bool,

    /// Don't stream the response, only print when complete.
    #[arg(long, short = 'S')]
    no_stream: bool,

    /// Print usage data to stderr.
    #[arg(long, short)]
    verbose: bool,

    /// Either plain text or '@' + prompt label to use a prompt from the
    /// configuration.  If no question is given, the default prompt will
    /// be used.  May be augmented with context.
    question: Option<String>,
}

fn process() -> Result<(), String> {
    let args = Args::parse();

    let config = Config::load()?;

    let prompt = config.resolve_prompt(
        args.question.as_deref(),
        args.model.as_deref(),
    )?;

    let mut context = Context::new();

    if let Some(ref include) = args.include {
        for pattern in include {
            if pattern == "-" {
                context.load_anonymous()?;
            } else {
                context.load_named(pattern)?;
            }
        }
    }

    if args.rag.is_some() {
        // TODO
        panic!("RAG support is not yet implemented");
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
            output.message = remove_think_block(&output.message);
            output.message = format!("{}\n", output.message.trim_end());
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
                    .map_err(|x| format!("{x}"))?;

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

        if args.verbose {
            if let Some(x) = output.prompt_tokens {
                eprintln!("note: prompt tokens: {x}");
            }

            if let Some(x) = output.approximate_total {
                eprintln!("note: total time: {x}");
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
