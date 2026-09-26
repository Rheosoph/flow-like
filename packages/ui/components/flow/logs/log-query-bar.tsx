import { useTranslation } from "@flow-like/locales";
import { Command as CommandPrimitive } from "cmdk";
import {
	ArrowUpToLineIcon,
	EyeOffIcon,
	FilterIcon,
	SearchIcon,
	XIcon,
} from "lucide-react";
import {
	type KeyboardEvent,
	type ReactNode,
	type RefObject,
	memo,
	useCallback,
	useMemo,
	useState,
} from "react";
import type { INode } from "../../../lib/schema/flow/node";
import { cn } from "../../../lib/utils";
import {
	Command,
	CommandGroup,
	CommandItem,
	CommandList,
} from "../../ui/command";
import { Popover, PopoverAnchor, PopoverContent } from "../../ui/popover";
import {
	type ILogChip,
	type ILogFilter,
	chipId,
	invertLevels,
	levelChip,
	removeChip,
	setChipUpstream,
	setLevelsOff,
	toggleChip,
} from "./filter-model";
import { groupLabel } from "./fold-model";
import { LevelIcon } from "./level-style";
import { LEVEL_NAMES } from "./log-format";
import { NodeGlyph } from "./log-node-chip";
import { type INodeRef, parseQueryInput } from "./query-tokens";
import {
	type ISuggestion,
	type ISuggestionContext,
	applyParsed,
	applySuggestion,
	buildSuggestions,
} from "./suggestions";

const CHIP =
	"inline-flex h-5 shrink-0 items-center gap-0.5 rounded pl-1.5 pr-0.5 font-mono text-[11px] whitespace-nowrap";

const FilterChip = memo(function FilterChip({
	negated,
	label,
	value,
	onToggle,
	onRemove,
	extra,
}: Readonly<{
	negated: boolean;
	label: string;
	value: string;
	onToggle: () => void;
	onRemove: () => void;
	extra?: ReactNode;
}>) {
	const { t } = useTranslation("flow");
	return (
		<span
			className={cn(
				CHIP,
				negated
					? "border border-dashed border-destructive/45 bg-destructive/5 text-destructive"
					: "border bg-secondary text-foreground",
			)}
		>
			<button
				type="button"
				onClick={onToggle}
				title={
					negated
						? t("logViewChipInclude", "Click to include instead")
						: t("logViewChipExclude", "Click to exclude instead")
				}
				className="inline-flex min-w-0 max-w-64 items-center gap-0.5 rounded-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
			>
				{negated ? <span aria-hidden>−</span> : null}
				<span
					className={negated ? "text-destructive/80" : "text-muted-foreground"}
				>
					{label}
				</span>
				<span
					className={cn(
						"min-w-0 truncate",
						negated && "line-through decoration-destructive/70",
					)}
				>
					{value}
				</span>
			</button>
			{extra}
			<button
				type="button"
				onClick={onRemove}
				aria-label={t("logViewRemoveFilter", "Remove filter {{filter}}", {
					filter: `${negated ? "-" : ""}${label}${value}`,
				})}
				className="flex size-4 items-center justify-center rounded-sm opacity-75 hover:opacity-100 focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
			>
				<XIcon className="size-3" />
			</button>
		</span>
	);
});

function SuggestionRow({
	suggestion,
	node,
}: Readonly<{ suggestion: ISuggestion; node?: INode }>) {
	const { t } = useTranslation("flow");
	const minus = suggestion.negated ? "−" : "";
	switch (suggestion.kind) {
		case "text":
			return (
				<>
					{suggestion.negated ? <EyeOffIcon /> : <SearchIcon />}
					<span className="min-w-0 flex-1 truncate">
						{suggestion.negated
							? t(
									"logViewHideContaining",
									"Hide messages containing “{{text}}”",
									{
										text: suggestion.text,
									},
								)
							: t(
									"logViewOnlyContaining",
									"Only messages containing “{{text}}”",
									{
										text: suggestion.text,
									},
								)}
					</span>
				</>
			);
		case "node":
			return (
				<>
					<NodeGlyph node={node} />
					<span className="min-w-0 flex-1 truncate font-mono text-xs">
						<span className="text-muted-foreground">{minus}node:</span>
						{suggestion.name}
					</span>
					<span className="font-mono text-xs text-muted-foreground tabular-nums">
						{suggestion.count.toLocaleString()}
					</span>
				</>
			);
		case "level":
			return (
				<>
					<LevelIcon level={suggestion.level} />
					<span className="min-w-0 flex-1 truncate font-mono text-xs">
						<span className="text-muted-foreground">{minus}level:</span>
						{LEVEL_NAMES[suggestion.level]}
					</span>
					<span className="font-mono text-xs text-muted-foreground tabular-nums">
						{suggestion.count.toLocaleString()}
					</span>
				</>
			);
		case "group":
			return (
				<>
					<FilterIcon />
					<span className="min-w-0 flex-1 truncate font-mono text-xs">
						{minus}“{groupLabel(suggestion.template, 60)}”
					</span>
					<span className="shrink-0 text-xs text-muted-foreground">
						{suggestion.negated
							? t("logViewHidesCount", "hides {{count, number}}", {
									count: suggestion.count,
								})
							: t("logViewMatchesCount", "matches {{count, number}}", {
									count: suggestion.count,
								})}
					</span>
				</>
			);
	}
}

