"use client";

import { useTranslation } from "@flow-like/locales";
import ChevronUpIcon from "lucide-react/dist/esm/icons/chevron-up.js";
import ZapIcon from "lucide-react/dist/esm/icons/zap.js";
import { useCallback, useMemo, useState } from "react";
import { useIsMobile } from "../../../hooks/use-mobile";
import { readUsageStat } from "../../../lib/llm/usage";
import { cn } from "../../../lib/utils";
import { Popover, PopoverContent, PopoverTrigger } from "../../ui/popover";
import {
	Sheet,
	SheetContent,
	SheetDescription,
	SheetTitle,
} from "../../ui/sheet";
import type { IChatUsageStat } from "./chat-db";
import {
	formatCost,
	formatDuration,
	formatTokens,
	usageBreakdown,
} from "./usage-breakdown";
import { usageEfficiency } from "./usage-efficiency";
import { UsageReceipt } from "./usage-receipt";

// The chip also sits inside FlowPilot panels, which stack very high (embedded board
// panel z-100, global overlay z-9999), so the usage surface is raised above them.
const ABOVE_PANELS = "z-10000";

export function UsageStats({
	stats: rawStats,
	className,
}: Readonly<{ stats: IChatUsageStat[]; className?: string }>) {
	const { t } = useTranslation("chat");
	const isMobile = useIsMobile();
	const [open, setOpen] = useState(false);
	const stats = useMemo(
		() =>
			rawStats
				.map(readUsageStat)
				.filter((stat): stat is IChatUsageStat => stat !== null),
		[rawStats],
	);
	const breakdown = useMemo(() => usageBreakdown(stats), [stats]);
	const efficiency = useMemo(() => usageEfficiency(stats), [stats]);
	const close = useCallback(() => setOpen(false), []);

	const chip = (
		<button
			type="button"
			aria-haspopup="dialog"
			aria-expanded={open}
			onClick={isMobile ? () => setOpen(true) : undefined}
			className={cn(
				"inline-flex h-7 items-center gap-1.5 rounded-lg border border-transparent px-2.5 text-xs text-muted-foreground tabular-nums transition-colors hover:border-border/50 hover:bg-muted/50 hover:text-foreground",
				open && "border-border/50 bg-muted/60 text-foreground",
				className,
			)}
		>
			<ZapIcon aria-hidden="true" className="size-3" />
			<span>
				{formatTokens(breakdown.totalTokens)}{" "}
				{t("usageEfficiency.tokenUnit", "tokens")}
			</span>
			{breakdown.cost != null && (
				<>
					<span aria-hidden="true" className="opacity-40">
						·
					</span>
					<span>{formatCost(breakdown.cost)}</span>
				</>
			)}
			{breakdown.durationMs != null && (
				<>
					<span aria-hidden="true" className="opacity-40">
						·
					</span>
					<span>{formatDuration(breakdown.durationMs)}</span>
				</>
			)}
			<ChevronUpIcon
				aria-hidden="true"
				className={cn(
					"size-3 opacity-50 transition-transform",
					open && "rotate-180",
				)}
			/>
		</button>
	);

	if (isMobile) {
		return (
			<>
				{chip}
				<Sheet open={open} onOpenChange={setOpen}>
					<SheetContent
						side="bottom"
						className={cn(
							ABOVE_PANELS,
							"h-[85dvh] gap-0 rounded-t-2xl p-0 pt-2",
						)}
						overlayClassName={ABOVE_PANELS}
					>
						<UsageReceipt
							stats={stats}
							breakdown={breakdown}
							efficiency={efficiency}
							title={
								<>
									<SheetTitle className="sr-only">
										{t("modelUsage", "Model Usage")}
									</SheetTitle>
									<SheetDescription className="sr-only">
										{t("usageReceipt.thisReply", "This reply")}
									</SheetDescription>
								</>
							}
						/>
					</SheetContent>
				</Sheet>
			</>
		);
	}

	// Modal so the receipt's own scroll areas keep wheel scrolling when the chat sits
	// inside a Radix dialog; outside clicks still just close it.
	return (
		<Popover open={open} onOpenChange={setOpen} modal>
			<PopoverTrigger asChild>{chip}</PopoverTrigger>
			<PopoverContent
				side="top"
				align="start"
				sideOffset={8}
				collisionPadding={12}
				aria-label={t("modelUsage", "Model Usage")}
				className={cn(
					ABOVE_PANELS,
					"flex h-[min(38rem,var(--radix-popover-content-available-height,38rem))] w-[min(30rem,calc(100vw-1.5rem))] flex-col overflow-hidden p-0 pt-1",
				)}
			>
				<UsageReceipt
					stats={stats}
					breakdown={breakdown}
					efficiency={efficiency}
					onClose={close}
				/>
			</PopoverContent>
		</Popover>
	);
}
