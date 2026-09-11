# Instrução curta — cole no Claude Code

Use o procedimento oficial de onboarding do Argos neste pacote. Leia `onboarding/skills/manifest.json` como manifesto consumível e siga `onboarding/skills/argos-onboarding/SKILL.md`. Pergunte apenas:

> Como você gostaria que seu Argos conversasse e trabalhasse com você?

Preserve a resposta literalmente em um único `SOUL.md` no workspace escolhido; faça `AGENTS.md` e `CLAUDE.md` referenciarem esse mesmo arquivo. Detecte Claude Code e Codex separadamente, sem pedir tokens, copiar arquivos de autenticação ou iniciar inferência durante a checagem. Use a opção padrão de WhatsApp somente se o usuário aceitar; `--without-whatsapp` deve removê-lo antes de qualquer download. Reutilize o mecanismo oficial idempotente do instalador, com backup/rollback, e mostre o resultado e os limites reais.
