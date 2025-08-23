use clap::Parser;

mod config;
mod context;
mod prompt;
mod server;

use crate::config::Config;
use crate::context::Context;

/// Command-line interface to open-webui.
#[derive(Debug, Parser)]
#[command(version, about)]
struct Args {
    /// Files to feed to open-webui's RAG API for use with the prompt.
    /// (Can be glob patterns.)
    #[arg(long, short, num_args = 1..)]
    files: Option<Vec<String>>,

    /// Files to include in the prompt sent to the model.  (Can be glob
    /// patterns, or '-' for stdin.)
    #[arg(long, short, num_args = 1..)]
    include: Option<Vec<String>>,

    #[arg(long, short)]
    model: Option<String>,

    /// Print the model's response in JSON form.
    #[arg(long, short = 'j')]
    output_json: bool,

    /// Keep the <think></think> block if the model's response has it.
    #[arg(long, short = 't')]
    keep_think_block: bool,

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

    if args.files.is_some() {
        // TODO
        panic!("RAG support is not yet implemented");
    }

    let mut response = config
        .server
        .send(&prompt.model, &prompt.render(&context))?;

    if !args.keep_think_block {
        response.remove_think_block();
    }

    if args.verbose {
        eprintln!("note: prompt tokens: {}", response.prompt_tokens);
        eprintln!("note: total time: {}", response.approximate_total);
    }

    if args.output_json {
        let response_json = serde_json::to_string(&response)
            .map_err(|x| format!("{x}"))?;

        println!("{response_json}");
    } else {
        println!("{}", response.message.trim_end());
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
