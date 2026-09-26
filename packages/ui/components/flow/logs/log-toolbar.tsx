import { useTranslation } from "@flow-like/locales";
import { ListCollapseIcon, type LucideIcon, TimerIcon } from "lucide-react";
import { type MouseEvent, memo } from "react";
import { cn } from "../../../lib/utils";
import { LEVEL_SHORT, LEVEL_TONES } from "./level-style";
import { ALL_LEVELS } from "./log-format";
import { type ILogQueryBarProps, LogQueryBar } from "./log-query-bar";

export const LevelControl = memo(function LevelControl({
	levelsOff,
	counts,
	onToggle,
	onOnly,
}: Readonly<{
	levelsOff: readonly number[];
	counts?: readonly number[];
	onToggle: (level: number) => void;
	onOnly: (level: number) => void;
}>) {
	const { t } = useTranslation("flow");
	const names = [
		t("logViewLevelDebug", "Debug"),
		t("logViewLevelInfo", "Info"),
		t("logViewLevelWarn", "Warn"),
		t("logViewLevelError", "Error"),
		t("logViewLevelFatal", "Fatal"),
	];
	return (
		<fieldset
			aria-label={t("logViewLogLevels", "Log levels")}
			className="flex h-6.5 min-w-0 shrink-0 overflow-hidden rounded-md border"
		>
			{ALL_LEVELS.map((level) => {
				const on = !levelsOff.includes(level);
				const count = counts?.[level];
				const name = names[level];
				return (
					<button
						key={level}
						type="button"
						aria-pressed={on}
						aria-label={
							count === undefined
								? name
								: t("logViewLevelCount", "{{level}}, {{count, number}} logs", {
										level: name,
										count,
									})
						}
						title={t(
							"logViewLevelHint",
							"Click to hide or show · Alt-click to show only this level",
						)}
						onClick={(event: MouseEvent<HTMLButtonElement>) =>
							event.altKey ? onOnly(level) : onToggle(level)
						}
						className={cn(
							"flex h-full items-center gap-1 px-1.75 text-[11px] transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-inset focus-visible:ring-ring",
							level > 0 && "border-l",
							on ? "bg-secondary/60" : "bg-transparent hover:bg-secondary/40",
						)}
					>
						<span
							className={cn(
								"font-bold",
								on
									? LEVEL_TONES[level]
									: "text-muted-foreground/50 line-through",
							)}
						>
							{LEVEL_SHORT[level]}
						</span>
						{count !== undefined ? (
							<span
								className={cn(
									"font-mono tabular-nums",
									on
										? "text-foreground/80"
										: "text-muted-foreground/50 line-through",
								)}
							>
								{count.toLocaleString()}
							</span>
						) : null}
					</button>
				);
			})}
		</fieldset>
	);
});

export const ToolbarToggle = memo(function ToolbarToggle({
	pressed,
	onPressedChange,
	icon: Icon,
	label,
	title,
}: Readonly<{
	pressed: boolean;
	onPressedChange: (pressed: boolean) => void;
	icon: LucideIcon;
	label: string;
	title?: string;
}>) {
	return (
		<button
			type="button"
			aria-pressed={pressed}
			title={title}
			onClick={() => onPressedChange(!pressed)}
			className={cn(
				"flex h-6.5 shrink-0 items-center gap-1.5 rounded-md border px-2 text-xs font-medium whitespace-nowrap transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring",
				pressed
					? "border-primary/45 bg-primary/10 text-foreground"
					: "text-muted-foreground hover:bg-secondary hover:text-foreground",
			)}
		>
			<Icon
				aria-hidden
				className={cn(
					"size-3.5",
					pressed ? "text-primary" : "text-muted-foreground",
				)}
			/>
			<span className="hidden md:inline">{label}</span>
		</button>
	);
});

export const LogToolbar = memo(function LogToolbar({
	queryBar,
	levelsOff,
	levelCounts,
	onToggleLevel,
	onOnlyLevel,
	foldAvailable,
	fold,
	onFoldChange,
	relative,
	onRelativeChange,
}: Readonly<{
	queryBar: ILogQueryBarProps;
	levelsOff: readonly number[];
	levelCounts?: readonly number[];
	onToggleLevel: (level: number) => void;
	onOnlyLevel: (level: number) => void;
	foldAvailable: boolean;
	fold: boolean;
	onFoldChange: (fold: boolean) => void;
	relative: boolean;
	onRelativeChange: (relative: boolean) => void;
}>) {
	const { t } = useTranslation("flow");
	return (
		<div
			role="toolbar"
			aria-label={t("logViewLogFilters", "Log filters")}
			className="flex h-9 shrink-0 items-center gap-1.5 border-b px-2"
		>
			<LogQueryBar {...queryBar} />
			<span aria-hidden className="h-4 w-px shrink-0 bg-border" />
			{foldAvailable ? (
				<ToolbarToggle
					pressed={fold}
					onPressedChange={onFoldChange}
					icon={ListCollapseIcon}
					label={t("logViewFoldRepeats", "Fold repeats")}
					title={t(
						"logViewFoldRepeatsHint",
						"Show each repeated message once, with a count",
					)}
				/>
			) : null}
			<ToolbarToggle
				pressed={relative}
				onPressedChange={onRelativeChange}
				icon={TimerIcon}
				label={t("logViewRelativeTime", "Relative")}
				title={t("logViewRelativeTimeHint", "Show time since the run started")}
			/>
			<span aria-hidden className="h-4 w-px shrink-0 bg-border" />
			<LevelControl
				levelsOff={levelsOff}
				counts={levelCounts}
				onToggle={onToggleLevel}
				onOnly={onOnlyLevel}
			/>
		</div>
	);
});
