"use client";

import { useTranslation } from "@flow-like/locales";
import { Check, Copy } from "lucide-react";
import type { ReactNode } from "react";
import { DvButton } from "./dv-button";
import { cx } from "./tone";
import { useCopy } from "./use-copy";

/**
 * SPEC §4.33: a command to run on the device, with Copy. One line scrolls
 * sideways (never wraps); multi-line commands render as a block.
 */
export function CommandBlock({
	command,
	note,
	className,
}: Readonly<{
	command: string;
	/** What the command shows or does. */
	note?: ReactNode;
	className?: string;
}>) {
	const { t } = useTranslation("devices");
	const { copied, copy } = useCopy();
	const multiline = command.includes("\n");
	const copyButton = (
		<DvButton
			size="xs"
			variant="ghost"
			icon={copied ? Check : Copy}
			aria-label={
				copied
					? t("view.cmd.copied", "Copied")
					: t("view.cmd.copyLabel", "Copy command: {{command}}", { command })
			}
			onClick={() => copy(command)}
			className="shrink-0"
		>
			{copied ? t("view.cmd.copied", "Copied") : t("view.cmd.copy", "Copy")}
		</DvButton>
	);
	return (
		<div data-command="" className={cx("flex min-w-0 flex-col", className)}>
			{multiline ? (
				<div className="relative min-w-0 rounded-lg border border-border bg-surface-sunken">
					<pre className="m-0 overflow-auto px-3 py-2.5 pr-20 font-mono text-[12.5px] leading-5 whitespace-pre">
						<code>{command}</code>
					</pre>
					<span className="absolute top-1 right-1">{copyButton}</span>
				</div>
			) : (
				<div className="flex min-w-0 items-center gap-2 rounded-lg border border-border bg-surface-sunken py-0.75 pr-0.75 pl-3">
					<code className="min-w-0 flex-auto overflow-x-auto font-mono text-[12.5px] leading-6 whitespace-nowrap [scrollbar-width:thin]">
						{command}
					</code>
					{copyButton}
				</div>
			)}
			{note ? (
				<p className="mt-1 text-xs text-muted-foreground">{note}</p>
			) : null}
		</div>
	);
}
