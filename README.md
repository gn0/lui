<h1 align="center">
  <img src="https://raw.githubusercontent.com/gn0/lui/main/logo/logo_300x127.png" alt="Lui's logo, a beaver with four logs of wood" />
</h1>

Lui is an LLM UI for the command line, using the API of [Open WebUI](https://github.com/open-webui/open-webui).

Compiling lui requires Rust 1.88.0 or newer because it uses [let chains](https://blog.rust-lang.org/2025/06/26/Rust-1.88.0/#let-chains).

## Contents

1. [Features](#features)
2. [Caveat](#caveat)
3. [Installation](#installation)
4. [Usage examples](#usage-examples)
   - [Basic configuration](#basic-configuration)
   - [Fill in docstring gaps](#fill-in-docstring-gaps)
   - [Review staged changes](#review-staged-changes)
   - [Ask ad hoc questions](#ask-ad-hoc-questions)
   - [Ask about an image](#ask-about-an-image)
   - [Ask about a document](#ask-about-a-document)
   - [Few-shot prompting with history](#few-shot-prompting-with-history)
5. [Detailed usage](#detailed-usage)
   - [No context](#no-context)
   - [Anonymous context](#anonymous-context)
   - [Multiple named files as context](#multiple-named-files-as-context)
   - [Glob pattern to define context](#glob-pattern-to-define-context)
   - [Pre-specified prompt](#pre-specified-prompt)
   - [Default prompt, etc.](#default-prompt-etc)
   - [Choosing the right context window](#choosing-the-right-context-window)
6. [License](#license)

## Features

+ [X] Configuration in `$XDG_CONFIG_HOME/lui/config.toml`.
  - [X] Host, port, and API key for Open WebUI.
  - [X] Prompts specified along with models.
+ [X] Pose question without context.
+ [X] Pose question with context.
  - [X] Text as context.
  - [X] PDFs and Word documents as context.
  - [X] Image files as context.
+ [X] Set a system prompt.
+ [X] Stream the tokens from Open WebUI.
+ [X] Remove `<think></think>` blocks from the response by default.
+ [ ] Automatically check if the context exceeds the maximum prompt token count.
+ [ ] List available models by querying Open WebUI.
+ [ ] List available prompts.

## Caveat

Models have a limited number of prompt tokens.
If a file that you include in the context is too large, then the model will silently ignore it even though lui does send it in the request.

One way to assess whether this is happening is by calling lui with the `-v` or (`--verbose`) command-line option, in which case it will print the prompt token count returned by Open WebUI to stderr.

(Also see [Choosing the right context window](#choosing-the-right-context-window) below.)

## Installation

If you have Cargo, then run:

```sh
cargo install --locked --git https://github.com/gn0/lui.git
```

If `$HOME/.cargo/bin` is not in your `PATH` environment variable, then you also need to run:

```sh
export PATH=$HOME/.cargo/bin:$PATH
```

To make this setting permanent:

```sh
echo 'export PATH=$HOME/.cargo/bin:$PATH' >> $HOME/.bashrc  # If using bash.
echo 'export PATH=$HOME/.cargo/bin:$PATH' >> $HOME/.zshrc   # If using zsh.
```

## Usage examples

### Basic configuration

Lui needs access to an Open WebUI API endpoint, and you probably also want to set a default model.
Add the following to `$HOME/.config/lui/config.toml`:

```toml
# Assuming Open WebUI has gemma3:27b available:
default-model = "gemma3:27b"

[server]
# Assuming Open WebUI is listening at 127.0.0.1:3000:
host = "127.0.0.1"
port = 3000
api-key = "..."
```

You can get an API key from Open WebUI by

1. clicking on your name in the bottom-left corner and navigating to "Settings,"
2. clicking on "Admin Panel" in the bottom-left corner of the Settings window,
3. making sure that "Enable API Key" is turned on,
4. clicking on your name again and navigating to "Settings," and
5. clicking on "Show" next to "API keys" on the "Account" tab of the Settings window.

### Fill in docstring gaps

It is good practice for docstrings to list the error conditions for functions that return `Result`:

```rust
/// Formats a doodad as a widget.
///
/// # Errors
///
/// This function returns an error if
///
/// TODO
pub fn as_widget(doodad: &Doodad) -> Result<Widget> {
    // ...
}
```

You can ask a model to fill in the gaps marked by `TODO`:

```sh
lui -i 'src/*.rs' -- \
    "Some of the docstrings have TODO where the error conditions \
     should be described. Can you fill in the missing error conditions \
     based on the code base?"
```

### Review staged changes

This is [Bill Mill's prompt](https://notes.billmill.org/blog/2025/07/An_AI_tool_I_find_useful.html) for rudimentary code review:

```toml
# Add this to $HOME/.config/lui/config.toml

[[prompt]]
label = "pr"
question = "Please review this PR as if you were a senior engineer."
model = "qwen3:32b"
```

As Bill also advises, take the result with more than a grain of salt.
Most of the response may be useless, some of it may be useful.

```sh
git diff --staged -U10 | lui @pr
```

You can also call it with `git review` by adding this to your `~/.gitconfig`:

```
[alias]
    review = "!sh -c 'if [ $(git diff --staged $* | wc -l) -eq 0 ]; then echo No staged changes to review.; else git diff --staged -U10 $* | lui @pr -v; fi' --"
```

If the diff exceeds the maximum prompt token count (see [Caveat](#Caveat)), then you can shrink the diff context from 10 lines to, say, 5 lines, by running `git review -U5`.

### Ask ad hoc questions

This is [kqr's system prompt](https://entropicthoughts.com/q) for asking quick questions on the command line:

```toml
default-system = """\
    Answer in as few words as possible. \
    Use a brief style with short replies.\
    """
```

```sh
lui 'how do i confine a docker container with apparmor?'
```

Response:

> Use `--security-opt apparmor=<profile-name>` when running the container. Create a custom profile in `/etc/apparmor.d/` or use Docker's default. Ensure AppArmor is enabled.

If you want the model to ignore the default system prompt, you can run lui with `-s ''`.

```sh
lui -s '' 'how do i confine a docker container with apparmor?'
```

Of course, the system prompt works with contexts, too:

```sh
lui -i 'src/*.hs' '*.cabal' \
    -m phi4-reasoning:14b \
    "Under RelNewlyAdded, the fields of the new entry are ignored. \
     Instead, this program only shows ChangeNothing in the output. I \
     want the program to instead list all the fields of the new entry, \
     e.g., ChangePatAdded (if applicable for a particular entry). \
     Where do I need to modify the source code to make this change?"
```

Response:

> The fix is in Diff.hs—specifically, the changeAll function’s pattern matching. Currently it only handles RelSameAs, RelSplitInto, and RelMergedAs (leaving RelNewlyAdded with an empty changes list). To have newly added entries report their fields, update changeAll so that it calls changeEntries for RelNewlyAdded (and similarly for RelRemoved) instead of returning []. For example, change its last clause to something like:
>
>      f x@(RelNewlyAdded b) = (x, changeEntries [] [b])
>      f x@(RelRemoved a)    = (x, changeEntries [a] [])
>      f _                    = (x, [])
>
> This way, the new entry’s fields will be processed and reported as changes.

That's not perfect Haskell code, but still a useful starting point if you haven't touched the code base in a while.

### Ask about an image

`-i` recognizes image files (PNG, JPEG, GIF, WebP) by their content and sends them to vision-capable models.
(Other binary formats, like PDFs, are rejected with a hint to use `-r` instead.  See [Ask about a document](#ask-about-a-document) below.)

```sh
lui -m gemma3:27b \
    -i diagram.png -- \
    'What does this architecture diagram show? List the components.'
```

Images and text files can be mixed in one call.
Each file passed through `-i` is routed by type, with images sent as vision input and text inlined into the prompt:

```sh
lui -m gemma3:27b \
    -i mockup.png notes.txt -- \
    'Does the mockup match the requirements in my notes?'
```

Image input requires a multimodal model (e.g., `gemma3` or `llama3.2-vision`).
A text-only model will ignore the image.

### Ask about a document

For PDF, Word, etc. documents, and for files that are too large to paste into the prompt, use `-r`/`--rag` instead of `-i`.
Lui uploads the files to Open WebUI's retrieval store and references them in the request, so the model retrieves only the relevant passages instead of reading the whole file:

```sh
lui -r handbook.pdf -- \
    'What is the policy on carrying over unused vacation days?'
```

Lui deletes the uploaded files from the server as soon as the query finishes.
If a run is interrupted (for example, Ctrl-C is pressed mid-answer), the upload is left on the server but recorded locally.
Lui reminds you on a later run, and you can delete the leftovers with:

```sh
lui --prune
```

`--prune` only removes files that lui itself uploaded and failed to clean up.
To delete every file your account can access on the server, you can run `lui --prune-all --yes`.

### Few-shot prompting with history

`-H`/`--history` seeds the conversation with prior turns, written as `user:...` and `assistant:...`.
This is useful for few-shot prompting: state the task in the system prompt (`-s`), give the model a couple of worked examples with `-H`, then ask it to handle a new case.
Lui orders the request conventionally (system prompt, then the examples, then your question), so the model sees the instruction before the demonstrations.

Here the examples teach a terse and imperative style for a one-line summary to write for a function, which is easier to show than to describe:

```sh
lui -s 'Summarize what each function does in one imperative line.' \
    -H 'user:fn f(s: &str) -> String { s.to_lowercase().replace(" ", "-") }' \
       'assistant:Convert a string to a lowercase, hyphenated slug.' \
       'user:def f(n): return n > 1 and all(n % i for i in range(2, n))' \
       'assistant:Check whether a number is prime.' \
    -- 'function f(x, lo, hi) { return Math.max(lo, Math.min(hi, x)); }'
```

Response:

> Clamp x to the range [lo, hi].

The `--` ends the `-H` list so that the final argument is taken as the question rather than as another history entry.

## Detailed usage

### No context

```sh
lui 'Why did the chicken cross the road?'
```

### Anonymous context

You can send an anonymous context to lui via a pipe:

```sh
cat make.log \
    | lui 'This build fails. How can I fix `foobar_baz`?'
```

### Multiple named files as context

```sh
lui -i foo.c bar.c baz.c make.log -- \
    'This build fails (see make.log). How can I fix `foobar_baz`?'
```

Named files can also be combined with an anonymous context:

```sh
cat make.log \
    | lui -i foo.c bar.c baz.c -- \
        'This build fails. How can I fix `foobar_baz`?'
```

You can also paste an anonymous context on stdin by using `-` as a pattern:

```sh
lui -i - -- 'This build fails. How can I fix `foobar_baz`?'
```

The `--` is necessary because `-i` accepts an arbitrary number of patterns.
Without the `--`, lui would would interpret the question as a glob pattern.
You can avoid having to use it by specifying the prompt first:

```sh
lui 'This build fails. How can I fix `foobar_baz`?' -i -
```

### Glob pattern to define context

```sh
lui -i 'src/**/*.[ch]' make.log -- \
    'This build fails (see make.log). How can I fix `foobar_baz`?'
```

### Pre-specified prompt

You can save prompts that you use often by adding them to `$HOME/.config/lui/config.toml`:

```toml
[[prompt]]
label = "build"
question = "Why does this build fail?"
model = "gemma3:27b"
```

Reference a pre-specified prompt by prepending `@` to its label:

```sh
lui -i 'src/**/*.c' make.log -- @build
```

### Default prompt, etc.

You can set a default prompt by label.
This prompt will be used when you run lui without specifying a question.

A default model and a default system prompt can also be set.
These will be applied to every prompt that doesn't explicitly set a model or a system prompt.
For example:

```toml
default-prompt = "tldr"
default-model = "gemma3:27b"
default-system = "Answer only the prompt and nothing else. Be brief."

[[prompt]]
label = "tldr"
question = "What is the tl;dr for the contents of the context?"
#
# Implied:
#
#   model = "gemma3:27b"
#   system = "Answer only the prompt and nothing else. Be brief."
#
```

```sh
lynx -dump -nolist \
    https://alexkondov.com/i-know-when-youre-vibe-coding/ \
    | lui
```

Response:

> Don't prioritize speed over code quality and maintainability, even when using LLMs. Care about consistency and long-term effects, not just a working solution.

### Choosing the right context window

Each model is limited by a maximum number of tokens that it can process at once.
This is called the context window.
Even though models support relatively large context windows, larger windows consume more GPU memory.
So unless you run small models or send prompts with small contexts, you will probably benefit from calibrating the context window for each model to match your available VRAM.

The context window is set by Open WebUI when querying Ollama, via the parameter called `num_ctx`.
You can modify the value that it sets by going to `Settings` > `Models`, and clicking on the edit button for the model that you want to calibrate.
Click on "Show" near "Advanced Params," and scroll down to `num_ctx`.

The value for this parameter should be chosen such that the model stays fully in VRAM.
Verify this by monitoring CPU and GPU usage (with tools like [`htop`](https://github.com/htop-dev/htop) and [`nvtop`](https://github.com/Syllo/nvtop)) while the model is processing your prompt.
If you choose a context window size larger than what your VRAM can handle, Ollama will fall back on CPU processing, resulting in GPU underutilization.
Your goal is to find the largest context window that still allows the model to run entirely in VRAM, enabling full GPU utilization.

## License

Lui is distributed under the GNU General Public License (GPL), version 3.
See the file [LICENSE](./LICENSE) for more information.