export interface ILogQueryBarProps {
	filter: ILogFilter;
	onFilterChange: (next: ILogFilter) => void;
	input: string;
	onInputChange: (value: string) => void;
	nodes: readonly INodeRef[];
	nodeOf: (nodeId: string) => INode | undefined;
	nodeName: (nodeId: string) => string;
	upstreamCount: (nodeId: string) => number;
	suggestionContext: ISuggestionContext;
	inputRef: RefObject<HTMLInputElement | null>;
}

export const LogQueryBar = memo(function LogQueryBar({
	filter,
	onFilterChange,
	input,
	onInputChange,
	nodes,
	nodeOf,
	nodeName,
	upstreamCount,
	suggestionContext,
	inputRef,
}: Readonly<ILogQueryBarProps>) {
	const { t } = useTranslation("flow");
	const [open, setOpen] = useState(false);
	const [active, setActive] = useState("s0");
	const [anchor, setAnchor] = useState<HTMLDivElement | null>(null);

	const suggestions = useMemo(
		() => buildSuggestions(input, suggestionContext),
		[input, suggestionContext],
	);

	const label = useCallback((template: string) => groupLabel(template), []);

	const commitInput = useCallback(() => {
		if (!input.trim()) return;
		onFilterChange(
			applyParsed(filter, parseQueryInput(input, nodes, { commit: true })),
		);
		onInputChange("");
	}, [filter, input, nodes, onFilterChange, onInputChange]);

	const pick = useCallback(
		(suggestion: ISuggestion) => {
			onFilterChange(applySuggestion(filter, input, suggestion, nodes, label));
			onInputChange("");
			setActive("s0");
			inputRef.current?.focus();
		},
		[filter, input, nodes, label, onFilterChange, onInputChange, inputRef],
	);

	const popLast = useCallback(() => {
		const last = filter.chips.at(-1);
		if (last) onFilterChange(removeChip(filter, chipId(last)));
		else if (filter.levelsOff.length > 0)
			onFilterChange(setLevelsOff(filter, []));
	}, [filter, onFilterChange]);

	const onKeyDown = useCallback(
		(event: KeyboardEvent<HTMLInputElement>) => {
			if (event.key === "Enter" && (!open || suggestions.length === 0)) {
				event.preventDefault();
				commitInput();
				return;
			}
			if (event.key === "Backspace" && input === "") {
				event.preventDefault();
				popLast();
				return;
			}
			if (event.key === "Home" || event.key === "End") {
				event.stopPropagation();
				return;
			}
			if (event.key === "Escape" && !open) {
				event.currentTarget.blur();
			}
		},
		[open, suggestions.length, commitInput, input, popLast],
	);

	const levels = levelChip(filter.levelsOff);

	return (
		<Popover open={open} onOpenChange={setOpen}>
			<Command
				shouldFilter={false}
				loop
				value={active}
				onValueChange={setActive}
				className="contents"
			>
				<PopoverAnchor asChild>
					<div
						ref={setAnchor}
						className="flex h-6.5 min-w-0 flex-1 items-center gap-1 overflow-hidden rounded-md border bg-card pr-1 pl-2 focus-within:border-ring/60"
					>
						<FilterIcon
							aria-hidden
							className="size-3.5 shrink-0 text-muted-foreground"
						/>
						{levels ? (
							<FilterChip
								negated={levels.negated}
								label="level:"
								value={levels.levels.map((l) => LEVEL_NAMES[l]).join(",")}
								onToggle={() => onFilterChange(invertLevels(filter))}
								onRemove={() => onFilterChange(setLevelsOff(filter, []))}
							/>
						) : null}
						{filter.chips.map((chip) => (
							<QueryChip
								key={chipId(chip)}
								chip={chip}
								filter={filter}
								onFilterChange={onFilterChange}
								nodeName={nodeName}
								upstreamCount={upstreamCount}
							/>
						))}
						<div className="flex min-w-20 flex-1">
							<CommandPrimitive.Input
								ref={inputRef}
								aria-label={t("logViewFilterLogs", "Filter logs")}
								value={input}
								onValueChange={(value) => {
									onInputChange(value);
									setActive("s0");
									setOpen(true);
								}}
								onFocus={() => setOpen(true)}
								onKeyDown={onKeyDown}
								placeholder={
									filter.chips.length || levels
										? t("logViewAddFilter", "Add filter…")
										: t(
												"logViewQueryPlaceholder",
												'node:  level:  -exclude  "phrase"',
											)
								}
								className="h-5.5 min-w-0 flex-1 bg-transparent px-1 font-mono text-xs outline-none placeholder:text-muted-foreground/70"
							/>
						</div>
						<kbd className="hidden h-4 shrink-0 items-center rounded border px-1 font-mono text-[10px] text-muted-foreground sm:inline-flex">
							/
						</kbd>
					</div>
				</PopoverAnchor>
				<PopoverContent
					align="start"
					sideOffset={4}
					onOpenAutoFocus={(event) => event.preventDefault()}
					onInteractOutside={(event) => {
						if (anchor?.contains(event.target as Node)) event.preventDefault();
					}}
					className="w-[min(420px,var(--radix-popover-trigger-width))] min-w-72 p-0"
				>
					{suggestions.length > 0 ? (
						<CommandList className="max-h-72">
							<CommandGroup>
								{suggestions.map((suggestion, i) => (
									<CommandItem
										key={suggestion.id}
										value={`s${i}`}
										onSelect={() => pick(suggestion)}
										className="h-7 py-1"
									>
										<SuggestionRow
											suggestion={suggestion}
											node={
												suggestion.kind === "node"
													? nodeOf(suggestion.nodeId)
													: undefined
											}
										/>
									</CommandItem>
								))}
							</CommandGroup>
						</CommandList>
					) : null}
					<div className="flex items-center gap-3 border-t px-2.5 py-1.5 font-mono text-[10.5px] text-muted-foreground">
						<span>node:</span>
						<span>level:</span>
						<span>-{t("logViewSyntaxNegate", "negate")}</span>
						<span>"{t("logViewSyntaxPhrase", "phrase")}"</span>
						<span className="flex-1" />
						<span>↑↓ ↵ ⌫</span>
					</div>
				</PopoverContent>
			</Command>
		</Popover>
	);
});

