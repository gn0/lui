# lui

LLMs for the command line via open-webui.

## Features

+ [X] Configuration in `$XDG_CONFIG_HOME/lui/config.toml`.
  - [X] Host, port, and API key for open-webui.
  - [X] Prompts specified along with models.
+ [X] Pose question without context.
+ [X] Pose question with context.
  - [X] Text as context.
  - [ ] PDFs and Word documents as context.
  - [ ] Image files as context.
+ [ ] Stream the tokens from open-webui.

## Usage examples

### Question without context

```sh
lui 'Why did the chicken cross the road?'
```

### Question with anonymous context

```sh
lui < make.log \
    'This build fails. How can I fix `foobar_baz`?'
```

### Question with multiple named files as context

```sh
lui -i foo.c bar.c baz.c make.log -- \
    'This build fails (see make.log). How can I fix `foobar_baz`?'
```

### Question with a directory and a named file as context

```sh
lui -i 'src/**/*.[ch]' make.log -- \
    'This build fails (see make.log). How can I fix `foobar_baz`?'
```

### Use a pre-specified prompt

```toml
[[prompt]]
label = "build"
question = "Why does this build fail?"
model = "gemma3:27b"
```

```sh
lui -i 'src/**/*.[ch]' make.log -- @build
```

### Make a pre-specified prompt the default

```toml
default-prompt = "memo"

[[prompt]]
label = "memo"
question = "What am I seeing here? Please summarize as if you were writing a memo."
model = "gemma3:27b"
```

