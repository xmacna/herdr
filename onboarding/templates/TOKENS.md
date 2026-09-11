# Tokens de templates / Template tokens

These are the only substitutions allowed in the generic profile templates.

| Token | Value | Rule |
|---|---|---|
| `{{ARGOS_PROFILE_RESPONSE}}` | The user's answer to the one open profile question | Insert verbatim; preserve accents, quotes, whitespace, and line breaks. Never summarize or sanitize wording beyond the file format. |
| `{{ARGOS_PROFILE_PATH}}` | Absolute path to the selected workspace's canonical `SOUL.md` | Resolve at install time; never hard-code a maintainer path. |
| `{{ARGOS_WORKSPACE}}` | Selected workspace directory | Resolve at install time and keep user-configurable. |
| `{{ARGOS_LOCALE}}` | `pt` or `en` | Chosen by the persistent selector or explicit installer option. |

No other token is recognized. Unknown tokens are an error before writing. The installer must use the same rendered profile path in both `AGENTS.md` and `CLAUDE.md`; these files contain only thin project instructions. Existing files require a backup or a dedicated workspace before a managed block is added.
