<!-- TRELLIS:START -->
# Trellis Instructions

These instructions are for AI assistants working in this project.

Use the `/trellis:start` command when starting a new session to:
- Initialize your developer identity
- Understand current project context
- Read relevant guidelines

Use `@/.trellis/` to learn:
- Development workflow (`workflow.md`)
- Project structure guidelines (`spec/`)
- Developer workspace (`workspace/`)

If you are using Codex, project-scoped helpers may also live in:
- `.agents/skills/` for reusable Trellis skills
- `.codex/agents/` for optional custom subagents

## Skill System

Reusable Trellis skills are located in `.trellis/skills/` (canonical source).

**Platform discovery:**
- **Codex Desktop**: `.agents/skills/` (junctions to `.trellis/skills/`)
- **Hermes Desktop**: `~/.hermes/skills/` (symlinks to `.trellis/skills/`)
- **Claude Code**: `.claude/commands/trellis/` (slash commands)

**Available skills:**
- `trellis-start` — Initialize session and start task workflow
- `trellis-finish-work` — Pre-commit quality checklist
- `trellis-brainstorm` — Requirements discovery and PRD writing
- `trellis-before-dev` — Read development guidelines before coding

Keep this managed block so `trellis update` can refresh the instructions.

<!-- TRELLIS:END -->
