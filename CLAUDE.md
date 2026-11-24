## Checks

`scripts/check.sh` runs fmt, clippy (`-D warnings`) and tests; the pre-commit hook and CI run it. Fresh clone: `git config core.hooksPath .githooks`.

## Library docs

Context7 IDs and version caveats per crate: see `docs/agents/dependencies.md`. gpui gotchas (cursor, window events, paint-phase calls): see `docs/agents/gpui.md`.

## Way of working

Workflow skills come from the `strangnet-skills` plugin: `shape` → `spec` → `slice` (when a spec needs more than one session) → `acceptance-testing` wrapping `tdd-cycle`.

- Specs live in `docs/specs/`, ADRs in `docs/adr/`, the glossary in `GLOSSARY.md`.
- Acceptance tests: yes, layered (scenario → DSL → protocol driver), no Gherkin. In-process driver per ADR-0001.
- Tasks: GitHub issues, as sub-issues of the source issue. Reference `#N` in commits.
- Sources: check the official docs for the pinned version before writing library code (see `docs/agents/dependencies.md`), and cite them.
- Doubt: gates. Cross-model: off.

## Issue tracker

Issues live in GitHub Issues on `strangnet/sampler` (via the `gh` CLI). See `docs/agents/issue-tracker.md`.

## Domain docs

Single-context: one `GLOSSARY.md` plus `docs/adr/` at the repo root. See `docs/agents/domain.md`.
