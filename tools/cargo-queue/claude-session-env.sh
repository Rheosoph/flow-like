#!/bin/sh
[ -n "$CLAUDE_ENV_FILE" ] || exit 0
bin=$(CDPATH= cd -- "$(dirname -- "$0")/bin" && pwd -P) || exit 0
printf 'case ":$PATH:" in *":%s:"*) ;; *) export PATH="%s:$PATH" ;; esac\n' "$bin" "$bin" >> "$CLAUDE_ENV_FILE"
