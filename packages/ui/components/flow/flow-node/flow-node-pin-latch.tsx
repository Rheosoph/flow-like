import { useTranslation } from "@flow-like/locales";
import { ChevronUpIcon } from "lucide-react";
import { memo } from "react";
import { cn } from "../../../lib/utils";

export const PIN_LATCH_HEIGHT = 10;

/**
 * Bottom strip that folds a node's unconnected data pins away. The chevron
 * points up while everything is shown (click to fold) and flips down once
 * pins are hidden, alongside the hidden count.
 */
export const FlowNodePinLatch = memo(function FlowNodePinLatch({
	collapsed,
	hiddenCount,
	onToggle,
}: Readonly<{
	collapsed: boolean;
	hiddenCount: number;
	onToggle: () => void;
}>) {
	const { t } = useTranslation("flow");
	const label = collapsed
		? t("showHiddenPins", "Show {{count}} hidden pins", { count: hiddenCount })
		: t("hideUnconnectedPins", "Hide unconnected pins");

	return (
		<button
			type="button"
			aria-expanded={!collapsed}
			aria-label={label}
			title={label}
			data-collapsed={collapsed ? "" : undefined}
			className={cn(
				"nodrag absolute inset-x-0 bottom-0 flex cursor-pointer items-center justify-center gap-0.5 rounded-md rounded-t-none border-t transition-colors",
				collapsed
					? "border-primary/30 bg-primary/10 text-primary hover:bg-primary/20"
					: "border-border/70 bg-muted/90 text-muted-foreground hover:bg-muted hover:text-foreground",
			)}
			style={{ height: PIN_LATCH_HEIGHT }}
			onClick={(event) => {
				event.stopPropagation();
				onToggle();
			}}
		>
			<ChevronUpIcon
				strokeWidth={3}
				className={cn(
					"size-2 transition-transform duration-200",
					collapsed && "rotate-180",
				)}
			/>
			{collapsed && (
				<span className="text-[7px] font-semibold leading-none tabular-nums">
					{hiddenCount}
				</span>
			)}
		</button>
	);
});
