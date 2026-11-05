"use client";

import ChevronDown from "lucide-react/dist/esm/icons/chevron-down.js";
import { useId, useState } from "react";
import { cn } from "../../../lib/utils";
import type { IToolActivityDetail } from "./chat-db";

type Terminal = NonNullable<IToolActivityDetail["terminal"]>;

function exitLabel(terminal: Terminal): string | undefined {
	if (terminal.timedOut) return "timed out";
	if (terminal.exitCode === undefined) return undefined;
	return terminal.exitCode === null ? "killed" : `exit ${terminal.exitCode}`;
}

export function TerminalCard({ terminal }: { terminal: Terminal }) {
	const [open, setOpen] = useState(false);
	const outputId = useId();
	const label = exitLabel(terminal);
	const failed = terminal.timedOut || (terminal.exitCode ?? 0) !== 0;
	const hasOutput = Boolean(terminal.output?.trim());
	return (
		<div
			className="min-w-0 rounded-md border border-border/60 bg-muted/40 font-mono text-[11px]"
			data-terminal-card
		>
			<button
				type="button"
				onClick={() => setOpen((value) => !value)}
				disabled={!hasOutput}
				aria-expanded={open}
				aria-controls={outputId}
				className="flex w-full min-w-0 items-center gap-2 px-2 py-1 text-left focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary/40 disabled:cursor-default"
			>
				<span className="text-muted-foreground" aria-hidden="true">
					$
				</span>
				<span
					className="min-w-0 flex-1 truncate text-foreground/90"
					title={terminal.command}
				>
					{terminal.command}
				</span>
				{label && (
					<span
						data-terminal-exit
						className={cn(
							"shrink-0 rounded border px-1 text-[10px]",
							failed
								? "border-destructive/40 text-destructive"
								: "border-border/60 text-muted-foreground",
						)}
					>
						{label}
					</span>
				)}
				{hasOutput && (
					<ChevronDown
						className={cn(
							"size-3 shrink-0 text-muted-foreground transition-transform",
							open && "rotate-180",
						)}
						aria-hidden="true"
					/>
				)}
			</button>
			{open && hasOutput && (
				<pre
					id={outputId}
					className="max-h-64 overflow-auto whitespace-pre-wrap break-words border-t border-border/60 px-2 py-1.5 text-foreground/80"
				>
					{terminal.output}
				</pre>
			)}
		</div>
	);
}
