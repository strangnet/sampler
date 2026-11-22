## Checks

`scripts/check.sh` runs fmt, clippy (`-D warnings`) and tests; the pre-commit hook and CI run it. Fresh clone: `git config core.hooksPath .githooks`.

## Library docs

Context7 IDs and version caveats per crate: see `docs/agents/dependencies.md`. gpui gotchas (cursor, window events, paint-phase calls): see `docs/agents/gpui.md`.

## Specs

When a grilling session settles a change, file the agreed spec as a GitHub issue before implementing, and reference `#N` in commits.

## Agent skills

### Issue tracker

Issues live in GitHub Issues on `strangnet/sampler` (via the `gh` CLI). See `docs/agents/issue-tracker.md`.

### Triage labels

Default vocabulary: `needs-triage`, `needs-info`, `ready-for-agent`, `ready-for-human`, `wontfix`. See `docs/agents/triage-labels.md`.

### Domain docs

Single-context: one `GLOSSARY.md` plus `docs/adr/` at the repo root. See `docs/agents/domain.md`.
