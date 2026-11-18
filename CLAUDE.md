# rustcreate

A native painting and animation app inspired by Procreate (plus the Photoshop features people
miss in it), written entirely in Rust: egui for the interface, wgpu for the canvas. Windows first.

**The user speaks Spanish: answer in Spanish. Docs, code and identifiers are in English.**

## Resume session

1. Read `docs/ROADMAP.md` → "Current status" says what is done and what is next.
2. Design decisions live in `docs/DESIGN.md` and `docs/adr/`. **Do not ask again about anything
   already decided**; if something must change, propose it and update the document.
3. To understand the code, query the graphify graph before reading individual files (see below).

## graphify

- The graph lives in `graphify-out/` (`graph.json`, `GRAPH_REPORT.md`, `graph.html`). Not in git.
- For code questions: `graphify query "<question>"`, `graphify explain "<node>"` or
  `graphify path "A" "B"`. Read files only for what the graph does not answer.
- **Code** (`.rs`, `Cargo.toml`): a `Stop` hook in `.claude/settings.local.json` runs
  `graphify update .` at the end of every turn. Nothing to do.
- **Documents** (`.md`): the hook does not cover them (they need semantic extraction). After
  changing any `.md`, run `/graphify . --update` before finishing the task.

## Rules

- [ADR 0006](docs/adr/0006-original-assets-only.md) is mandatory: no Procreate icons, brushes,
  textures or branding. Everything visual is original.
- `core` knows nothing about the GPU; `render` knows nothing about egui.
- Every visible string is a Fluent token with `en` and `es` translations.
- Before calling something done: `cargo test --workspace` and
  `cargo clippy --workspace --all-targets` with no warnings.
- When a task is finished: tick it in `docs/ROADMAP.md`, update "Current status" and update the
  graphify graph.
- Never run `git commit`. At the end of each finished step, suggest one commit message
  (Conventional Commits, English); the user commits.