const QueryChip = memo(function QueryChip({
	chip,
	filter,
	onFilterChange,
	nodeName,
	upstreamCount,
}: Readonly<{
	chip: ILogChip;
	filter: ILogFilter;
	onFilterChange: (next: ILogFilter) => void;
	nodeName: (nodeId: string) => string;
	upstreamCount: (nodeId: string) => number;
}>) {
	const { t } = useTranslation("flow");
	const id = chipId(chip);
	const onToggle = useCallback(
		() => onFilterChange(toggleChip(filter, id)),
		[filter, id, onFilterChange],
	);
	const onRemove = useCallback(
		() => onFilterChange(removeChip(filter, id)),
		[filter, id, onFilterChange],
	);

	if (chip.kind === "node") {
		const name = nodeName(chip.value);
		const upstream = chip.upstream ? upstreamCount(chip.value) : 0;
		const value = chip.upstream
			? t("logViewNodeWithUpstream", "{{name}} + {{count, number}} upstream", {
					name,
					count: upstream,
				})
			: name;
		const upstreamLabel = chip.upstream
			? t("logViewDropUpstream", "Only {{name}}", { name })
			: t(
					"logViewAddUpstream",
					"+ upstream: add the nodes that feed {{name}}",
					{ name },
				);
		const extra = chip.negated ? null : (
			<button
				type="button"
				aria-pressed={!!chip.upstream}
				aria-label={upstreamLabel}
				title={upstreamLabel}
				onClick={() =>
					onFilterChange(setChipUpstream(filter, id, !chip.upstream))
				}
				className={cn(
					"ml-0.5 flex h-4 items-center gap-0.5 rounded-sm px-0.5 text-[10px] focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring",
					chip.upstream
						? "bg-primary/15 text-primary"
						: "text-muted-foreground hover:text-foreground",
				)}
			>
				<ArrowUpToLineIcon className="size-3" />
			</button>
		);
		return (
			<FilterChip
				negated={chip.negated}
				label="node:"
				value={value}
				onToggle={onToggle}
				onRemove={onRemove}
				extra={extra}
			/>
		);
	}
	if (chip.kind === "group") {
		return (
			<FilterChip
				negated={chip.negated}
				label="like:"
				value={`“${chip.label}”`}
				onToggle={onToggle}
				onRemove={onRemove}
			/>
		);
	}
	return (
		<FilterChip
			negated={chip.negated}
			label=""
			value={`"${chip.value}"`}
			onToggle={onToggle}
			onRemove={onRemove}
		/>
	);
});
