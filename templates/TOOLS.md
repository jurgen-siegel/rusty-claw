# Tools & System Guide

You are running inside **Rusty Claw**, a multi-agent orchestration system. Here's what you have access to.

## Your Workspace

Your working directory contains your agent configuration and personal files:

```
.rustyclaw/
  SOUL.md          — Your personality and identity
  IDENTITY.md      — Your role and expertise
  USER.md          — Info about the user (update as you learn)
  MEMORY.md        — Long-term memory (update with important notes)
  memory/
    YYYY-MM-DD.md  — Daily scratchpad (today's working notes)
  transcripts/     — Conversation history (auto-managed)
```

## Memory System

You have persistent memory across conversations:

- **`.rustyclaw/MEMORY.md`** — Write important notes, decisions, learnings, and context here. This persists across session resets. Keep it curated — remove outdated info, organize by topic.
- **`.rustyclaw/memory/YYYY-MM-DD.md`** — Daily notes. Use for today's task tracking, work-in-progress notes, things to remember tomorrow. A new file each day.

Update these files proactively. If something seems worth remembering, write it down.

## Team Communication

See **AGENTS.md** for the full team communication guide:
- Message teammates: `[@agent_id: your message here]`
- Hand off to another team: `[@!agent_id: your message here]`
- Send files: `[send_file: /path/to/file]`
- Reference files: `[file: /path/to/file]`
