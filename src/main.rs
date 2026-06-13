use clap::{ArgAction, ArgGroup, Parser};
use std::borrow::Cow;
use std::io::Write;
use std::path::Path;

mod config;
mod context;
mod journal;
mod logger;
mod prompt;
mod server;

use crate::config::Config;
use crate::context::Context;
use crate::server::{
    Message, Output, OutputReader, Server, parse_message,
    remove_think_block,
};

/// Command-line interface to open-webui.
#[derive(Debug, Parser)]
#[command(version, about)]
#[command(
    group = ArgGroup::new("prune_mode")
        .args(["prune", "prune_all"])
        .multiple(true)
)]
struct Args {
    /// Files to feed to open-webui's RAG API for use with the prompt.
    /// (Can be glob patterns.)
    #[arg(long, short, num_args = 1..)]
    rag: Option<Vec<String>>,

    /// Files to include in the prompt sent to the model. (Can be glob
    /// patterns, or '-' for stdin.) Image files (PNG/JPEG/GIF/WebP) are
    /// detected by content and sent to vision-capable models.
    /// Documents (PDF/Word/...) should use -r/--rag instead.
    #[arg(long, short, num_args = 1..)]
    include: Option<Vec<String>>,

    /// Use this model, even if the prompt is configured with a
    /// different one.
    #[arg(long, short)]
    model: Option<String>,

