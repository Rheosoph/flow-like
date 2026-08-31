"use client";

import { useTranslation } from "@flow-like/locales";
import CheckIcon from "lucide-react/dist/esm/icons/check.js";
import ChevronDown from "lucide-react/dist/esm/icons/chevron-down.js";
import StopIcon from "lucide-react/dist/esm/icons/circle-stop.js";
import CopyIcon from "lucide-react/dist/esm/icons/copy.js";
import CreditCardIcon from "lucide-react/dist/esm/icons/credit-card.js";
import ScissorsIcon from "lucide-react/dist/esm/icons/file-warning.js";
import HourglassIcon from "lucide-react/dist/esm/icons/hourglass.js";
import KeyRoundIcon from "lucide-react/dist/esm/icons/key-round.js";
import ServerCrashIcon from "lucide-react/dist/esm/icons/server-crash.js";
import ShieldAlertIcon from "lucide-react/dist/esm/icons/shield-alert.js";
import SlidersIcon from "lucide-react/dist/esm/icons/sliders-horizontal.js";
import TerminalIcon from "lucide-react/dist/esm/icons/terminal.js";
import TriangleAlert from "lucide-react/dist/esm/icons/triangle-alert.js";
import WifiOffIcon from "lucide-react/dist/esm/icons/wifi-off.js";
import { useRouter } from "next/navigation";
import { useCallback, useMemo, useState } from "react";
import type {
	ChatErrorKind,
	IChatMessageError,
} from "../../../lib/flowpilot/chat-error";
import { cn } from "../../../lib/utils";
import { openUpgradeDialogIfEnabled } from "../../../state/upgrade-dialog-state";
import { Button } from "../../ui/button";
import {
	Collapsible,
	CollapsibleContent,
	CollapsibleTrigger,
} from "../../ui/collapsible";

/**
 * How loud the card is. A failure the user caused (stopped the run, hit a limit) or can fix in two
 * clicks (no model configured, plan too small) is not a red alarm — only something that actually
 * broke gets the destructive treatment.
 */
type ErrorTone = "danger" | "action" | "neutral";

const TONE: Record<ChatErrorKind, ErrorTone> = {
	cancelled: "neutral",
	"rate-limit": "neutral",
	input: "neutral",
	config: "action",
	billing: "action",
	auth: "action",
	permission: "danger",
	network: "danger",
	server: "danger",
	backend: "danger",
	generic: "danger",
};

const ICON: Record<ChatErrorKind, typeof TriangleAlert> = {
	cancelled: StopIcon,
	"rate-limit": HourglassIcon,
	input: ScissorsIcon,
	config: SlidersIcon,
	billing: CreditCardIcon,
	auth: KeyRoundIcon,
	permission: ShieldAlertIcon,
	network: WifiOffIcon,
	server: ServerCrashIcon,
	backend: TerminalIcon,
	generic: TriangleAlert,
};

const TONE_STYLES: Record<
	ErrorTone,
	{ card: string; icon: string; title: string }
> = {
	danger: {
		card: "border-destructive/25 bg-destructive/[0.04]",
		icon: "bg-destructive/10 text-destructive",
		title: "text-foreground",
	},
	action: {
		card: "border-primary/25 bg-primary/[0.04]",
		icon: "bg-primary/10 text-primary",
		title: "text-foreground",
	},
	neutral: {
		card: "border-border bg-muted/30",
		icon: "bg-muted text-muted-foreground",
		title: "text-foreground",
	},
};

function CopyButton({
	text,
	label,
	className,
}: {
	text: string;
	label: string;
	className?: string;
}) {
	const [copied, setCopied] = useState(false);
	const copy = useCallback(() => {
		navigator.clipboard
			.writeText(text)
			.then(() => {
				setCopied(true);
				setTimeout(() => setCopied(false), 1_500);
			})
			.catch(() => undefined);
	}, [text]);

	return (
		<Button
			variant="ghost"
			size="sm"
			onClick={copy}
			className={cn(
				"h-7 gap-1.5 px-2 text-xs text-muted-foreground hover:text-foreground",
				className,
			)}
		>
			{copied ? (
				<CheckIcon className="size-3.5" />
			) : (
				<CopyIcon className="size-3.5" />
			)}
			{label}
		</Button>
	);
}

/**
 * A failed assistant turn, rendered as a card instead of an apology sentence in the answer: what
 * broke, in one line the user can act on, with the raw failure kept one click away.
 */
