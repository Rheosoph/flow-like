---
applyTo: "**"
---
# Project general coding standards

## GIT OPERATIONS
- `git diff` is fine. NEVER stash, reset, or run any other destructive/state-mutating git command without explicit confirmation — the user is actively working in the same codebase.

## Comments and Verbosity
- Do not comment code that is self-explanatory.
- Avoid verbose comments; use concise, clear language.

## Focus on DRY
- Follow the DRY (Don't Repeat Yourself) principle.
- Use components, functions, and utilities to avoid code duplication.
- Do not create huge logic blocks; break them into smaller, reusable functions.

## NEVER RETURN DIFF MODE CODE SNIPPETS
I cant copy paste them if they make sense.. better return drop in replacements

## Subagents
whenever a task can be worked on in parallel, first create a todo list, either in todo/*.md or in memory and then feel free to create subagents to work on the individual tasks in parallel.