    /// Specify conversation history.
    #[arg(
        long,
        short = 'H',
        num_args = 1..,
        value_parser = parse_message,
    )]
    history: Option<Vec<Message>>,

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

    /// Don't print the RAG source excerpts, only the filenames.
    #[arg(long, short = 'E')]
    hide_excerpts: bool,

    /// Don't delete files uploaded for RAG (-r) after the query.
    #[arg(long)]
    keep_uploads: bool,

    /// Delete RAG files this machine uploaded but never cleaned up
    /// (e.g., after a crash), then exit. This is a standalone
    /// maintenance operation and cannot be combined with a prompt or
    /// any prompting option.
    #[arg(
        long,
        conflicts_with_all = [
            "prune_all", "question", "rag", "include", "history",
            "model", "system", "output_json", "keep_think_block",
            "no_stream", "keep_uploads", "hide_excerpts",
        ]
    )]
    prune: bool,

    /// Delete EVERY file the user can access on the server, including
    /// persistent files and ones not uploaded by lui, then exit.
    /// Requires --yes. Like --prune, this is a standalone maintenance
    /// operation and cannot be combined with a prompt.
    #[arg(
        long,
        conflicts_with_all = [
            "question", "rag", "include", "history", "model",
            "system", "output_json", "keep_think_block", "no_stream",
            "keep_uploads", "hide_excerpts",
        ]
    )]
    prune_all: bool,

    /// Confirm the destructive --prune-all operation.
    #[arg(long, requires = "prune_all")]
    yes: bool,

    /// With --prune or --prune-all, list the files that would be
    /// deleted without deleting them.
    #[arg(long, requires = "prune_mode")]
    dry_run_prune: bool,

    /// Set log level (-v for info, -vv for debug, -vvv for trace).
    #[arg(long, short, action = ArgAction::Count)]
    verbose: u8,

    /// Either plain text or '@' + prompt label to use a prompt from the
    /// configuration. If no question is given, the default prompt will
    /// be used. May be augmented with context.
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

    /// The citation `sources` the server returned, if any.  Only
    /// meaningful once the response has been fully consumed.
    fn sources(&self) -> &[serde_json::Value] {
        self.output_reader.sources()
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

    // Prune subcommands don't need a prompt or context, and would
    // otherwise fail in resolve_prompt when no question is given.
    if args.prune {
        return prune(&config.server, args.dry_run_prune);
    }

    if args.prune_all {
        return prune_all(&config.server, args.yes, args.dry_run_prune);
    }

    warn_if_stale_uploads();

    let prompt = config.resolve_prompt(
        args.history.as_deref(),
        args.system.as_deref(),
        args.question.as_deref(),
        args.model.as_deref(),
    )?;

    let context = Context::load(args.include.as_deref())?;

    let uploads = match args.rag.as_deref() {
        Some(patterns) => upload_rag(&config.server, patterns)?,
        None => Vec::new(),
    };

    // The bare UUIDs are what the chat request, journaling, and cleanup
    // all use as keys.  The paths in `uploads` are only used to label
    // sources.
    let rag_file_ids: Vec<String> =
        uploads.iter().map(|u| u.id.clone()).collect();

    if log::log_enabled!(log::Level::Info) {
        log::info!(
            "querying model {:?}",
            prompt.model.as_deref().unwrap_or("")
        );

        if let Some(ref xs) = prompt.history {
            for message in xs {
                log::info!(
                    "using history: {} wrote {:?}",
                    message.role,
                    message.content
                );
            }
        }

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

        if !rag_file_ids.is_empty() {
            log::info!("referencing {} RAG files", rag_file_ids.len());
        }

        if !context.images.is_empty() {
            log::info!("sending {} images", context.images.len());
        }
    }

    let response = config.server.send(
        &prompt,
        &context,
        &rag_file_ids,
        !args.no_stream,
    )?;

    // Kept alive past the loop so the citation `sources` captured while
    // streaming can be read once the response is complete.
    let mut normalizer = OutputNormalizer::new(
        response,
        args.keep_think_block,
        args.no_stream,
    );

    while let Some(output) = normalizer.next() {
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

    report_sources(args, &uploads, normalizer.sources())?;

    if !args.keep_uploads {
        cleanup_uploads(&config.server, &rag_file_ids);
    }

    Ok(())
}

/// Warns if the local journal holds RAG uploads old enough to be
/// abandoned leftovers (e.g. from an interrupted run), pointing the
/// user at `--prune`.
///
/// Markers younger than the threshold are ignored so an instance of lui
/// that is running concurrently is not flagged.
///
/// Any error while checking is silently downgraded to a debug log.
fn warn_if_stale_uploads() {
    const STALE_AFTER: std::time::Duration =
        std::time::Duration::from_secs(30 * 60);

    let Some(dir) = journal::pending_dir() else {
        return;
    };

    match journal::count_older_than(&dir, STALE_AFTER) {
        Ok(0) => {}
        Ok(count) => log::warn!(
            "{count} RAG upload(s) in {} are over 30 minutes old; \
             run `lui --prune` to remove them",
            dir.to_string_lossy()
        ),
        Err(x) => log::debug!("could not check pending uploads: {x}"),
    }
}

/// Metadata for a file uploaded for a RAG request.
struct RagUpload {
    /// Server-assigned UUID.
    id: String,
    /// The path the user gave on the command line.
    name: String,
}

/// Surfaces the citation `sources` the server returned for a RAG query.
/// See the [Open WebUI sources
/// schema](crate::server#note-open-webui-sources-schema) note.
///
/// In text mode, the sources are appended to stdout as a Markdown-like
/// footer.  Each is numbered with the citation number Open WebUI gave
/// the model (so it lines up with any inline citations in the answer),
/// and its UUID is resolved back to the original filename when it
/// matches one of the `uploads`, falling back to
/// [`server::source_label`] otherwise.  Uploaded files that no source
/// cited are listed separately below.
///
/// In `--output-json` mode, they are emitted as a final JSON object so
/// stdout stays valid JSON.  Each resolvable source gains a
/// `resolved_name` field while its raw fields are preserved.
///
/// If RAG files were sent but no sources came back, warn the user
/// because the server likely ignored them.
fn report_sources(
    args: &Args,
    uploads: &[RagUpload],
    sources: &[serde_json::Value],
) -> Result<(), String> {
    // The (id, display name) pairs the resolver matches sources
    // against.
    let pairs: Vec<(String, String)> = uploads
        .iter()
        .map(|u| (u.id.clone(), u.name.clone()))
        .collect();

    let numbers = server::citation_numbers(sources);
    let unused = unused_uploads(uploads, sources, &numbers);

    if !sources.is_empty() {
        if args.output_json {
            let with_resolved: Vec<serde_json::Value> = sources
                .iter()
                .enumerate()
                .map(|(index, source)| {
                    let mut value = source.clone();
                    if let Some(object) = value.as_object_mut() {
                        if let Some(name) =
                            server::resolve_source_label(source, &pairs)
                        {
                            object.insert(
                                "resolved_name".to_string(),
                                serde_json::Value::String(name),
                            );
                        }
                        object.insert(
                            "citation".to_string(),
                            serde_json::json!(numbers[index]),
                        );
                        if !args.hide_excerpts {
                            object.insert(
                                "excerpts".to_string(),
                                excerpts_json(source),
                            );
                        }
                    }
                    value
                })
                .collect();

            let json = serde_json::to_string(&serde_json::json!({
                "sources": with_resolved,
                "unused": unused,
            }))
            .map_err(|x| x.to_string())?;

            println!("{json}");
        } else {
            print!("\n\n---\n");

            for (index, source) in sources.iter().enumerate() {
                // Skip if the source contributed no chunk: the model
                // was never shown it, so it has no citation number.
                let Some(number) = numbers[index] else {
                    continue;
                };

                let label =
                    server::resolve_source_label(source, &pairs)
                        .unwrap_or_else(|| {
                            server::source_label(source)
                        });

                print!("\n{number}. `{label}`");

                if !args.hide_excerpts {
                    print_excerpts(source);
                }
            }

            if !unused.is_empty() {
                let names: Vec<String> =
                    unused.iter().map(|n| format!("`{n}`")).collect();
                print!(
                    "\n\n(uploaded but not retrieved: {})",
                    names.join(", ")
                );
            }

            println!();

            let _ = std::io::stdout().flush();
        }
    } else if !uploads.is_empty() {
        log::warn!(
            "the server returned no sources; the uploaded RAG files \
             may not have been used, or Open WebUI's JSON schema may \
             have drifted"
        );
    }

    Ok(())
}

/// Returns the display names of uploads that no source cited.
///
/// Matches by UUID (`source.source.id`, see the [Open WebUI sources
/// schema](crate::server#note-open-webui-sources-schema) note in
/// server.rs), never by filename, so same-basename uploads with
/// distinct UUIDs are distinguished.
fn unused_uploads<'a>(
    uploads: &'a [RagUpload],
    sources: &[serde_json::Value],
    numbers: &[Option<usize>],
) -> Vec<&'a str> {
    let retrieved: Vec<&str> = sources
        .iter()
        .zip(numbers)
        .filter(|(_, number)| {
            // `numbers[i]` is `Some` exactly when source `i`
            // contributed a chunk, i.e., it was actually retrieved.
            number.is_some()
        })
        .filter_map(|(source, _)| source["source"]["id"].as_str())
        .collect();

    uploads
        .iter()
        .filter(|u| !retrieved.contains(&u.id.as_str()))
        .map(|u| u.name.as_str())
        .collect()
}

