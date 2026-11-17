"use client";

import { BotAvatar } from "@flow-like/flow-like-ui/components/bots";
import { Button } from "@flow-like/flow-like-ui/components/ui/button";
import type { BotApprovalDecision } from "@flow-like/flow-like-ui/state/backend-state/bot-state";
import { useTranslation } from "@flow-like/locales";
import {
	Globe,
	type LucideIcon,
	Mail,
	MessageSquare,
	Monitor,
	Sparkles,
	SquareTerminal,
	X,
	Zap,
} from "lucide-react";
import type { Ref } from "react";
import type { NotchBot } from "../../lib/bot-notch/protocol";
import type { NotchCard } from "./use-notch-tasks";

const DECISIONS: readonly BotApprovalDecision[] = ["once", "always", "deny"];

/** The kind of action an approval asks about, as an icon. */
function toolIcon(tool: string): LucideIcon {
	if (tool.startsWith("computer_")) return Monitor;
	if (tool.startsWith("browser_") || tool.startsWith("web_")) return Globe;
	if (tool === "shell_run" || tool === "code_run") return SquareTerminal;
	if (tool.startsWith("email")) return Mail;
	return Sparkles;
}

const CARD_SHELL =
	"w-72 overflow-hidden rounded-2xl border border-foreground/10 bg-popover/95 text-popover-foreground shadow-[0_18px_40px_-12px_rgb(0_0_0/0.65)] backdrop-blur-xl animate-in fade-in-0 zoom-in-95 slide-in-from-top-1 duration-200";

function CardHeader({
	bot,
	label,
	tone,
}: Readonly<{
	bot: NotchBot;
	label: string;
	tone: "attention" | "quiet" | "failed";
}>) {
	return (
		<div className="flex items-center gap-2 px-3 pt-3">
			<BotAvatar
				appearance={bot.appearance}
				size={20}
				animated={false}
				decorative
			/>
			<span className="min-w-0 flex-1 truncate text-xs font-semibold">
				{bot.name}
			</span>
			<span
				className={
					tone === "attention"
						? "text-[11px] font-medium text-primary"
						: tone === "failed"
							? "text-[11px] font-medium text-destructive"
							: "text-[11px] text-muted-foreground"
				}
			>
				{label}
			</span>
		</div>
	);
}

export function NotchCardView({
	nodeRef,
	bot,
	card,
	status,
	onAnswer,
	onAutoApprove,
	onDismiss,
	onOpen,
}: Readonly<{
	nodeRef: Ref<HTMLDivElement>;
	bot: NotchBot;
	card: NotchCard | null;
	status: string | null;
	onAnswer: (decision: BotApprovalDecision) => void;
	onAutoApprove: () => void;
	onDismiss: () => void;
	onOpen: () => void;
}>) {
	const { t } = useTranslation("bots");
	if (!card) {
		return (
			<div ref={nodeRef} className="pointer-events-none absolute left-0 top-0">
				<div className="flex max-w-64 items-center gap-2 rounded-full border border-foreground/10 bg-popover/95 px-3 py-1 text-xs text-popover-foreground shadow-lg backdrop-blur-xl animate-in fade-in-0 zoom-in-95 duration-150">
					<span className="size-1.5 shrink-0 animate-pulse rounded-full bg-primary" />
					<span className="truncate">{status}</span>
				</div>
			</div>
		);
	}
	if (card.kind === "approval") {
		const labels: Record<BotApprovalDecision, string> = {
			once: t("notch.allowOnce", "Allow once"),
			always: t("notch.allowAlways", "Always"),
			deny: t("notch.deny", "Deny"),
		};
		const Icon = toolIcon(card.approval.tool);
		const batch = card.approval.items?.length ?? 0;
		return (
			<div ref={nodeRef} className="absolute left-0 top-0">
				<div className={CARD_SHELL}>
					<CardHeader
						bot={bot}
						label={t("notch.needsYou", "Needs you")}
						tone="attention"
					/>
					<div className="flex gap-2.5 px-3 pt-2.5">
						<span className="flex size-7 shrink-0 items-center justify-center rounded-lg bg-muted text-muted-foreground">
							<Icon className="size-3.5" aria-hidden />
						</span>
						<p className="text-sm leading-snug">
							{card.approval.summary || card.approval.tool}
							{batch > 1 && (
								<span className="mt-0.5 block text-xs text-muted-foreground">
									{t("notch.batch", "{{number}} actions in one go", {
										number: batch,
									})}
								</span>
							)}
						</p>
					</div>
					<div className="flex flex-wrap gap-1.5 px-3 pt-3">
						{DECISIONS.map((decision) => (
							<Button
								key={decision}
								size="sm"
								variant={
									decision === "once"
										? "default"
										: decision === "always"
											? "secondary"
											: "ghost"
								}
								className="h-7 rounded-full px-3 text-xs"
								onClick={() => onAnswer(decision)}
							>
								{labels[decision]}
							</Button>
						))}
					</div>
					<button
						type="button"
						onClick={onAutoApprove}
						className="mt-3 flex w-full items-center gap-1.5 border-t border-foreground/10 px-3 py-2 text-left text-xs text-muted-foreground transition-colors hover:bg-foreground/5 hover:text-foreground"
					>
						<Zap className="size-3 shrink-0" aria-hidden />
						{t(
							"notch.dontAskAgain",
							"Allow and stop asking — {{name}} acts on its own",
							{
								name: bot.name,
							},
						)}
					</button>
				</div>
			</div>
		);
	}
	return (
		<div ref={nodeRef} className="absolute left-0 top-0">
			<div className={CARD_SHELL}>
				<CardHeader
					bot={bot}
					label={
						card.failed
							? t("notch.failedLabel", "Couldn't finish")
							: t("notch.done", "Done")
					}
					tone={card.failed ? "failed" : "quiet"}
				/>
				<p className="line-clamp-5 px-3 pt-2 text-sm leading-snug">
					{card.text}
				</p>
				<div className="flex items-center justify-end gap-1 px-2 pt-1.5 pb-2">
					<Button
						size="sm"
						variant="ghost"
						className="h-7 gap-1.5 rounded-full px-2.5 text-xs"
						onClick={onOpen}
					>
						<MessageSquare className="size-3.5" />
						{t("notch.openChat", "Open chat")}
					</Button>
					<Button
						size="icon"
						variant="ghost"
						className="size-7 rounded-full"
						onClick={onDismiss}
						aria-label={t("notch.dismiss", "Dismiss")}
					>
						<X className="size-3.5" />
					</Button>
				</div>
			</div>
		</div>
	);
}
