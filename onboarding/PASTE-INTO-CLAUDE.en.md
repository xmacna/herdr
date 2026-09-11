# Short instruction — paste into Claude Code

Use this package's official Argos onboarding procedure. Read `onboarding/skills/manifest.json` as the consumable selection manifest and follow `onboarding/skills/argos-onboarding/SKILL.md`. Ask exactly one question:

> How would you like your Argos to communicate and work with you?

Preserve the answer literally in one `SOUL.md` in the selected workspace; make `AGENTS.md` and `CLAUDE.md` reference that same file. Detect Claude Code and Codex separately without requesting tokens, copying auth files, or launching inference during checks. Keep WhatsApp selected by default only when the user accepts; `--without-whatsapp` must remove it before any download. Reuse the official idempotent installer mechanism with backup/rollback, and report real results and limits.