/// The longest excerpt printed in the text footer before truncation.
const EXCERPT_MAX_CHARS: usize = 200;

/// Prints one source's retrieved passages as a Markdown bullet list
/// under its citation, each prefixed with its page when known:
///
/// ```text
///    - p. 12: "Unused vacation days carry over up to a maximum of..."
/// ```
fn print_excerpts(source: &serde_json::Value) {
    for excerpt in server::source_excerpts(source) {
        // Truncate to keep the terminal footer compact.
        let text = format_excerpt_text(&excerpt.text);

        if text.is_empty() {
            continue;
        }

        match excerpt.page {
            Some(page) => print!("\n   - p. {page}: \"{text}\""),
            None => print!("\n   - \"{text}\""),
        }
    }
}

/// Normalizes the excerpt's internal whitespace and truncates the
/// excerpt to [`EXCERPT_MAX_CHARS`] characters (not bytes, to avoid
/// splitting multi-byte text mid-character).
fn format_excerpt_text(text: &str) -> String {
    let collapsed =
        text.split_whitespace().collect::<Vec<_>>().join(" ");

    let mut chars = collapsed.chars();
    let truncated: String =
        chars.by_ref().take(EXCERPT_MAX_CHARS).collect();

    if chars.next().is_some() {
        format!("{truncated}...")
    } else {
        truncated
    }
}

/// Builds the JSON `excerpts` array for one source: the full,
/// untruncated passages with their page labels.
fn excerpts_json(source: &serde_json::Value) -> serde_json::Value {
    let excerpts: Vec<serde_json::Value> =
        server::source_excerpts(source)
            .into_iter()
            .map(|excerpt| {
                serde_json::json!({
                    "page": excerpt.page,
                    "text": excerpt.text,
                })
            })
            .collect();

    serde_json::Value::Array(excerpts)
}

/// Uploads each file matched by the RAG glob patterns and records its
/// ID in the local journal *before* the chat request is sent, so that a
/// crash still leaves a prunable record.
///
/// # Errors
///
/// This function returns an error if a pattern matches no files or an
/// upload fails.  A failure to record an upload in the journal is only
/// logged: the upload itself succeeded, so the query proceeds.
fn upload_rag(
    server: &Server,
    patterns: &[String],
) -> Result<Vec<RagUpload>, String> {
    let paths = context::expand_rag_paths(patterns)?;
    let dir = journal::pending_dir();

    if dir.is_none() {
        log::warn!(
            "home directory cannot be determined; \
             uploads will not be journaled for --prune"
        );
    }

    log::info!("uploading {} RAG files", paths.len());

    let mut uploads = Vec::new();

    for path in &paths {
        log::debug!("uploading RAG file {path:?}");

        let id = server.upload_file(path)?;

        if let Some(ref dir) = dir
            && let Err(x) = journal::add(dir, &id)
        {
            log::warn!("could not record upload {id}: {x}");
        }

        uploads.push(RagUpload {
            id,
            name: path.to_string_lossy().into_owned(),
        });
    }

    Ok(uploads)
}

