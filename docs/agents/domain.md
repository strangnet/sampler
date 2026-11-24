# Domain docs

Single-context repo: one `GLOSSARY.md` at the root and ADRs in `docs/adr/`.

## Before exploring, read these

- **`GLOSSARY.md`**: the project's domain terms.
- **`docs/adr/`**: the ADRs that touch the area you're about to work in.

## Use the glossary's vocabulary

When your output names a domain concept (an issue title, a spec, a test name, a status message), use the term as `GLOSSARY.md` defines it, and don't drift to the synonyms it lists under _Avoid_. A concept that isn't in the glossary is either language the project doesn't use (reconsider) or a real gap: settle it during `shape` and add it then.

## Flag ADR conflicts

If your output contradicts an existing ADR, say so explicitly rather than silently overriding it:

> _Contradicts ADR-0001 (in-process acceptance driver), but worth reopening because…_

The formats for glossary entries and ADRs, and when a decision earns an ADR, are in the `shape` skill.
