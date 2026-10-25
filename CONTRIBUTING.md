# Contributing to InfiniShell

Thanks for helping improve InfiniShell! This guide explains how to open issues, propose changes, and get your work reviewed.

## Contribution workflow

- Start with an issue in this repository to describe the problem and agree on scope.
- For larger features, discuss a product and technical spec with the maintainers
  before implementation. Bug fixes can usually proceed with a focused regression test.
- Maintainers review the change, its tests and its compatibility boundaries before
  merging.
- If an issue has a readiness label, follow it; otherwise ask about scope on the issue.
  A label is not an assignment or a guarantee that a proposed implementation will merge.

## Filing a Good Issue

Search [existing issues](https://github.com/Infinimesh-ai/InfiniShell-Desktop/issues) before filing to avoid duplicates.

Use the repository issue templates. Review logs, screenshots and attachments before
posting; remove credentials, private host details and unrelated conversation content.

### Bug reports

A good bug report includes:

- A clear title and a one-paragraph summary of the problem.
- Steps to reproduce (with a minimal example where possible).
- Expected vs. actual behavior.
- InfiniShell version and OS (see `Settings → About`).
- Logs, screenshots, or screen recordings when relevant.

Link the issue in the fix PR and explain how the regression test reproduces it.

### Feature requests

A good feature request describes the user-facing problem before any proposed implementation. Include:

- The user need or pain point, and who experiences it.
- The current behavior and why it falls short.
- A sketch of the desired behavior or workflow (a short example or mock is helpful but not required).
- Any relevant constraints (compatibility, related features, prior art, etc.).

Discuss scope with a maintainer before implementing larger features. Use a spec PR
when the behavior, architecture or compatibility tradeoffs need agreement.

## Opening a Spec PR

For work that needs a spec, agree on it before writing the implementation. A spec consists of two short documents committed under [`specs/GH<issue-number>/`](specs/):

- **`product.md`** (the *product spec*) — Defines the desired behavior from the consumer's perspective (the user, an API caller, a CLI user, etc.) and stays out of implementation detail. The core is a numbered list of **testable behavior invariants** covering the happy path, user-visible states, inputs and responses, and edge cases (empty / error / loading, cancellation, offline, permission denied, races, accessibility). Optional sections: problem statement, goals / non-goals, Figma link, open questions.
- **`tech.md`** (the *tech spec*) — The implementation plan, grounded in this codebase. Required sections: **Context** (the current system and relevant files with line references), **Proposed changes** (modules touched, new types / APIs / state, data flow, tradeoffs), and **Testing and validation** (how each invariant from the product spec will be verified). Optional: end-to-end flow, Mermaid diagrams, risks, parallelization, follow-ups.

To open a spec PR:

1. Add `specs/GH<issue-number>/product.md` and `specs/GH<issue-number>/tech.md`. See [`specs/GH408/`](specs/GH408/), [`specs/GH1063/`](specs/GH1063/), and [`specs/GH1066/`](specs/GH1066/) for examples of well-structured specs, and browse the rest of [`specs/`](specs/) for more.
2. Use the PR as the home for product and technical discussion.
3. Once the specs are approved, implementation generally continues on the same PR. In rarer cases — for example, if a large spec is merged on its own so the implementation can be broken up — it can move to a linked follow-up PR.

## Opening a Code PR

For an agreed feature or a focused bug fix:

1. Branch from `main`.
2. Implement the change and add tests (see [Testing](#testing)).
3. Run `./script/presubmit` and fix any failures before pushing.
4. Open a PR using the [pull request template](.github/pull_request_template.md) and add a changelog entry (`CHANGELOG-NEW-FEATURE`, `CHANGELOG-IMPROVEMENT`, or `CHANGELOG-BUG-FIX`); omit only for docs-only or refactoring-only changes.
5. Keep the PR focused on a single logical change and merge `main` in before the PR enters review.

Address reviewer feedback on the PR and describe the validation performed after each
revision. Maintainers decide when a change is ready to merge.

## Using a Coding Agent

You can use **any coding agent** to implement a contribution — for example, InfiniShell's built-in agent, Claude Code, Codex, Gemini CLI, or others — or no agent at all. This repository ships agent-readable context (skills under [`.agents/skills/`](.agents/skills/), specs under [`specs/`](specs/), and [`AGENTS.md`](AGENTS.md)) that any harness supporting these formats can pick up.

## Development Setup

See [README.md](README.md) and [AGENTS.md](AGENTS.md) for the full engineering guide. Quick start:

```bash
./script/bootstrap   # platform-specific setup
cargo run            # build and run InfiniShell
./script/presubmit   # fmt, clippy, and tests
```

## Testing

Tests are required for most code changes:

- **Bug fixes** should include a regression test that would have caught the bug.
- **Algorithmic or non-trivial logic** needs unit tests.
- **User-facing flows** should have end-to-end coverage under [`crates/integration/`](crates/integration/) whenever the behavior can be exercised that way. The bar is high-quality coverage of the changes you ship — with agent-driven development the expectation is more integration tests, not just coverage of P0 paths. If a flow is worth shipping, it's usually worth an integration test.

Before proposing a PR or pushing a commit, run `cargo check`. Run the
workspace test suite with:

```bash
cargo nextest run --no-fail-fast --workspace --exclude command-signatures-v2
```

Every new or changed user feature also needs an English/Simplified Chinese audit,
`cargo test -p warp --lib i18n::tests`, and `cargo check -p warp`. Update both Fluent
catalogs with matching variables; check both languages when layout can change.
If no user-visible text changes, record that no localization change is needed.
See [AGENTS.md](AGENTS.md) for the full gates and platform-specific test guidance.

## Code Style

- Follow the formatting and lint commands in `./script/presubmit`; it checks the
  workspace and the default GUI configuration separately. Do not substitute an
  `--all-features` invocation for that script.
- Prefer imports over path qualifiers, inline format args (`println!("{x}")`), and exhaustive `match` over `_` wildcards.
- See [AGENTS.md](AGENTS.md) for the full style guide, including WarpUI patterns and terminal model locking rules.

## Commit and Branch Conventions

- Branch names should be prefixed with your handle (e.g. `alice/fix-parser`).
- Write new commit subjects and bodies in English. The Simplified Chinese requirement in `AGENTS.md` applies to code comments and conversational replies, not commit messages.
- Use `type(scope): description` for commit subjects; the scope is optional. Use one of `feat`, `fix`, `docs`, `refactor`, `perf`, `test`, `build`, `ci`, `chore`, or `revert` as the type. Scopes should identify stable modules or subsystems, such as `cli-agent` or `terminal`, rather than temporary acceptance IDs.
- Use a concise imperative description of the actual change, without a trailing period. Commit messages should explain *what* and *why*; when the subject cannot explain the reason, add a body separated by a blank line describing the problem, motivation, or tradeoff.
- Example: `fix(cli-agent): preserve PowerShell 5.1 script encoding`.
- These conventions apply to new commits; existing commit history does not need to be rewritten.

## Code of Conduct

This project adopts the [Contributor Covenant](https://www.contributor-covenant.org/) (v2.1) as its code of conduct. All contributors and maintainers are expected to follow it in every project space. See [`CODE_OF_CONDUCT.md`](CODE_OF_CONDUCT.md) for the full text.

## Reporting Security Issues

See [`SECURITY.md`](SECURITY.md) for our security disclosure policy and private reporting channels. **Do not open public issues for security vulnerabilities.**

## Getting Help

- Open a [GitHub issue](https://github.com/Infinimesh-ai/InfiniShell-Desktop/issues) for bugs or feature requests.
