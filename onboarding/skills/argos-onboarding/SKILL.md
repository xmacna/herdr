---
name: argos-onboarding
description: Configure a generic Argos workspace from the supplied manifest and profile templates. Use whenever a user asks to install Argos, configure Claude Code or Codex, choose onboarding skills, create SOUL.md/AGENTS.md/CLAUDE.md, or include/exclude WhatsApp. Keep actions CLI-driven, explicit, idempotent, and safe for an existing profile.
compatibility: Windows-first; requires the official installer/integrator or a normal Herdr terminal, but does not install tools itself.
---

# Argos onboarding

Use this skill as the instruction layer for the official installer. It does not replace Herdr, a provider CLI, authentication, or the installer owned by the integration package.

## Contract

1. Read `../manifest.json` as the selection authority.
2. Treat `../../templates/{locale}/SOUL.md` as the only profile source. `AGENTS.md` and `CLAUDE.md` are thin project instructions that reference that same file.
3. Preserve `{{ARGOS_PROFILE_RESPONSE}}` literally, including accents, quotes, line breaks, and user wording. Do not summarize it with an LLM.
4. Resolve every destination from the selected workspace/profile directory. Never use a maintainer's home path, account, host, store, phone number, or private brand as a default.
5. Make changes idempotently: preserve an existing file with a dated backup or a dedicated workspace, and never silently overwrite user instructions.

## Selection

The core set is always available: the current Herdr runtime skill, generic Herdr coordination guidance, handoff guidance, and the profile templates. Development skills are opt-in. WhatsApp is selected by default, but `--without-whatsapp` must remove it entirely before any download, install, pairing, or skill copy.

Do not infer that a skill is redistributable from its name. Use the source/license/constraint fields in the manifest. When a source has no established redistribution license, keep a reference to its official installation/source location and write a new generic adapter instead of copying it.

## Provider checks

Check Claude Code and Codex independently. A smoke check may report whether a CLI is present and whether its own status command is readable; it must not log in, log out, copy auth files, ask for tokens, or launch an agent. One authenticated provider is enough to continue. An absent, disconnected, or network-failed provider is a visible state, not a successful install.

For an unlisted CLI, use a normal Herdr terminal and a user-supplied command. Report capabilities as unknown until that CLI documents and proves them. Do not claim that all CLIs load `AGENTS.md`, `CLAUDE.md`, `SOUL.md`, skills, resume, or structured auth in the same way. Do not claim a universal `@import`.

## Profile rendering

Ask exactly one open question in the selected locale:

- PT: `Como você gostaria que seu Argos conversasse e trabalhasse com você?`
- EN: `How would you like your Argos to communicate and work with you?`

The answer is written verbatim into `SOUL.md`. The renderer may replace only the documented tokens in `../../templates/TOKENS.md`; it must not change permission boundaries. The generated `AGENTS.md` and `CLAUDE.md` point to the same canonical profile path and state that they are project instructions, not provider system prompts.

Communication preferences never authorize purchases, messages, pairing, destructive commands, process termination, or external publication. Editing the profile later must remain available through preferences or a documented CLI command.

## WACLI boundary

When selected, show the official WACLI version/license/Windows availability and use the explicit account selector required by that release. Installing does not pair an account or send a message. Pairing is a separate human-confirmed action. If the Windows artifact or license is not verified, report that limit instead of claiming success.

When deselected, the result must contain no WACLI download, install, pairing, skill, store, or account change. Existing WACLI installations remain untouched in either path.

## Evidence and rollback

Use disposable fake command directories for installed/authenticated, installed/disconnected, absent, one-provider, network-failure, reinstall-preserving-profile, PT/EN quoting, WACLI default/opt-out, and generic-CLI-without-inference cases. Real smoke checks on the current machine are read-only evidence only; they are not proof of a clean installation elsewhere.

Report the selected manifest revision, template paths, provider observations, files changed, backups, and rollback command. Keep secrets, auth files, transcripts, private identifiers, and raw process output out of the report and Git.

## Direct CLI entry

The human-facing installer and the short Claude Code instruction must invoke the same official idempotent mechanism. The short instruction should say where that mechanism is, which manifest it consumes, and how to inspect its result; it must not contain a second installer implementation.