/// Deletes each ID from the server and, on success, drops its journal
/// marker.  Returns the number deleted.  A delete failure is reported to
/// stderr (so it is visible regardless of `-v`) but never fatal: the ID
/// stays in the journal for a later `--prune`.
fn delete_and_unjournal(
    server: &Server,
    dir: Option<&Path>,
    ids: &[String],
) -> usize {
    let mut deleted = 0;

    for id in ids {
        match server.delete_file(id) {
            Ok(()) => {
                if let Some(dir) = dir {
                    let _ = journal::remove(dir, id);
                }
                deleted += 1;
            }
            Err(x) => {
                log::warn!("could not delete file {id}: {x}")
            }
        }
    }

    deleted
}

/// Deletes the files uploaded for this query and clears their journal
/// records.
fn cleanup_uploads(server: &Server, ids: &[String]) {
    let dir = journal::pending_dir();

    delete_and_unjournal(server, dir.as_deref(), ids);
}

/// Deletes RAG files this machine uploaded but never cleaned up, using
/// the local journal as the source of truth (so it never touches a file
/// lui didn't create).
///
/// # Errors
///
/// This function returns an error if the home directory or the journal
/// cannot be read.  Individual delete failures are only warned about.
fn prune(server: &Server, dry_run: bool) -> Result<(), String> {
    let dir = journal::pending_dir().ok_or_else(|| {
        "home directory cannot be determined".to_string()
    })?;

    let ids = journal::load(&dir)?;

    if dry_run {
        for id in &ids {
            println!("{id}");
        }
        log::info!("{} files would be pruned", ids.len());
        return Ok(());
    }

    let deleted = delete_and_unjournal(server, Some(&dir), &ids);

    log::info!("pruned {deleted} files");

    Ok(())
}

/// Deletes every file the user can access on the server.  Destructive
/// and irreversible, so it refuses to run without `--yes` unless this is
/// a dry run.
///
/// # Errors
///
/// This function returns an error if `--yes` was not given, or if
/// listing the files fails.  Individual delete failures are only warned
/// about.
fn prune_all(
    server: &Server,
    yes: bool,
    dry_run: bool,
) -> Result<(), String> {
    if !dry_run {
        // Check confirmation before any network call.
        prune_all_confirmed(yes)?;
    }

    let ids = server.list_files()?;

    if dry_run {
        for id in &ids {
            println!("{id}");
        }
        log::info!("{} files would be deleted", ids.len());
        return Ok(());
    }

    let dir = journal::pending_dir();
    let deleted = delete_and_unjournal(server, dir.as_deref(), &ids);

    log::info!("deleted {deleted} of {} files", ids.len());

    Ok(())
}

/// Gate for the destructive `--prune-all` operation.
///
/// # Errors
///
/// Returns an error unless `yes` is true.
fn prune_all_confirmed(yes: bool) -> Result<(), String> {
    if yes {
        Ok(())
    } else {
        Err("--prune-all deletes every file you can access on the \
             server, including persistent files and ones not uploaded \
             by lui. Re-run with --yes to confirm, or --dry-run-prune \
             to preview."
            .to_string())
    }
}