export function ChatMessageError({
	error,
}: { readonly error: IChatMessageError }) {
	const { t } = useTranslation("chat");
	const router = useRouter();
	const [detailsOpen, setDetailsOpen] = useState(false);

	const tone = TONE[error.kind] ?? "danger";
	const styles = TONE_STYLES[tone];
	const Icon = ICON[error.kind] ?? TriangleAlert;

	const meta = useMemo(
		() =>
			[
				error.code,
				error.status ? `HTTP ${error.status}` : undefined,
				error.reference
					? `${t("errorReference", "Reference")} ${error.reference}`
					: undefined,
			].filter(Boolean) as string[],
		[error.code, error.status, error.reference, t],
	);

	const clipboardText = useMemo(
		() =>
			[
				`${error.title}: ${error.message}`,
				...meta,
				error.command ? `$ ${error.command}` : undefined,
				error.detail,
			]
				.filter(Boolean)
				.join("\n"),
		[error.title, error.message, error.command, error.detail, meta],
	);

	const hasDetails = Boolean(error.detail || error.command || meta.length > 0);

	const runAction = useCallback(() => {
		if (!error.action) return;
		if (error.action.kind === "upgrade") {
			// A hub with conversion disabled has no dialog to open — send the user to the place
			// where they can pick a model their plan does cover instead of doing nothing.
			const opened = openUpgradeDialogIfEnabled({
				reason: "model-tier",
				message: error.message,
			});
			if (!opened) router.push("/settings/ai?tab=models");
			return;
		}
		if (error.action.href) router.push(error.action.href);
	}, [error.action, error.message, router]);

	return (
		<div
			className={cn(
				"mt-2 w-full overflow-hidden rounded-xl border px-3 py-3",
				styles.card,
			)}
			data-fl-chat-error={error.kind}
			role="alert"
		>
			<div className="flex items-start gap-3">
				<span
					className={cn(
						"mt-0.5 flex size-7 shrink-0 items-center justify-center rounded-lg",
						styles.icon,
					)}
				>
					<Icon className="size-4" />
				</span>
				<div className="min-w-0 flex-1 space-y-1">
					<p className={cn("text-sm font-medium leading-snug", styles.title)}>
						{error.title}
					</p>
					<p className="whitespace-pre-wrap break-words text-[13px] leading-relaxed text-muted-foreground">
						{error.message}
					</p>
					<div className="flex flex-wrap items-center gap-1 pt-1.5">
						{error.action && (
							<Button
								size="sm"
								variant={tone === "danger" ? "outline" : "default"}
								onClick={runAction}
								className="h-7 px-2.5 text-xs"
							>
								{error.action.label}
							</Button>
						)}
						<CopyButton
							text={clipboardText}
							label={t("copyErrorDetails", "Copy details")}
							// Without an action button ahead of it the ghost button's own padding
							// pushes its label out of the card's text column.
							className={error.action ? undefined : "-ml-2"}
						/>
					</div>
					{hasDetails && (
						<Collapsible open={detailsOpen} onOpenChange={setDetailsOpen}>
							<CollapsibleTrigger asChild>
								<button
									type="button"
									className="flex items-center gap-1 rounded-md py-1 text-[11px] font-medium text-muted-foreground outline-none transition-colors hover:text-foreground focus-visible:ring-2 focus-visible:ring-primary/40"
								>
									<span>{t("technicalDetails", "Technical details")}</span>
									<ChevronDown
										className={cn(
											"size-3 transition-transform",
											detailsOpen && "rotate-180",
										)}
									/>
								</button>
							</CollapsibleTrigger>
							<CollapsibleContent>
								<div className="mt-1 space-y-2 rounded-lg border border-border/50 bg-background/60 p-2.5">
									{meta.length > 0 && (
										<div className="flex flex-wrap gap-1.5">
											{meta.map((entry) => (
												<span
													key={entry}
													className="rounded-md bg-muted px-1.5 py-0.5 font-mono text-[10px] text-muted-foreground"
												>
													{entry}
												</span>
											))}
										</div>
									)}
									{error.command && (
										<code className="block overflow-x-auto whitespace-pre rounded-md bg-muted px-2 py-1.5 font-mono text-[11px] text-foreground">
											{error.command}
										</code>
									)}
									{error.detail && (
										<pre className="max-h-40 overflow-auto whitespace-pre-wrap break-words font-mono text-[11px] leading-relaxed text-muted-foreground">
											{error.detail}
										</pre>
									)}
								</div>
							</CollapsibleContent>
						</Collapsible>
					)}
				</div>
			</div>
		</div>
	);
}
