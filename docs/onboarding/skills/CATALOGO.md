# Catálogo de skills e templates — onboarding Argos

**Revisão:** 2026-09-11  
**Posse:** `onboarding/skills/`, `onboarding/templates/`, `docs/onboarding/skills/`  
**Estado:** pacote local acionável; sem instalação, publicação, login, pareamento, envio ou alteração de runtime.

## Decisão

O manifesto consumível está em [`onboarding/skills/manifest.json`](../../../onboarding/skills/manifest.json). Ele define `schema_version: 1`, a fonte única `SOUL.md`, tokens permitidos, core obrigatório, desenvolvimento opt-in e WACLI selecionado por padrão com exclusão total por `--without-whatsapp`.

A skill genérica criada é [`argos-onboarding`](../../../onboarding/skills/argos-onboarding/SKILL.md). Ela orienta o mecanismo oficial idempotente; não cria um instalador concorrente, não autentica provedores e não lança inferência.

## Matriz de candidatos

| ID | Grupo | Decisão | Fonte/revisão verificada | Licença | Dependências/limite |
|---|---|---|---|---|---|
| `herdr-runtime` | core | incluir como referência | skill Herdr, SHA-256 `25A68BC3…9DB1E9F` | Apache-2.0 do repositório | CLI Herdr + `HERDR_ENV=1`; seguir help/runtime real, não snapshot |
| `herdr-coordination` | core | adaptar | mecânica `herdr-gerente`, SHA-256 `3E4654A0…A2C720` | não estabelecida na fonte | reescrever sem nomes, caminhos, contas ou doutrina privados |
| `handoff` | core | adaptar | skill handoff, SHA-256 `D215DD8F…3D7B0A3` | não estabelecida na fonte | handoff sanitizado, recuperável e sem transcript integral |
| `profile` | core | incluir como autoria nova | templates deste pacote | termos do projeto | somente `SOUL.md` é fonte canônica |
| `wacli-generic` | WhatsApp | incluir por padrão, excluir integralmente por flag | referência `wacli-hermes`, SHA-256 `BE542755…2B5B` | não estabelecida na fonte | WACLI escolhido pelo usuário; sem conta/store/JID/telefone; sem pareamento |
| `skill-creator-adapter` | development | opcional/adaptar | referência oficial Anthropic, SHA-256 `DCD4803E…C2BCF` | Apache-2.0 (`LICENSE.txt` observado) | não duplicar o nome `skill-creator` |
| `diagnose` | development | opcional/referência | SHA-256 `28886402…B6D13` | não estabelecida | feedback loop concreto; script HITL não roda no onboarding |
| `tdd` | development | opcional/referência | SHA-256 `AF059705…70638` | não estabelecida | runner do projeto; não exigido pelo instalador |
| `architecture-review` | development | opcional/referência | SHA-256 `F00DE04C…6C749` | não estabelecida | glossário/ADRs; sem relatório gráfico no onboarding |
| `zoom-out` | development | opcional/referência | SHA-256 `2A6894C7…ACF48` | não estabelecida | contexto do projeto; não exigido |
| `git-guardrails` | development | opcional/referência | SHA-256 `03C4DE76…1C29F7` | não estabelecida | não instala hook como efeito colateral |
| `setup-pre-commit` | development | opcional/referência | SHA-256 `872F4037…F63BBD` | não estabelecida | só ação posterior explícita pode instalar dependências/hooks |
| `conselho-workflow` | development | opcional/adaptar | SHA-256 `F4B2C19B…572F24` | não estabelecida | guia configurável; nunca convocado automaticamente |

### Scripts auditados

- `herdr-gerente`: `despachar.sh`, `registrar.sh`, `scan.sh`. Mecânica local útil como referência; não copiada porque a licença de redistribuição não foi estabelecida.
- `skill-creator`: `quick_validate.py`, `package_skill.py`, `run_eval.py`, `run_loop.py`, `aggregate_benchmark.py`, `generate_report.py`, `improve_description.py`, `utils.py`. `quick_validate.py` foi usado; os demais não são dependência do onboarding.
- `diagnose`: `hitl-loop.template.sh`. Mantido fora do pacote; onboarding usa fakes controlados, não loop interativo.
- `git-guardrails`: `block-dangerous-git.sh`. Referência opt-in; nenhum hook foi instalado.
- Os demais candidatos não têm scripts na fonte auditada.

## Fora do padrão

`xmacna-*`, Redigir/publicadores, `nyx-agent-bridge`, `openclaw-watch` e a skill original `wacli-hermes` ficam excluídos por acoplamento a marca, máquina, serviço, conta, store ou infraestrutura privada. A única exceção é a adaptação genérica `wacli-generic`, que não carrega nenhum dado operacional.

Ausência de licença não foi tratada como autorização de cópia. A entrega usa referências descritivas e autoria nova; uma futura redistribuição deve confirmar a licença e preservar avisos das fontes licenciadas.

## Templates e contrato de carregamento

Há versões `pt` e `en` de `SOUL.md`, `AGENTS.md` e `CLAUDE.md` em [`onboarding/templates/`](../../../onboarding/templates/). `SOUL.md` recebe literalmente a resposta única do usuário; os outros dois apontam para o mesmo caminho absoluto via `{{ARGOS_PROFILE_PATH}}`. Eles são instruções de projeto, não system prompts universais. O formato completo de tokens está em [`TOKENS.md`](../../../onboarding/templates/TOKENS.md).

As instruções curtas para a segunda entrada estão em [`COLE-NO-CLAUDE.pt.md`](../../../onboarding/COLE-NO-CLAUDE.pt.md) e [`PASTE-INTO-CLAUDE.en.md`](../../../onboarding/PASTE-INTO-CLAUDE.en.md). Ambas apontam ao mesmo manifesto/procedimento, sem uma segunda implementação do instalador.

## Provas e limites

Executado em diretórios temporários descartáveis:

```text
Invoke-OnboardingContractTests.ps1: PASS
manifest schema, core, WACLI default/opt-out, templates, literal UTF-8,
backup de arquivo existente e fixtures de provedores separados
quick_validate.py argos-onboarding: Skill is valid!
```

Os fixtures cobrem instalado/autenticado, um provedor, desconectado e falha de rede sem credenciais. Não houve logout/login, download, instalação, QR, mensagem, inferência ou teste de instalação limpa. O estado real de autenticação deste PC continua evidência somente de smoke read-only; não prova outra máquina.

## Handoff ao build

O agente existente `herdr-build-luna` foi avisado no pane `w1T:p3`. O contrato para integração é: consumir `onboarding/skills/manifest.json`; resolver `../templates/{locale}` a partir da pasta do manifesto; substituir somente os quatro tokens documentados; preservar a resposta literal; aplicar `--without-whatsapp` antes de qualquer download; e chamar o mecanismo oficial idempotente com backup/rollback. Nenhum Rust foi reaberto e nenhum servidor foi reiniciado.