fn main() {
    let args = Args::parse();

    let max_level = match args.verbose {
        0 => log::Level::Warn,
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

    #[test]
    fn format_excerpt_text_normalizes_and_truncates() {
        assert_eq!(
            format_excerpt_text("a\n  b\tc"),
            "a b c".to_string()
        );

        // A passage longer than the cap is truncated to the cap with an
        // ellipsis appended.
        let long = "x".repeat(EXCERPT_MAX_CHARS + 1);
        let formatted = format_excerpt_text(&long);
        assert_eq!(
            formatted,
            format!("{}...", "x".repeat(EXCERPT_MAX_CHARS))
        );

        // A passage exactly at the cap is not given an ellipsis.
        let exact = "y".repeat(EXCERPT_MAX_CHARS);
        assert_eq!(format_excerpt_text(&exact), exact);
    }

    #[test]
    fn hide_excerpts_flag_parses() {
        use clap::Parser;

        let args = Args::try_parse_from(["lui", "-E", "foo"]).unwrap();
        assert!(args.hide_excerpts);

        let args = Args::try_parse_from(["lui", "foo"]).unwrap();
        assert!(!args.hide_excerpts);
    }

    #[test]
    fn unused_uploads_finds_uncited_files_by_uuid() {
        use serde_json::json;

        let upload = |id: &str, name: &str| RagUpload {
            id: id.to_string(),
            name: name.to_string(),
        };

        // Two uploads with the same basename `report.pdf` but distinct
        // UUIDs.  Only `id-b` was retrieved.  Matching by UUID (not name)
        // must report exactly `dir_a/report.pdf` as unused, by its full
        // path.
        let uploads = vec![
            upload("id-a", "dir_a/report.pdf"),
            upload("id-b", "dir_b/report.pdf"),
        ];
        let sources = vec![json!({
            "source": {"id": "id-b", "type": "file"},
            "document": ["text"],
            "metadata": [{"source": "report.pdf"}],
        })];
        let numbers = vec![Some(1)];

        assert_eq!(
            unused_uploads(&uploads, &sources, &numbers),
            vec!["dir_a/report.pdf"]
        );
    }

    #[test]
    fn unused_uploads_counts_empty_document_sources_as_unused() {
        use serde_json::json;

        // The file came back as a source but with no chunks (number is
        // None), so it was uploaded but not actually retrieved.
        let uploads = vec![RagUpload {
            id: "id-a".to_string(),
            name: "notes.txt".to_string(),
        }];
        let sources = vec![json!({
            "source": {"id": "id-a"},
            "document": [],
            "metadata": [],
        })];

        assert_eq!(
            unused_uploads(&uploads, &sources, &[None]),
            vec!["notes.txt"]
        );
    }

    #[test]
    fn prune_all_requires_confirmation() {
        // Without --yes the guard fails, and it does so before any
        // network call (it takes no server argument).
        assert!(prune_all_confirmed(false).is_err());
        assert!(prune_all_confirmed(true).is_ok());
    }

    #[test]
    fn prune_is_a_standalone_operation() {
        use clap::Parser;

        let ok = |a: &[&str]| Args::try_parse_from(a).is_ok();
        let err = |a: &[&str]| Args::try_parse_from(a).is_err();

        // Pruning is allowed on its own (plus --yes/-v).
        assert!(ok(&["lui", "--prune"]));
        assert!(ok(&["lui", "--prune", "-v"]));
        assert!(ok(&["lui", "--prune-all", "--yes"]));

        // Pruning cannot be mixed with a prompt or prompting options.
        assert!(err(&["lui", "--prune", "hello"]));
        assert!(err(&["lui", "--prune", "-r", "x.pdf"]));
        assert!(err(&["lui", "--prune", "-i", "x.txt"]));
        assert!(err(&["lui", "--prune", "-m", "gemma"]));
        assert!(err(&["lui", "--prune", "--keep-uploads"]));
        assert!(err(&["lui", "--prune-all", "--yes", "hello"]));

        // The two prune modes are mutually exclusive, and --yes is
        // only allowed with --prune-all.
        assert!(err(&["lui", "--prune", "--prune-all"]));
        assert!(err(&["lui", "--yes"]));

        // --dry-run-prune is only allowed with a prune mode.
        assert!(err(&["lui", "--dry-run-prune"]));
        assert!(ok(&["lui", "--prune", "--dry-run-prune"]));
        assert!(ok(&["lui", "--prune-all", "--dry-run-prune"]));

        // A normal prompt with prompting options is still fine.
        assert!(ok(&["lui", "hello", "-r", "x.pdf", "-m", "gemma"]));
    }

    #[test]
    fn history_arg_parses_multiple_and_is_standalone() {
        use clap::Parser;

        // Multiple `role:content` values parse into a list of messages.
        // (The question is given first so -H's greedy values don't
        // swallow it.)
        let args = Args::try_parse_from([
            "lui",
            "hello",
            "-H",
            "user:foo",
            "assistant:bar",
        ])
        .unwrap();
        assert_eq!(
            args.history,
            Some(vec![
                parse_message("user:foo").unwrap(),
                parse_message("assistant:bar").unwrap(),
            ])
        );

        // An ill-formed value is rejected by the value parser.
        assert!(
            Args::try_parse_from(["lui", "hello", "-H", "no-role"])
                .is_err()
        );

        // --history is a prompting option, so it conflicts with --prune.
        assert!(
            Args::try_parse_from(["lui", "--prune", "-H", "user:foo"])
                .is_err()
        );
    }

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
