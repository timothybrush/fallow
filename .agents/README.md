# Agent Skills

This directory contains repo-scoped resources for agents that support the open Agent Skills format.

The contents here are additive. They do not replace Fallow's existing tool-specific agent configuration.

## Existing agent configuration

Fallow also keeps agent-specific configuration in these locations:

- `CLAUDE.md`: general repository guidance and working conventions. Some agents, including Pi, can load this as startup context.
- `.claude/agents/`: Claude-specific subagent definitions.
- `.codex/agents/`: Codex-specific custom agent definitions.

Do not remove or rename those files when adding portable skills.

## Portable skills

Repo-scoped Agent Skills live under:

```text
.agents/skills/<skill-name>/SKILL.md
```

Each skill follows the Agent Skills specification:

- the skill is a directory containing `SKILL.md`
- `SKILL.md` starts with YAML frontmatter
- `name` and `description` are required
- `name` matches the parent directory and uses lowercase letters, numbers, and hyphens
- `description` explains what the skill does and when an agent should use it

Use `.agents/skills/` for focused, repeatable workflows that can be reused by multiple agent harnesses.

## Provenance and references

The initial skills in this directory are adapted from existing reviewer definitions under `.claude/agents/`. The initial skill bodies are content-preserving adaptations of those source files; only the frontmatter is changed to match the Agent Skills format.

Reference:

- Agent Skills specification: https://agentskills.io/specification
- Agent Skills clients: https://agentskills.io/clients

## Pi usage

Pi already loads `CLAUDE.md` for general project context. Skills are separate: after the project is trusted, Pi can discover directories under `.agents/skills/` that contain `SKILL.md`.

In Pi, a contributor can force-load a skill with:

```text
/skill:ci-formats-reviewer
```

## Compatibility

This directory is intentionally tool-agnostic. Tool-specific agent definitions remain in their native locations.
Claude-specific subagents remain under `.claude/agents/`, and Codex-specific custom agents remain under `.codex/agents/`.
Compatible clients and harnesses that support Agent Skills can additionally discover `.agents/skills/`.
