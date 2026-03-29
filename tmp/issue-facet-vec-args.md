# bug: `Vec<String>` named args in facet require all subsequent named args

## Problem

Any command using `Vec<String>` with `#[facet(args::named)]` makes the CLI unusable in non-interactive mode.
The facet args parser treats Vec fields as required sequences and refuses to dispatch the command until every named arg — including later unrelated ones — is provided.
Even then it rejects scalar values with:

```text
Error: unexpected token: got scalar, expected sequence start
```

This affects every `Vec<String>` field in `cli.rs`:

- `issue new --label` / `--assignee`
- `issue edit --add-label` / `--remove-label` / `--add-assignee` / `--remove-assignee`

## Reproduction

```console
$ forge issue new "test" --body "body"
Error: missing required argument
   ╭─[ <suggestion>:1:28 ]
 1 │ issue new test --body body --label <label>
   │                            ───────┬───────
   │                                   ╰─── Labels to attach.
───╯

$ forge issue new "test" --body "body" --label enhancement --label bug --assignee me
Error: unexpected token: got scalar, expected sequence start
```

Even `--interactive` alone fails because the parser insists on the Vec fields before it ever reaches the `interactive` bool:

```console
$ forge issue new --interactive
Error: missing required argument
```

## Proposed fix

Change `Vec<String>` fields to `Option<String>` with comma-separated parsing (matching how `issue list --state`, `--platform`, and `--id` already work), then split in the handler:

```rust
// cli.rs
/// Labels to attach (comma-separated).
#[facet(args::named, args::short = 'l', rename = "label")]
labels: Option<String>,

// exe.rs
let labels: Vec<String> = labels
    .as_deref()
    .map(|s| s.split(',').map(|v| v.trim().to_string()).filter(|v| !v.is_empty()).collect())
    .unwrap_or_default();
```

Apply the same pattern to `assignees` in `New` and all Vec fields in `Edit`.
