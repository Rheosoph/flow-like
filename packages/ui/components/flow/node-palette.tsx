"use client";

import { Trans, useTranslation } from "@flow-like/locales";
import {
	ChevronDownIcon,
	ChevronRightIcon,
	FileCode2Icon,
	FileIcon,
	FolderIcon,
	FolderInputIcon,
	ListIcon,
	LocateFixedIcon,
	type LucideIcon,
	MessageCircleDashedIcon,
	MessageCircleIcon,
	PlayCircleIcon,
	SearchIcon,
	VariableIcon,
	WorkflowIcon,
	XIcon,
	ZapIcon,
} from "lucide-react";
import {
	type KeyboardEvent,
	type ReactNode,
	memo,
	useCallback,
	useEffect,
	useMemo,
	useRef,
	useState,
} from "react";
import { type IBoardModule, MAIN_FILE_LABEL } from "../../lib/flow-modules";
import type { INode } from "../../lib/schema/flow/node";
import {
	type IPin,
	IPinType,
	IValueType,
	IVariableType,
} from "../../lib/schema/flow/pin";
import { cn } from "../../lib/utils";
import { useNodePaletteStore } from "../../state/node-palette-state";
import {
	DropdownMenu,
	DropdownMenuContent,
	DropdownMenuItem,
	DropdownMenuLabel,
	DropdownMenuTrigger,
} from "../ui/dropdown-menu";
import { DynamicImage } from "../ui/dynamic-image";
import { PopoverContent } from "../ui/popover";
import {
	type PaletteCategory,
	type PaletteDropIndex,
	type PaletteEntry,
	type PaletteGroup,
	type PaletteModel,
	type PaletteRow,
	type PaletteSelectableRow,
	buildPaletteRows,
	filterPaletteTree,
	findCategory,
	inScope,
	isFunctionReferencePin,
	isSelectableRow,
	matchesWords,
	pinSignature,
	pinTypeLabel,
	resolveRecents,
	searchPalette,
	sortedPins,
	tokenizeWithOperators,
} from "./node-palette-model";
import { typeToColor } from "./utils";

type PaletteActionId =
	| "comment"
	| "event"
	| "placeholder"
	| "ping"
	| "flowscript"
	| "chat"
	| "move"
	| "variable";

type PaletteReferent = "here" | "selection" | "pin";

export interface NodePaletteHandlers {
	placeEntry: (entry: PaletteEntry) => void;
	placeComment: () => void;
	placePlaceholder: (name: string) => void;
	placeEvent?: () => void;
	ping?: () => void;
	editAsFlowScript?: () => void;
	discussInChat?: () => void;
	moveToModule?: (target: string | null) => void;
	createVariable?: () => void;
}

interface NodePaletteDrop {
	pin: IPin;
	fits: PaletteDropIndex;
}

const pinDisplayName = (pin: Pick<IPin, "friendly_name" | "name">) =>
	pin.friendly_name || pin.name;

type Translate = ReturnType<typeof useTranslation>["t"];

function countLabel(t: Translate, count: number, compatible: boolean) {
	return compatible
		? t("paletteCompatibleCount", "{{count, number}} compatible", {
				count,
				ns: "flow",
			})
		: t("paletteNodeCount", "{{count, number}} nodes", { count, ns: "flow" });
}

function useFormatCount() {
	const { i18n } = useTranslation();
	return useMemo(() => {
		const format = new Intl.NumberFormat(i18n.language);
		return (value: number) => format.format(value);
	}, [i18n.language]);
}

interface PaletteAction {
	id: PaletteActionId;
	referent: PaletteReferent;
	icon: LucideIcon;
	label: string;
	title: string;
	keywords: string;
	description: string;
	run?: () => void;
	live?: boolean;
}

interface MoveTarget {
	id: string | null;
	label: string;
}

const QUICK_GROUPS: { referent: PaletteReferent; ids: PaletteActionId[] }[] = [
	{ referent: "here", ids: ["comment", "event", "placeholder", "ping"] },
	{ referent: "selection", ids: ["flowscript", "chat", "move"] },
	{ referent: "pin", ids: ["variable"] },
];

const PREVIEW_PIN_ROWS = 7;
const PAGE_STEP = 8;
const PEEK_DELAY_MS = 140;

function usePaletteActions(
	handlers: NodePaletteHandlers,
	droppedPin: IPin | undefined,
): PaletteAction[] {
	const { t } = useTranslation("flow");
	return useMemo(() => {
		const pinName = droppedPin ? pinDisplayName(droppedPin) : "";
		const candidates: (PaletteAction | false | undefined)[] = [
			{
				id: "comment",
				referent: "here",
				icon: MessageCircleDashedIcon,
				label: t("comment", "Comment"),
				title: t("comment", "Comment"),
				keywords: t("paletteCommentKeywords", "note frame annotate group"),
				description: t(
					"paletteCommentDescription",
					"Places a comment frame here to label and group the nodes around it.",
				),
				run: handlers.placeComment,
			},
			handlers.placeEvent && {
				id: "event",
				referent: "here",
				icon: PlayCircleIcon,
				label: t("event", "Event"),
				title: t("event", "Event"),
				keywords: t("paletteEventKeywords", "start trigger entry simple"),
				description: t(
					"paletteEventDescription",
					"Places a Simple Event: a start node with one execution output.",
				),
				run: handlers.placeEvent,
			},
			{
				id: "placeholder",
				referent: "here",
				icon: ZapIcon,
				label: t("placeholder", "Placeholder"),
				title: t("placeholder", "Placeholder"),
				keywords: t("palettePlaceholderKeywords", "stub temporary todo"),
				description: t(
					"palettePlaceholderDescription",
					"Places a named stand-in you can swap for a real node later.",
				),
			},
			handlers.ping && {
				id: "ping",
				referent: "here",
				icon: LocateFixedIcon,
				label: t("palettePing", "Ping"),
				title: t("pingHere", "Ping here for teammates"),
				keywords: t("palettePingKeywords", "teammates locate attention"),
				description: t(
					"palettePingDescription",
					"Shows teammates on this board a short-lived marker at this point.",
				),
				run: handlers.ping,
				live: true,
			},
			handlers.editAsFlowScript && {
				id: "flowscript",
				referent: "selection",
				icon: FileCode2Icon,
				label: t("paletteFlowScript", "FlowScript"),
				title: t("editSelectionAsFlowscript", "Edit selection as FlowScript"),
				keywords: t("paletteFlowScriptKeywords", "script code edit"),
				description: t(
					"paletteFlowScriptDescription",
					"Opens the FlowScript editor with only the selection in scope.",
				),
				run: handlers.editAsFlowScript,
			},
			handlers.discussInChat && {
				id: "chat",
				referent: "selection",
				icon: MessageCircleIcon,
				label: t("paletteChat", "Chat"),
				title: t("discussInChat", "Discuss in chat"),
				keywords: t("paletteChatKeywords", "discuss talk conversation"),
				description: t(
					"paletteChatDescription",
					"Opens the board chat with a reference to the selected node.",
				),
				run: handlers.discussInChat,
			},
			handlers.moveToModule && {
				id: "move",
				referent: "selection",
				icon: FolderInputIcon,
				label: t("paletteMove", "Move"),
				title: t("moveToModule", "Move to module"),
				keywords: t("paletteMoveKeywords", "module file relocate"),
				description: t(
					"paletteMoveDescription",
					"Moves the selection into another module of this board.",
				),
			},
			handlers.createVariable && {
				id: "variable",
				referent: "pin",
				icon: VariableIcon,
				label: t("paletteVariable", "Variable from pin"),
				title: t("paletteCreateVariable", "Create variable from pin"),
				keywords: t("paletteVariableKeywords", "store save variable"),
				description: t(
					"paletteVariableDescription",
					"Creates a variable from {{pin}} and places a node wired to it.",
					{ pin: pinName },
				),
				run: handlers.createVariable,
			},
		];
		return candidates.filter((action): action is PaletteAction =>
			Boolean(action),
		);
	}, [handlers, droppedPin, t]);
}

const isComposing = (event: KeyboardEvent | globalThis.KeyboardEvent) => {
	const native = "nativeEvent" in event ? event.nativeEvent : event;
	return native.isComposing || native.keyCode === 229;
};

export function NodePalette({
	open,
	model,
	drop,
	selectionCount,
	modules,
	currentModuleId,
	handlers,
}: Readonly<{
	/** False while the palette animates out: it keeps rendering but takes no more input. */
	open: boolean;
	model: PaletteModel;
	drop?: NodePaletteDrop;
	selectionCount: number;
	modules: IBoardModule[];
	currentModuleId: string | null;
	handlers: NodePaletteHandlers;
}>) {
	const { t } = useTranslation("flow");
	const formatCount = useFormatCount();
	// Read once per session: a placement reorders recents while the palette fades out.
	const [recents] = useState(() => useNodePaletteStore.getState().recents);
	const inputRef = useRef<HTMLInputElement>(null);
	const listRef = useRef<HTMLDivElement>(null);
	const armedRef = useRef(false);
	const pointerRef = useRef("");
	const scrollToHighlightRef = useRef(false);
	const peekTimerRef = useRef<ReturnType<typeof setTimeout> | undefined>(
		undefined,
	);

	const [query, setQuery] = useState("");
	const [scope, setScope] = useState<string[]>([]);
	const [slide, setSlide] = useState<"in" | "out" | null>(null);
	const [compatibleOnly, setCompatibleOnly] = useState(true);
	const [naming, setNaming] = useState<string | null>(null);
	const [namingInvalid, setNamingInvalid] = useState(false);
	const [highlightId, setHighlightId] = useState<string | null>(null);
	const [peek, setPeek] = useState<PaletteActionId | null>(null);
	const [moveOpen, setMoveOpen] = useState(false);

	const actions = usePaletteActions(handlers, drop?.pin);
	const actionById = useMemo(
		() => new Map(actions.map((action) => [action.id, action])),
		[actions],
	);
	const moveTargets = useMemo<MoveTarget[]>(
		() => [
			{ id: null, label: MAIN_FILE_LABEL },
			...modules.map((module) => ({ id: module.id, label: module.pathLabel })),
		],
		[modules],
	);

	const accepts = useMemo(
		() =>
			drop && compatibleOnly
				? (entry: PaletteEntry) => drop.fits.has(entry.key)
				: undefined,
		[drop, compatibleOnly],
	);
	const tree = useMemo(
		() => (accepts ? filterPaletteTree(model, accepts) : model.tree),
		[model, accepts],
	);
	const trimmed = query.trim();
	const results = useMemo(
		() =>
			trimmed ? searchPalette(model, trimmed, { scope, recents, accepts }) : [],
		[model, trimmed, scope, recents, accepts],
	);
	const canWiden = useMemo(
		() =>
			Boolean(accepts) &&
			trimmed !== "" &&
			results.length === 0 &&
			searchPalette(model, trimmed, { scope, recents }).length > 0,
		[accepts, trimmed, results, model, scope, recents],
	);
	const matchedActions = useMemo(
		() =>
			trimmed && scope.length === 0
				? actions
						.filter((action) =>
							matchesWords(
								trimmed,
								action.title,
								`${action.label} ${action.keywords}`,
							),
						)
						.map((action) => action.id)
				: [],
		[actions, trimmed, scope],
	);
	const recentEntries = useMemo(
		() => resolveRecents(model, recents, accepts),
		[model, recents, accepts],
	);
	const rows = useMemo(
		() =>
			buildPaletteRows({
				query,
				scope,
				tree,
				recents: recentEntries,
				results,
				actions: matchedActions,
				dropActive: Boolean(accepts),
				canWiden,
			}),
		[
			query,
			scope,
			tree,
			recentEntries,
			results,
			matchedActions,
			accepts,
			canWiden,
		],
	);
	const selectable = useMemo(() => rows.filter(isSelectableRow), [rows]);
	const highlighted =
		selectable.find((row) => row.id === highlightId) ?? selectable[0];

	useEffect(() => {
		if (!scrollToHighlightRef.current || !highlighted) return;
		scrollToHighlightRef.current = false;
		document
			.getElementById(rowDomId(highlighted.id))
			?.scrollIntoView({ block: "nearest" });
	}, [highlighted]);

	useEffect(() => () => clearTimeout(peekTimerRef.current), []);

	const focusInput = useCallback(
		() => inputRef.current?.focus({ preventScroll: true }),
		[],
	);

	const changeQuery = useCallback((next: string) => {
		setQuery(next);
		setHighlightId(null);
		if (listRef.current) listRef.current.scrollTop = 0;
	}, []);

	const changeScope = useCallback(
		(next: string[], from?: string) => {
			setSlide(next.length > scope.length ? "in" : "out");
			setScope(next);
			setQuery("");
			setHighlightId(from ? `category:${from}` : null);
			scrollToHighlightRef.current = Boolean(from);
			if (listRef.current && !from) listRef.current.scrollTop = 0;
		},
		[scope],
	);

	const goUp = useCallback(() => {
		if (scope.length === 0) return;
		changeScope(scope.slice(0, -1), scope.join("/"));
	}, [scope, changeScope]);

	const startNaming = useCallback(() => {
		setNaming(t("placeholder", "Placeholder"));
		setNamingInvalid(false);
		setPeek(null);
		requestAnimationFrame(() => inputRef.current?.select());
	}, [t]);

	const commitNaming = useCallback(() => {
		if (!open) return;
		const name = naming?.trim();
		if (!name) {
			setNamingInvalid(true);
			return;
		}
		handlers.placePlaceholder(name);
	}, [open, naming, handlers]);

	const runAction = useCallback(
		(id: PaletteActionId) => {
			if (!open) return;
			if (id === "placeholder") return startNaming();
			if (id === "move") return setMoveOpen(true);
			actionById.get(id)?.run?.();
		},
		[open, actionById, startNaming],
	);

	const endNaming = useCallback(() => {
		setNaming(null);
		setNamingInvalid(false);
		inputRef.current?.focus({ preventScroll: true });
	}, []);

	const placeEntry = useCallback(
		(entry: PaletteEntry) => {
			if (!open) return;
			useNodePaletteStore.getState().recordPlacement(entry.key);
			handlers.placeEntry(entry);
		},
		[open, handlers],
	);

	const activate = useCallback(
		(row: PaletteSelectableRow) => {
			if (!open) return;
			switch (row.kind) {
				case "node":
					placeEntry(row.entry);
					return;
				case "category":
					changeScope(row.category.path);
					return;
				case "action":
					runAction(row.action as PaletteActionId);
					return;
				case "widen":
					if (row.widen === "scope") changeScope([]);
					else setCompatibleOnly(false);
					focusInput();
			}
		},
		[open, placeEntry, changeScope, runAction, focusInput],
	);

	const moveHighlight = useCallback(
		(delta: number, clampToEnds = false) => {
			if (selectable.length === 0) return;
			const current = highlighted ? selectable.indexOf(highlighted) : -1;
			const next = clampToEnds
				? Math.max(0, Math.min(selectable.length - 1, current + delta))
				: (current + delta + selectable.length) % selectable.length;
			setPeek(null);
			if (next === current) return;
			scrollToHighlightRef.current = true;
			setHighlightId(selectable[next].id);
		},
		[selectable, highlighted],
	);

	/** Esc leaves naming, then the open category, before it closes the palette. */
	const stepBack = useCallback(() => {
		if (naming !== null) {
			endNaming();
			return true;
		}
		if (scope.length > 0) {
			goUp();
			return true;
		}
		return false;
	}, [naming, scope, goUp, endNaming]);

	const onInputKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
		if (!open || isComposing(event)) return;
		if (naming !== null) {
			if (event.key === "Enter") {
				event.preventDefault();
				commitNaming();
			}
			return;
		}
		const input = event.currentTarget;
		const caretAtEnd = input.selectionStart === input.value.length;
		switch (event.key) {
			case "ArrowDown":
			case "ArrowUp":
				event.preventDefault();
				moveHighlight(event.key === "ArrowDown" ? 1 : -1);
				return;
			case "PageDown":
			case "PageUp":
				event.preventDefault();
				moveHighlight(event.key === "PageDown" ? PAGE_STEP : -PAGE_STEP, true);
				return;
			case "Enter":
				event.preventDefault();
				if (highlighted) activate(highlighted);
				return;
			case "ArrowRight":
				if (
					caretAtEnd &&
					highlighted &&
					(highlighted.kind === "category" ||
						(highlighted.kind === "action" && highlighted.action === "move"))
				) {
					event.preventDefault();
					activate(highlighted);
				}
				return;
			case "ArrowLeft":
			case "Backspace":
				// A held Backspace that just cleared the query must not keep climbing levels.
				if (event.key === "Backspace" && event.repeat) return;
				if (!input.value && scope.length > 0) {
					event.preventDefault();
					goUp();
				}
		}
	};

	const onListPointerMove = (event: React.PointerEvent<HTMLDivElement>) => {
		const position = `${event.clientX},${event.clientY}`;
		if (position === pointerRef.current) return;
		pointerRef.current = position;
		scrollToHighlightRef.current = false;
		const target = (event.target as HTMLElement).closest<HTMLElement>(
			"[data-row-id]",
		);
		const id = target?.dataset.rowId;
		if (id && id !== highlighted?.id) {
			setHighlightId(id);
			setPeek(null);
		}
	};

	const onListClick = (event: React.MouseEvent<HTMLDivElement>) => {
		if (!armedRef.current) return;
		const target = event.target as HTMLElement;
		const id = target.closest<HTMLElement>("[data-row-id]")?.dataset.rowId;
		const row = selectable.find((candidate) => candidate.id === id);
		if (row) activate(row);
	};

	const peekAction = (id: PaletteActionId | null, immediate = false) => {
		clearTimeout(peekTimerRef.current);
		if (immediate || id === null) {
			setPeek(id);
			return;
		}
		peekTimerRef.current = setTimeout(() => setPeek(id), PEEK_DELAY_MS);
	};

	const dropPinName = drop ? pinDisplayName(drop.pin) : "";
	const selectionLabel = t("paletteSelected", "{{count, number}} selected", {
		count: selectionCount,
	});
	const currentCategory = scope.length > 0 ? findCategory(tree, scope) : tree;

	return (
		<PopoverContent
			side="bottom"
			align="start"
			sideOffset={4}
			collisionPadding={16}
			aria-label={t("paletteDialog", "Add to board")}
			// Closing unmounts at once: a fading modal layer would keep the canvas unclickable
			// and its chips live for the length of the animation.
			className="grid w-[min(600px,calc(100vw-2rem))] grid-rows-[auto_auto_minmax(0,1fr)_auto] overflow-hidden p-0 data-[state=closed]:animate-none"
			style={{
				height:
					"min(440px, var(--radix-popover-content-available-height, 440px))",
			}}
			onOpenAutoFocus={(event) => {
				event.preventDefault();
				focusInput();
			}}
			onCloseAutoFocus={(event) => event.preventDefault()}
			onEscapeKeyDown={(event) => {
				if (isComposing(event) || stepBack()) event.preventDefault();
			}}
			onPointerDownCapture={() => {
				armedRef.current = true;
			}}
			// Clicks anywhere in the palette leave focus in the search field. A focused chip,
			// crumb or row that re-renders away would drop focus to <body>, where Backspace
			// reaches the board's delete shortcut.
			onMouseDown={(event) => {
				if (event.target !== inputRef.current) event.preventDefault();
			}}
			onKeyDown={(event) => event.stopPropagation()}
			onContextMenu={(event) => {
				if (event.target !== inputRef.current) event.preventDefault();
			}}
		>
			<div className="flex min-h-12 items-center gap-2 border-b py-1 pr-2 pl-3">
				<SearchIcon className="size-4 shrink-0 text-muted-foreground" />
				<div className="flex min-w-0 flex-1 items-center gap-1.5">
					{naming !== null ? (
						<ScopeChip
							icon={ZapIcon}
							segments={[t("paletteNamePlaceholder", "Name placeholder")]}
							clearLabel={t("paletteCancelPlaceholder", "Cancel placeholder")}
							onClear={endNaming}
						/>
					) : (
						scope.length > 0 && (
							<ScopeChip
								segments={scope}
								clearLabel={t("paletteClearScope", "Search all categories")}
								onClear={() => {
									changeScope([]);
									focusInput();
								}}
							/>
						)
					)}
					<input
						ref={inputRef}
						id="node-palette-search"
						type="text"
						role="combobox"
						aria-expanded={naming === null}
						aria-controls={naming === null ? "node-palette-list" : undefined}
						aria-autocomplete="list"
						aria-activedescendant={
							naming === null && highlighted
								? rowDomId(highlighted.id)
								: undefined
						}
						aria-invalid={(naming !== null && namingInvalid) || undefined}
						aria-describedby={naming !== null ? NAMING_HINT_ID : undefined}
						autoComplete="off"
						autoCorrect="off"
						autoCapitalize="off"
						spellCheck={false}
						className="h-10 min-w-16 flex-1 bg-transparent text-[15px] outline-hidden placeholder:text-muted-foreground"
						placeholder={
							naming !== null
								? t("egTemporaryResult", "e.g. Temporary Result")
								: scope.length > 0
									? t("paletteSearchIn", "Search in {{category}}", {
											category: scope[scope.length - 1],
										})
									: t("paletteSearch", "Search nodes and actions")
						}
						value={naming ?? query}
						onChange={(event) => {
							if (naming !== null) {
								setNaming(event.target.value);
								setNamingInvalid(false);
							} else changeQuery(event.target.value);
						}}
						onKeyDown={onInputKeyDown}
					/>
				</div>
				{drop && (
					<DropChip
						pin={drop.pin}
						active={compatibleOnly}
						onToggle={() => {
							setCompatibleOnly((value) => !value);
							setHighlightId(null);
							focusInput();
						}}
					/>
				)}
			</div>

			<div className="flex flex-wrap items-center justify-between gap-x-3.5 gap-y-px border-b px-2 py-1">
				{QUICK_GROUPS.map(({ referent, ids }) => {
					const group = ids.flatMap((id) => actionById.get(id) ?? []);
					if (group.length === 0) return null;
					return (
						<div
							key={referent}
							className="flex min-w-0 flex-wrap items-center gap-px"
						>
							<span
								className={cn(
									"pr-1.5 pl-1 text-[10.5px] text-muted-foreground whitespace-nowrap",
									referent === "selection"
										? "font-mono"
										: "font-medium uppercase tracking-wider",
								)}
							>
								{referentLabel(referent, t, selectionLabel)}
							</span>
							{group.map((action) =>
								action.id === "move" ? (
									<MoveMenu
										key={action.id}
										action={action}
										open={moveOpen}
										targets={moveTargets}
										currentModuleId={currentModuleId}
										onOpenChange={(next) => {
											if (open || !next) setMoveOpen(next);
										}}
										onMove={(target) => {
											if (open) handlers.moveToModule?.(target);
										}}
										onReturnFocus={focusInput}
										onPeek={peekAction}
									/>
								) : (
									<QuickAction
										key={action.id}
										action={action}
										active={action.id === "placeholder" && naming !== null}
										onRun={() => runAction(action.id)}
										onPeek={peekAction}
									/>
								),
							)}
						</div>
					);
				})}
			</div>

			<div className="grid min-h-0 grid-cols-[minmax(0,1fr)_280px] max-sm:grid-cols-1">
				<div className="flex min-h-0 min-w-0 flex-col">
					{scope.length > 0 && naming === null && (
						<Breadcrumbs
							scope={scope}
							onNavigate={(path) => {
								changeScope(path);
								focusInput();
							}}
						/>
					)}
					{naming !== null ? (
						<NamingRows
							name={naming}
							invalid={namingInvalid}
							onPlace={commitNaming}
						/>
					) : (
						// biome-ignore lint/a11y/useKeyWithClickEvents: keys reach the options through the combobox input's aria-activedescendant; this click only delegates to the option under the pointer.
						<div
							ref={listRef}
							id="node-palette-list"
							// biome-ignore lint/a11y/useSemanticElements: the popup owned by the search combobox; a select cannot hold rich option rows.
							role="listbox"
							tabIndex={-1}
							aria-label={t("paletteResults", "Nodes and actions")}
							className="min-h-0 flex-1 overflow-y-auto overscroll-contain px-1.5 pt-0.5 pb-2 outline-hidden"
							onPointerMove={onListPointerMove}
							onClick={onListClick}
						>
							<div
								key={scope.join("/")}
								className={cn(
									"motion-safe:animate-in motion-safe:fade-in-0 motion-safe:duration-150",
									slide === "in" && "motion-safe:slide-in-from-right-4",
									slide === "out" && "motion-safe:slide-in-from-left-4",
								)}
							>
								{groupRows(rows).map(({ header, rows: members }) => {
									const views = members.map((row) => (
										<PaletteRowView
											key={row.id}
											row={row}
											highlighted={row.id === highlighted?.id}
											query={trimmed}
											drop={drop}
											actionById={actionById}
											scopeName={scope[scope.length - 1]}
											dropPinName={dropPinName}
											filtering={Boolean(accepts)}
											formatCount={formatCount}
										/>
									));
									if (!header) return views;
									return (
										<div
											key={header.id}
											// biome-ignore lint/a11y/useSemanticElements: a listbox may only own options and groups of options; a fieldset is not allowed there.
											role="group"
											aria-labelledby={rowDomId(header.id)}
										>
											<GroupHeader
												row={header}
												scopeName={scope[scope.length - 1]}
												formatCount={formatCount}
											/>
											{views}
										</div>
									);
								})}
							</div>
						</div>
					)}
				</div>
				<div className="min-h-0 overflow-y-auto overscroll-contain border-l px-3.5 pt-3 pb-4 max-sm:hidden">
					<PaletteDetail
						row={naming === null ? highlighted : undefined}
						action={
							peek
								? actionById.get(peek)
								: highlighted?.kind === "action"
									? actionById.get(highlighted.action as PaletteActionId)
									: undefined
						}
						naming={naming}
						drop={drop}
						compatible={Boolean(accepts)}
						recents={recentEntries}
						moveTargets={moveTargets}
						currentModuleId={currentModuleId}
						onDrill={(path) => {
							changeScope(path);
							focusInput();
						}}
						onPlace={placeEntry}
					/>
				</div>
			</div>

			<PaletteFooter
				naming={naming !== null}
				count={footerCount({
					t,
					query: trimmed,
					results: results.length,
					category: currentCategory,
					scoped: scope.length > 0,
					compatible: Boolean(accepts),
				})}
			/>
		</PopoverContent>
	);
}

const rowDomId = (id: string) =>
	`node-palette-row-${id.replace(/[^\w-]/g, "_")}`;

const NAMING_HINT_ID = "node-palette-naming-hint";

type GroupRow = Extract<PaletteRow, { kind: "group" }>;

/** Splits the flat rows at each group header so every section renders as one listbox group. */
function groupRows(rows: PaletteRow[]) {
	const sections: { header?: GroupRow; rows: PaletteRow[] }[] = [];
	for (const row of rows) {
		if (row.kind === "group") {
			sections.push({ header: row, rows: [] });
			continue;
		}
		const last = sections[sections.length - 1];
		if (last) last.rows.push(row);
		else sections.push({ rows: [row] });
	}
	return sections;
}

function referentLabel(
	referent: PaletteReferent,
	t: Translate,
	selectionLabel: string,
) {
	if (referent === "here") return t("paletteHere", "Here");
	if (referent === "pin") return t("palettePin", "Pin");
	return selectionLabel;
}

function footerCount({
	t,
	query,
	results,
	category,
	scoped,
	compatible,
}: {
	t: Translate;
	query: string;
	results: number;
	category: PaletteCategory | undefined;
	scoped: boolean;
	compatible: boolean;
}) {
	if (query) {
		return t("paletteResultCount", "{{count, number}} results", {
			count: results,
		});
	}
	if (!category) return "";
	if (scoped) {
		return t("paletteCountIn", "{{count, number}} in {{category}}", {
			count: category.count,
			category: category.name,
		});
	}
	return countLabel(t, category.count, compatible);
}

/** Catalog icons are SVG masks painted with the background colour; the fallback is a stroked glyph. */
function NodeIcon({
	node,
	className,
	muted,
}: Readonly<{
	node: Pick<INode, "icon">;
	className?: string;
	muted?: boolean;
}>) {
	return node.icon ? (
		<DynamicImage
			url={node.icon}
			className={cn(
				"size-4 shrink-0",
				muted ? "bg-foreground/80" : "bg-foreground",
				className,
			)}
		/>
	) : (
		<WorkflowIcon
			className={cn(
				"size-4 shrink-0",
				muted ? "text-foreground/80" : "text-foreground",
				className,
			)}
		/>
	);
}

function MatchText({ text, query }: Readonly<{ text: string; query: string }>) {
	const parts = useMemo(() => {
		const tokens = tokenizeWithOperators(query.toLowerCase());
		if (tokens.length === 0) return [{ text, match: false }];
		const lower = text.toLowerCase();
		const marks = new Array<boolean>(text.length).fill(false);
		for (const token of tokens) {
			let fallback = -1;
			let found = false;
			for (
				let at = lower.indexOf(token);
				at >= 0;
				at = lower.indexOf(token, at + 1)
			) {
				const wordStart = at === 0 || !/[\p{L}\p{N}]/u.test(lower[at - 1]);
				if (wordStart) {
					marks.fill(true, at, at + token.length);
					found = true;
				} else if (fallback < 0) fallback = at;
			}
			if (!found && fallback >= 0)
				marks.fill(true, fallback, fallback + token.length);
		}
		const out: { text: string; match: boolean }[] = [];
		for (let i = 0; i < text.length; i++) {
			const last = out[out.length - 1];
			if (last && last.match === marks[i]) last.text += text[i];
			else out.push({ text: text[i], match: marks[i] });
		}
		return out;
	}, [text, query]);

	return (
		<>
			{parts.map((part, i) =>
				part.match ? (
					// biome-ignore lint/suspicious/noArrayIndexKey: segments are positional
					<b key={i} className="font-semibold">
						{part.text}
					</b>
				) : (
					part.text
				),
			)}
		</>
	);
}

function PinDot({
	pin,
	className,
}: Readonly<{
	pin: Pick<IPin, "data_type" | "value_type">;
	className?: string;
}>) {
	if (pin.data_type === IVariableType.Execution) {
		return (
			<svg
				viewBox="0 0 10 10"
				aria-hidden="true"
				className={cn("size-2.5 shrink-0 text-foreground", className)}
			>
				<path d="M1.5 1.5h3.7L8.6 5 5.2 8.5H1.5z" fill="currentColor" />
			</svg>
		);
	}
	return (
		<span
			aria-hidden
			className={cn(
				"size-2 shrink-0",
				pin.value_type === IValueType.Normal ? "rounded-full" : "rounded-[2px]",
				className,
			)}
			style={{ backgroundColor: typeToColor(pin.data_type) }}
		/>
	);
}

function LandingHint({ fit }: Readonly<{ fit?: { target?: IPin } }>) {
	if (!fit?.target) return null;
	return (
		<span className="ml-auto flex shrink-0 items-center gap-1.5 pl-2 font-mono text-[10.5px] text-muted-foreground">
			{"→"}
			<PinDot pin={fit.target} className="size-1.5" />
			<span className="max-w-28 truncate">{pinDisplayName(fit.target)}</span>
		</span>
	);
}

function emptyMessage(
	t: Translate,
	{
		reason,
		filtering,
		scopeName,
		pin,
		query,
	}: {
		reason: "query" | "scope";
		filtering: boolean;
		scopeName?: string;
		pin: string;
		query: string;
	},
) {
	if (reason === "query") {
		return filtering
			? t(
					"paletteEmptyCompatible",
					"No nodes that accept {{pin}} match “{{query}}”.",
					{ pin, query },
				)
			: t("paletteEmpty", "No nodes match “{{query}}”.", { query });
	}
	if (!filtering) return t("paletteEmptyScope", "Nothing to show here yet.");
	return scopeName
		? t(
				"paletteEmptyScopeCompatible",
				"Nothing in {{category}} accepts {{pin}}.",
				{
					category: scopeName,
					pin,
				},
			)
		: t("paletteEmptyRootCompatible", "No nodes accept {{pin}}.", { pin });
}

function groupLabel(group: PaletteGroup, t: Translate, scopeName?: string) {
	switch (group) {
		case "recent":
			return t("recent", "Recent");
		case "recentMatches":
			return t("paletteGroupRecentMatches", "Recent matches");
		case "browse":
			return t("paletteGroupBrowse", "Browse");
		case "actions":
			return t("actions", "Actions");
		case "nodes":
			return scopeName
				? t("paletteNodesIn", "Nodes in {{category}}", { category: scopeName })
				: t("nodes", "Nodes");
	}
}

function GroupHeader({
	row,
	scopeName,
	formatCount,
}: Readonly<{
	row: GroupRow;
	scopeName?: string;
	formatCount: (value: number) => string;
}>) {
	const { t } = useTranslation("flow");
	return (
		<div
			id={rowDomId(row.id)}
			role="presentation"
			className="flex items-baseline justify-between gap-2 px-2 pt-2.5 pb-1 font-medium text-[11px] text-muted-foreground uppercase tracking-wider"
		>
			<span>{groupLabel(row.group, t, scopeName)}</span>
			{row.count !== undefined && (
				<span className="font-mono font-normal normal-case tracking-normal">
					{formatCount(row.count)}
				</span>
			)}
		</div>
	);
}

const PaletteRowView = memo(function PaletteRowView({
	row,
	highlighted,
	query,
	drop,
	actionById,
	scopeName,
	dropPinName,
	filtering,
	formatCount,
}: Readonly<{
	row: PaletteRow;
	highlighted: boolean;
	query: string;
	drop?: NodePaletteDrop;
	actionById: Map<PaletteActionId, PaletteAction>;
	scopeName?: string;
	dropPinName: string;
	/** The compatible-only filter is narrowing the list. */
	filtering: boolean;
	formatCount: (value: number) => string;
}>) {
	const { t } = useTranslation("flow");

	if (row.kind === "group") {
		return (
			<GroupHeader row={row} scopeName={scopeName} formatCount={formatCount} />
		);
	}
	if (row.kind === "divider") {
		return <div role="presentation" className="mx-2 my-1.5 border-t" />;
	}
	if (row.kind === "overflow") {
		return (
			<div
				role="presentation"
				className="px-2 pt-2.5 pb-1 text-center text-muted-foreground text-xs"
			>
				{t(
					"paletteOverflow",
					"Showing {{shown, number}} of {{total, number}}. Refine your search",
					{ shown: row.shown, total: row.total },
				)}
			</div>
		);
	}
	if (row.kind === "empty") {
		return (
			<div
				role="presentation"
				className="text-balance px-4 pt-6 pb-2 text-center text-[12.5px] text-muted-foreground"
			>
				{emptyMessage(t, {
					reason: row.reason,
					filtering,
					scopeName,
					pin: dropPinName,
					query,
				})}
			</div>
		);
	}

	const shared = {
		id: rowDomId(row.id),
		role: "option" as const,
		"aria-selected": highlighted,
		"data-row-id": row.id,
		className: cn(
			"flex h-7.5 cursor-default select-none items-center gap-2.5 rounded-[5px] px-2 text-[13px]",
			highlighted && "bg-primary/15 ring-1 ring-primary ring-inset",
		),
	};

	if (row.kind === "widen") {
		return (
			<div {...shared}>
				{row.widen === "scope" ? (
					<SearchIcon className="size-4 shrink-0 text-muted-foreground" />
				) : (
					<ListIcon className="size-4 shrink-0 text-muted-foreground" />
				)}
				<span className="truncate">
					{row.widen === "scope"
						? query
							? t(
									"paletteWidenScope",
									"Search all categories for “{{query}}”",
									{
										query,
									},
								)
							: t("paletteShowAllCategories", "Show all categories")
						: t(
								"paletteWidenCompatible",
								"Include nodes that don't accept {{pin}}",
								{
									pin: dropPinName,
								},
							)}
				</span>
			</div>
		);
	}

	if (row.kind === "category") {
		return (
			<div {...shared}>
				{row.root && row.category.icon ? (
					<NodeIcon node={row.category} muted />
				) : (
					<FolderIcon className="size-4 shrink-0 text-muted-foreground" />
				)}
				<span className="truncate">{row.category.name}</span>
				<span className="ml-auto pl-2 font-mono text-[11px] text-muted-foreground">
					{formatCount(row.category.count)}
				</span>
				<ChevronRightIcon className="-mr-1 size-3.5 shrink-0 text-muted-foreground" />
			</div>
		);
	}

	if (row.kind === "action") {
		const action = actionById.get(row.action as PaletteActionId);
		if (!action) return null;
		const Icon = action.icon;
		return (
			<div {...shared}>
				<Icon className="size-4 shrink-0 text-muted-foreground" />
				<span className="truncate">
					<MatchText text={action.title} query={query} />
				</span>
				<span className="ml-auto shrink-0 rounded border px-1.5 font-mono text-[10px] text-muted-foreground leading-4">
					{action.referent === "here"
						? t("paletteTagHere", "here")
						: action.referent === "pin"
							? t("paletteTagPin", "pin")
							: t("paletteTagSelection", "selection")}
				</span>
				{action.id === "move" && (
					<ChevronRightIcon className="-mr-1 size-3.5 shrink-0 text-muted-foreground" />
				)}
			</div>
		);
	}

	const { entry } = row;
	const fit = drop?.fits.get(entry.key);
	const path = entry.path.join(" › ");

	if (row.variant === "result") {
		return (
			<div {...shared} className={cn(shared.className, "h-10.5")}>
				<NodeIcon node={entry.node} muted />
				<span className="flex min-w-0 flex-1 flex-col gap-px">
					<span className="flex min-w-0 items-center">
						<span className="truncate">
							<MatchText text={entry.node.friendly_name} query={query} />
						</span>
						<LandingHint fit={fit} />
					</span>
					<span className="flex min-w-0 items-baseline gap-2.5 text-[11px] text-muted-foreground">
						<span className="max-w-[62%] shrink-0 truncate">{path}</span>
						<span className="min-w-0 flex-1 truncate text-right font-mono text-[10.5px]">
							{pinSignature(entry.node)}
						</span>
					</span>
				</span>
			</div>
		);
	}

	return (
		<div {...shared}>
			<NodeIcon node={entry.node} muted />
			<span className="truncate" title={entry.node.friendly_name}>
				{entry.node.friendly_name}
			</span>
			{fit ? (
				<LandingHint fit={fit} />
			) : (
				row.variant === "compact" && (
					<span className="ml-auto min-w-0 truncate pl-2 text-[11px] text-muted-foreground">
						{path}
					</span>
				)
			)}
		</div>
	);
});

function ScopeChip({
	segments,
	icon: Icon,
	clearLabel,
	onClear,
}: Readonly<{
	segments: string[];
	icon?: LucideIcon;
	clearLabel: string;
	onClear: () => void;
}>) {
	const shown = segments.length > 2 ? ["…", ...segments.slice(-2)] : segments;
	return (
		<span className="inline-flex h-6 max-w-[52%] shrink-0 items-center rounded-[5px] border border-primary pr-0.5 pl-2 font-medium text-xs">
			{Icon && <Icon className="mr-1.5 size-3.5 text-muted-foreground" />}
			{shown.map((segment, i) => (
				<span
					// biome-ignore lint/suspicious/noArrayIndexKey: path segments may repeat
					key={i}
					className="flex min-w-0 items-center"
				>
					{i > 0 && <span className="px-1 text-muted-foreground">{"›"}</span>}
					<span className="truncate">{segment}</span>
				</span>
			))}
			<button
				type="button"
				aria-label={clearLabel}
				title={clearLabel}
				className="ml-1 grid size-5 shrink-0 place-items-center rounded-[3px] text-muted-foreground hover:bg-muted hover:text-foreground"
				onClick={onClear}
			>
				<XIcon className="size-3.5" />
			</button>
		</span>
	);
}

function DropChip({
	pin,
	active,
	onToggle,
}: Readonly<{ pin: IPin; active: boolean; onToggle: () => void }>) {
	const { t } = useTranslation("flow");
	const name = pinDisplayName(pin);
	const reference = isFunctionReferencePin(pin);
	const color = typeToColor(pin.data_type);
	// The name rides in as a component, not an interpolated value: Trans parses the
	// translated string as markup, so a pin called "List<Item>" would otherwise break it.
	const nameTag = (
		<b
			className={cn(
				"max-w-32 truncate max-sm:max-w-20",
				active ? "font-semibold text-foreground" : "font-medium",
			)}
		>
			{name}
		</b>
	);
	return (
		<button
			type="button"
			aria-pressed={active}
			title={
				active
					? t(
							"paletteCompatibleOn",
							"Showing nodes that accept {{pin}}. Click to show all nodes.",
							{ pin: name },
						)
					: t(
							"paletteCompatibleOff",
							"Showing all nodes. Click to show only nodes that accept {{pin}}.",
							{ pin: name },
						)
			}
			onClick={onToggle}
			className={cn(
				"inline-flex h-6.5 min-w-0 shrink items-center gap-1.5 rounded-full border px-2.5 text-muted-foreground text-xs whitespace-nowrap",
				!active && "border-dashed",
			)}
			style={
				active
					? {
							borderColor: `color-mix(in oklab, ${color} 45%, var(--border))`,
							backgroundColor: `color-mix(in oklch, ${color} 10%, transparent)`,
						}
					: undefined
			}
		>
			<span
				aria-hidden
				className="size-2 rounded-full"
				style={
					active
						? { backgroundColor: color }
						: { boxShadow: `inset 0 0 0 1.5px ${color}` }
				}
			/>
			<span className="flex min-w-0 items-center gap-1">
				{pin.pin_type === IPinType.Output ? (
					<Trans
						t={t}
						i18nKey="paletteFromPin"
						defaults="From <1/>"
						components={{ 1: nameTag }}
					/>
				) : (
					<Trans
						t={t}
						i18nKey="paletteIntoPin"
						defaults="Into <1/>"
						components={{ 1: nameTag }}
					/>
				)}
			</span>
			{!reference && (
				<span className="flex items-center gap-1.5 max-sm:hidden">
					<span aria-hidden>{"·"}</span>
					<span className="font-mono text-[11px] text-foreground">
						{pinTypeLabel(pin)}
					</span>
				</span>
			)}
		</button>
	);
}

function QuickAction({
	action,
	active,
	onRun,
	onPeek,
}: Readonly<{
	action: PaletteAction;
	active?: boolean;
	onRun: () => void;
	onPeek: (id: PaletteActionId | null, immediate?: boolean) => void;
}>) {
	const Icon = action.icon;
	return (
		<button
			type="button"
			data-palette-action={action.id}
			data-active={active || undefined}
			aria-label={action.title}
			title={action.title}
			className="group relative inline-flex h-6.5 items-center gap-1.5 rounded-[5px] px-2 font-medium text-foreground/90 text-xs whitespace-nowrap hover:bg-muted hover:text-foreground data-active:bg-muted"
			onClick={onRun}
			onMouseEnter={() => onPeek(action.id)}
			onMouseLeave={() => onPeek(null)}
			onFocus={() => onPeek(action.id, true)}
			onBlur={() => onPeek(null)}
		>
			<Icon className="size-3.5 shrink-0 text-muted-foreground group-hover:text-foreground" />
			<span className="max-sm:hidden">{action.label}</span>
			{action.live && (
				<span
					aria-hidden
					className="size-1.5 rounded-full bg-emerald-500 ring-2 ring-emerald-500/20 motion-safe:animate-pulse"
				/>
			)}
		</button>
	);
}

function MoveMenu({
	action,
	open,
	targets,
	currentModuleId,
	onOpenChange,
	onMove,
	onReturnFocus,
	onPeek,
}: Readonly<{
	action: PaletteAction;
	open: boolean;
	targets: MoveTarget[];
	currentModuleId: string | null;
	onOpenChange: (open: boolean) => void;
	onMove: (target: string | null) => void;
	onReturnFocus: () => void;
	onPeek: (id: PaletteActionId | null, immediate?: boolean) => void;
}>) {
	const { t } = useTranslation("flow");
	return (
		// Not modal: the palette around it already blocks the rest of the app, and a second
		// modal layer torn down together with the palette leaves `pointer-events: none` stuck
		// on <body>.
		<DropdownMenu modal={false} open={open} onOpenChange={onOpenChange}>
			<DropdownMenuTrigger asChild>
				<button
					type="button"
					data-palette-action={action.id}
					aria-label={action.title}
					title={action.title}
					className="group inline-flex h-6.5 items-center gap-1.5 rounded-[5px] px-2 font-medium text-foreground/90 text-xs whitespace-nowrap hover:bg-muted hover:text-foreground data-[state=open]:bg-muted"
					onMouseEnter={() => onPeek(action.id)}
					onMouseLeave={() => onPeek(null)}
				>
					<action.icon className="size-3.5 shrink-0 text-muted-foreground group-hover:text-foreground" />
					<span className="max-sm:hidden">{action.label}</span>
					<ChevronDownIcon className="-ml-0.5 size-3 text-muted-foreground" />
				</button>
			</DropdownMenuTrigger>
			<DropdownMenuContent
				align="start"
				className="min-w-52"
				onCloseAutoFocus={(event) => {
					event.preventDefault();
					onReturnFocus();
				}}
				onKeyDown={(event) => {
					if (event.key !== "ArrowLeft") return;
					event.preventDefault();
					onOpenChange(false);
				}}
			>
				<DropdownMenuLabel className="font-medium text-[11px] text-muted-foreground uppercase tracking-wider">
					{t("paletteMoveTo", "Move to")}
				</DropdownMenuLabel>
				{targets.map((target) => (
					<DropdownMenuItem
						key={target.id ?? "main"}
						disabled={target.id === currentModuleId}
						className="font-mono text-xs"
						onSelect={() => onMove(target.id)}
					>
						<FileIcon className="size-3.5 text-muted-foreground" />
						<span className="truncate">{target.label}</span>
						{target.id === currentModuleId && <CurrentBadge />}
					</DropdownMenuItem>
				))}
			</DropdownMenuContent>
		</DropdownMenu>
	);
}

function CurrentBadge() {
	const { t } = useTranslation("flow");
	return (
		<span className="ml-auto rounded border px-1.5 font-sans text-[10px] text-muted-foreground leading-4">
			{t("paletteCurrent", "current")}
		</span>
	);
}

function Breadcrumbs({
	scope,
	onNavigate,
}: Readonly<{ scope: string[]; onNavigate: (path: string[]) => void }>) {
	const { t } = useTranslation("flow");
	return (
		<nav
			aria-label={t("paletteCategoryPath", "Category path")}
			className="flex h-7 shrink-0 items-center gap-px overflow-hidden border-b px-2 text-[11.5px] text-muted-foreground whitespace-nowrap"
		>
			{[t("all", "All"), ...scope].map((segment, depth) => {
				const last = depth === scope.length;
				return (
					<span
						// biome-ignore lint/suspicious/noArrayIndexKey: depth identifies the crumb
						key={depth}
						className="flex min-w-0 items-center"
					>
						{depth > 0 && (
							<span aria-hidden className="px-px opacity-70">
								{"›"}
							</span>
						)}
						{last ? (
							<span
								aria-current="location"
								className="truncate px-1.5 py-0.5 font-medium text-foreground"
							>
								{segment}
							</span>
						) : (
							<button
								type="button"
								className="rounded px-1.5 py-0.5 hover:bg-muted hover:text-foreground"
								onClick={() => onNavigate(scope.slice(0, depth))}
							>
								{segment}
							</button>
						)}
					</span>
				);
			})}
		</nav>
	);
}

function NamingRows({
	name,
	invalid,
	onPlace,
}: Readonly<{ name: string; invalid: boolean; onPlace: () => void }>) {
	const { t } = useTranslation("flow");
	const trimmed = name.trim();
	return (
		<div className="min-h-0 flex-1 overflow-y-auto px-1.5 pt-0.5 pb-2">
			<div className="px-2 pt-1.5 pb-1 font-medium text-[11px] text-muted-foreground uppercase tracking-wider">
				{t("paletteNewPlaceholder", "New placeholder")}
			</div>
			<button
				type="button"
				data-naming-row
				onClick={onPlace}
				className="flex h-7.5 w-full items-center gap-2.5 rounded-[5px] bg-primary/15 px-2 text-left text-[13px] ring-1 ring-primary ring-inset"
			>
				<ZapIcon className="size-4 shrink-0 text-muted-foreground" />
				<span className="truncate">
					{trimmed
						? t("palettePlacePlaceholder", "Place “{{name}}” here", {
								name: trimmed,
							})
						: t(
								"paletteTypePlaceholderName",
								"Type a name for the placeholder",
							)}
				</span>
				<span className="ml-auto font-mono text-[11px] text-muted-foreground">
					{"↵"}
				</span>
			</button>
			<p
				id={NAMING_HINT_ID}
				role={invalid ? "alert" : undefined}
				className={cn(
					"px-2 py-1.5 text-[11.5px] leading-snug",
					invalid ? "text-foreground" : "text-muted-foreground",
				)}
			>
				{invalid
					? t(
							"palettePlaceholderNeedsName",
							"Give the placeholder a name first.",
						)
					: t(
							"palettePlaceholderHint",
							"Type a name, then press ↵ to place it here. Esc cancels.",
						)}
			</p>
		</div>
	);
}

function PaletteFooter({
	naming,
	count,
}: Readonly<{ naming: boolean; count: string }>) {
	const { t } = useTranslation("flow");
	const esc = t("paletteKeyEsc", "esc");
	const keys: [string, string][] = naming
		? [
				["↵", t("paletteKeyPlace", "place")],
				[esc, t("paletteKeyCancel", "cancel")],
			]
		: [
				["↑↓", t("paletteKeyMove", "move")],
				["→", t("paletteKeyOpen", "open")],
				["←", t("paletteKeyBack", "back")],
				["↵", t("paletteKeyPlace", "place")],
				[esc, t("paletteKeyClose", "close")],
			];
	return (
		<div className="flex h-7.5 items-center justify-between gap-3 overflow-hidden border-t px-3 font-mono text-[10.5px] text-muted-foreground whitespace-nowrap">
			<span className="truncate" aria-hidden>
				{keys.map(([key, label], i) => (
					<span key={key + label}>
						{i > 0 && <span className="opacity-60"> {"·"} </span>}
						<span className="text-foreground/80">{key}</span> {label}
					</span>
				))}
			</span>
			<output className="shrink-0 max-sm:sr-only">{naming ? "" : count}</output>
		</div>
	);
}

function PaletteDetail({
	row,
	action,
	naming,
	drop,
	compatible,
	recents,
	moveTargets,
	currentModuleId,
	onDrill,
	onPlace,
}: Readonly<{
	row?: PaletteSelectableRow;
	action?: PaletteAction;
	naming: string | null;
	drop?: NodePaletteDrop;
	/** The compatible-only filter is on, so category counts are compatible counts. */
	compatible: boolean;
	recents: PaletteEntry[];
	moveTargets: MoveTarget[];
	currentModuleId: string | null;
	onDrill: (path: string[]) => void;
	onPlace: (entry: PaletteEntry) => void;
}>) {
	const { t } = useTranslation("flow");

	if (naming !== null) {
		return (
			<ActionDetail
				title={naming.trim() || t("placeholder", "Placeholder")}
				description={t(
					"palettePlaceholderDescription",
					"Places a named stand-in you can swap for a real node later.",
				)}
				referent="here"
				preview={<PlaceholderPreview name={naming.trim()} />}
			/>
		);
	}

	if (action) {
		return (
			<ActionDetail
				title={action.title}
				description={action.description}
				referent={action.referent}
				pinName={drop ? pinDisplayName(drop.pin) : undefined}
				preview={<ActionPreview action={action} />}
				moveTargets={action.id === "move" ? moveTargets : undefined}
				currentModuleId={currentModuleId}
			/>
		);
	}

	if (!row) {
		return (
			<p className="text-balance px-1.5 py-8 text-center text-muted-foreground text-xs">
				{t("paletteNothingToPreview", "Nothing to preview.")}
			</p>
		);
	}

	if (row.kind === "node") return <NodeDetail entry={row.entry} drop={drop} />;

	if (row.kind === "category") {
		return (
			<CategoryDetail
				category={row.category}
				recents={recents}
				compatible={compatible}
				onDrill={onDrill}
				onPlace={onPlace}
			/>
		);
	}

	if (row.kind === "action") return null;

	return (
		<p className="text-pretty py-2 text-[12.5px] text-muted-foreground leading-relaxed">
			{row.widen === "scope"
				? t(
						"paletteWidenScopeDescription",
						"Clears the category scope and searches every category.",
					)
				: t(
						"paletteWidenCompatibleDescription",
						"Turns off the compatible-only filter. Nodes that can't take this wire are placed unconnected.",
					)}
		</p>
	);
}

function PreviewStage({
	wired,
	children,
}: Readonly<{ wired?: boolean; children: ReactNode }>) {
	return (
		<div
			className={cn(
				"relative mb-3 rounded-md border bg-background p-3.5",
				wired && "pl-8",
			)}
			style={{
				backgroundImage:
					"radial-gradient(circle at 1px 1px, color-mix(in oklch, var(--foreground) 13%, transparent) 1px, transparent 0)",
				backgroundSize: "12px 12px",
			}}
		>
			{children}
		</div>
	);
}

function headerTone(node: INode, isExec: boolean) {
	if (node.start) return "bg-linear-to-r from-card via-primary/50 to-primary";
	if (node.event_callback)
		return "bg-linear-to-l from-card via-primary/50 to-primary";
	if (node.name === "control_call_function")
		return "bg-linear-to-r from-card via-violet-500/50 to-violet-500";
	if (!isExec) return "bg-linear-to-r from-card via-tertiary/50 to-tertiary";
	return "bg-card";
}

function visiblePins(pins: IPin[], keep?: IPin) {
	if (pins.length <= PREVIEW_PIN_ROWS) return { shown: pins, hidden: 0 };
	const shown = pins.slice(0, PREVIEW_PIN_ROWS - 1);
	if (keep && !shown.includes(keep)) shown[shown.length - 1] = keep;
	return { shown, hidden: pins.length - shown.length };
}

function PreviewPinColumn({
	pins,
	side,
	landing,
}: Readonly<{ pins: IPin[]; side: "in" | "out"; landing?: IPin }>) {
	const { t } = useTranslation("flow");
	const { shown, hidden } = visiblePins(pins, landing);
	return (
		<div
			className={cn(
				"flex min-w-0 flex-col",
				side === "out" && "items-end text-right",
			)}
		>
			{shown.map((pin) => {
				const lands = pin === landing;
				return (
					<div
						key={pin.id}
						title={`${pinDisplayName(pin)} · ${pinTypeLabel(pin)}`}
						className={cn(
							"relative flex h-4.5 max-w-full items-center px-2",
							lands ? "font-semibold text-foreground" : "text-foreground/75",
						)}
					>
						{lands && (
							<span
								aria-hidden
								className="-left-6.5 absolute top-1/2 w-5.5 border-t-2 border-dashed"
								style={{ borderColor: typeToColor(pin.data_type) }}
							/>
						)}
						<PinDot
							pin={pin}
							className={cn(
								"absolute top-1/2 -translate-y-1/2",
								side === "in" ? "-left-1" : "-right-1",
								lands && "ring-2 ring-card outline-2 outline-offset-2",
							)}
						/>
						<span className="truncate">{pinDisplayName(pin)}</span>
					</div>
				);
			})}
			{hidden > 0 && (
				<div className="flex h-4.5 items-center px-2 text-[10px] text-muted-foreground">
					{t("paletteMore", "+{{count}} more", { count: hidden })}
				</div>
			)}
		</div>
	);
}

function NodePreview({
	node,
	landing,
}: Readonly<{ node: INode; landing?: IPin }>) {
	const inputs = sortedPins(node, IPinType.Input);
	const outputs = sortedPins(node, IPinType.Output);
	const isExec = [...inputs, ...outputs].some(
		(pin) => pin.data_type === IVariableType.Execution,
	);
	return (
		<div className="rounded-md border bg-card text-[10.5px] shadow-xs">
			<div
				className={cn(
					"flex h-5.5 items-center gap-1.5 rounded-t-[5px] border-b px-2 font-semibold text-[11px]",
					headerTone(node, isExec),
				)}
			>
				<NodeIcon node={node} className="size-3" />
				<span className="truncate">{node.friendly_name}</span>
			</div>
			{(inputs.length > 0 || outputs.length > 0) && (
				<div className="grid grid-cols-2 gap-x-2.5 py-1">
					<PreviewPinColumn pins={inputs} side="in" landing={landing} />
					<PreviewPinColumn pins={outputs} side="out" />
				</div>
			)}
		</div>
	);
}

function PlaceholderPreview({ name }: Readonly<{ name: string }>) {
	const { t } = useTranslation("flow");
	return (
		<PreviewStage>
			<div className="flex h-5.5 items-center gap-1.5 rounded-md border border-foreground/35 border-dashed bg-card px-2 font-semibold text-[11px]">
				<ZapIcon className="size-3 shrink-0" />
				<span className="truncate">
					{name || t("placeholder", "Placeholder")}
				</span>
			</div>
		</PreviewStage>
	);
}

function ActionPreview({ action }: Readonly<{ action: PaletteAction }>) {
	if (action.id === "placeholder") return <PlaceholderPreview name="" />;
	if (action.id === "comment") {
		return (
			<PreviewStage>
				<div className="h-18.5 rounded-md border-[1.5px] border-foreground/30 border-dashed bg-foreground/3 px-2 py-1.5 font-medium text-[11px] text-muted-foreground">
					{action.title}
				</div>
			</PreviewStage>
		);
	}
	if (action.id === "ping") {
		return (
			<PreviewStage>
				<div className="grid h-14 place-items-center">
					<span className="relative size-2.5 rounded-full bg-primary">
						<span className="absolute inset-0 rounded-full bg-primary motion-safe:animate-ping" />
					</span>
				</div>
			</PreviewStage>
		);
	}
	const Icon = action.icon;
	return (
		<div className="mb-2.5 grid size-10 place-items-center rounded-lg bg-muted">
			<Icon className="size-4.5" />
		</div>
	);
}

function ActionDetail({
	title,
	description,
	referent,
	pinName,
	preview,
	moveTargets,
	currentModuleId,
}: Readonly<{
	title: string;
	description: string;
	referent: PaletteReferent;
	pinName?: string;
	preview: ReactNode;
	moveTargets?: MoveTarget[];
	currentModuleId?: string | null;
}>) {
	const { t } = useTranslation("flow");
	const appliesTo =
		referent === "here"
			? t("paletteAppliesHere", "This point on the canvas")
			: referent === "pin"
				? t("paletteAppliesPin", "The dragged pin {{pin}}", {
						pin: pinName ?? "",
					})
				: t("paletteAppliesSelection", "The current selection");
	return (
		<>
			{preview}
			<h3 className="text-balance font-semibold text-sm leading-snug">
				{title}
			</h3>
			<p className="mt-2 text-pretty text-[12.5px] text-foreground/85 leading-relaxed">
				{description}
			</p>
			{moveTargets && (
				<ul className="mt-2.5">
					{moveTargets.map((target) => (
						<li
							key={target.id ?? "main"}
							className="flex h-6.5 items-center gap-2 px-1.5 font-mono text-[11.5px]"
						>
							<FileIcon className="size-3.5 text-muted-foreground" />
							<span className="truncate">{target.label}</span>
							{target.id === currentModuleId && <CurrentBadge />}
						</li>
					))}
				</ul>
			)}
			<p className="mt-3 rounded-md border px-2.5 py-2 text-xs leading-relaxed">
				<Trans
					t={t}
					i18nKey="paletteAppliesToTarget"
					defaults="<1>Applies to:</1> <2/>"
					components={{
						1: <span className="text-muted-foreground" />,
						2: <span>{appliesTo}</span>,
					}}
				/>
			</p>
		</>
	);
}

function DropLanding({
	drop,
	fit,
}: Readonly<{ drop: NodePaletteDrop; fit?: { target?: IPin } }>) {
	const { t } = useTranslation("flow");
	const pinName = pinDisplayName(drop.pin);
	if (fit?.target) {
		return (
			<div className="mt-2 flex flex-wrap items-center gap-1.5 rounded-[5px] bg-muted px-2 py-1.5 text-[11.5px]">
				<PinDot pin={fit.target} className="size-1.75" />
				<span>
					<Trans
						t={t}
						i18nKey="paletteConnectsToTarget"
						defaults="<1/> connects to <2/>"
						components={{
							1: <span>{pinName}</span>,
							2: <b className="font-semibold">{pinDisplayName(fit.target)}</b>,
						}}
					/>
				</span>
				<span className="ml-auto font-mono text-[10.5px] text-muted-foreground">
					{pinTypeLabel(fit.target)}
				</span>
			</div>
		);
	}
	return (
		<div className="mt-2 rounded-[5px] bg-muted px-2 py-1.5 text-[11.5px] text-muted-foreground">
			{fit && isFunctionReferencePin(drop.pin)
				? t("paletteConnectsAsReference", "Connects as a function reference.")
				: t(
						"paletteNoLanding",
						"Nothing here accepts {{pin}}. It is placed unconnected.",
						{ pin: pinName },
					)}
		</div>
	);
}

const QUALITY_KEYS = [
	"privacy",
	"security",
	"performance",
	"governance",
	"reliability",
	"cost",
] as const;

function QualityScores({
	scores,
}: Readonly<{ scores: NonNullable<INode["scores"]> }>) {
	const { t } = useTranslation("flow");
	const labels: Record<(typeof QUALITY_KEYS)[number], string> = {
		privacy: t("privacy", "Privacy"),
		security: t("security", "Security"),
		performance: t("performance", "Performance"),
		governance: t("governance", "Governance"),
		reliability: t("reliability", "Reliability"),
		cost: t("cost", "Cost"),
	};
	return (
		<DetailSection title={t("quality", "Quality")}>
			<div className="grid grid-cols-2 gap-x-3.5 gap-y-2">
				{QUALITY_KEYS.map((key) => {
					const value = Math.max(0, Math.min(10, Number(scores[key]) || 0));
					return (
						<div key={key}>
							<div className="flex justify-between gap-1.5 text-[11px] text-muted-foreground">
								<span>{labels[key]}</span>
								<span className="font-mono text-[10.5px] text-foreground tabular-nums">
									{value}
								</span>
							</div>
							<div
								role="meter"
								aria-label={labels[key]}
								aria-valuemin={0}
								aria-valuemax={10}
								aria-valuenow={value}
								className="mt-1 h-1 overflow-hidden rounded-full bg-muted"
							>
								<div
									className="h-full rounded-full bg-foreground/60"
									style={{ width: `${value * 10}%` }}
								/>
							</div>
						</div>
					);
				})}
			</div>
		</DetailSection>
	);
}

function NodeDetail({
	entry,
	drop,
}: Readonly<{ entry: PaletteEntry; drop?: NodePaletteDrop }>) {
	const { t } = useTranslation("flow");
	const fit = drop?.fits.get(entry.key);
	const { node } = entry;
	return (
		<>
			<PreviewStage wired={Boolean(fit?.target)}>
				<NodePreview node={node} landing={fit?.target} />
			</PreviewStage>
			<h3 className="text-balance wrap-break-word font-semibold text-sm leading-snug">
				{node.friendly_name}
			</h3>
			<p className="mt-0.5 text-[11.5px] text-muted-foreground leading-snug">
				{entry.path.join(" › ")}
			</p>
			{drop && <DropLanding drop={drop} fit={fit} />}
			<p className="mt-2 text-pretty wrap-break-word text-[12.5px] text-foreground/85 leading-relaxed">
				{node.description || (
					<span className="text-muted-foreground">
						{t("paletteNoDescription", "No description yet.")}
					</span>
				)}
			</p>
			{node.scores && <QualityScores scores={node.scores} />}
		</>
	);
}

const DETAIL_SUBCATEGORIES = 8;
const DETAIL_RECENTS = 3;
const DETAIL_SAMPLE = 5;

/** The first `limit` entries of a category: its own nodes, then its subcategories' in order. */
function sampleEntries(
	category: PaletteCategory,
	limit: number,
	skip: ReadonlySet<string>,
): PaletteEntry[] {
	const out: PaletteEntry[] = [];
	const visit = (node: PaletteCategory) => {
		for (const entry of node.entries) {
			if (out.length === limit) return;
			if (!skip.has(entry.key)) out.push(entry);
		}
		for (const child of node.children) {
			if (out.length === limit) return;
			visit(child);
		}
	};
	visit(category);
	return out;
}

function DetailSection({
	title,
	children,
}: Readonly<{ title: string; children: ReactNode }>) {
	return (
		<section className="mt-3.5">
			<h4 className="mb-1.5 font-medium text-[11px] text-muted-foreground uppercase leading-4 tracking-wider">
				{title}
			</h4>
			{children}
		</section>
	);
}

const DETAIL_ROW =
	"flex h-6.5 w-full items-center gap-2 rounded px-1.5 text-left text-[12.5px] hover:bg-muted";

function EntryList({
	entries,
	onPlace,
}: Readonly<{
	entries: PaletteEntry[];
	onPlace: (entry: PaletteEntry) => void;
}>) {
	return (
		<ul>
			{entries.map((entry) => (
				<li key={entry.key}>
					<button
						type="button"
						data-detail-entry={entry.key}
						title={entry.path.join(" › ")}
						className={DETAIL_ROW}
						onClick={() => onPlace(entry)}
					>
						<NodeIcon node={entry.node} className="size-3.5" muted />
						<span className="truncate">{entry.node.friendly_name}</span>
					</button>
				</li>
			))}
		</ul>
	);
}

function CategoryDetail({
	category,
	recents,
	compatible,
	onDrill,
	onPlace,
}: Readonly<{
	category: PaletteCategory;
	recents: PaletteEntry[];
	compatible: boolean;
	onDrill: (path: string[]) => void;
	onPlace: (entry: PaletteEntry) => void;
}>) {
	const { t } = useTranslation("flow");
	const formatCount = useFormatCount();
	const recentHere = recents
		.filter((entry) => inScope(entry, category.path))
		.slice(0, DETAIL_RECENTS);
	const sample = sampleEntries(
		category,
		DETAIL_SAMPLE,
		new Set(recentHere.map((entry) => entry.key)),
	);
	const children = category.children.slice(0, DETAIL_SUBCATEGORIES);
	return (
		<>
			<div className="flex items-center gap-2.5">
				<span className="grid size-9 shrink-0 place-items-center rounded-lg bg-muted">
					{category.icon ? (
						<NodeIcon node={category} className="size-4.5" />
					) : (
						<FolderIcon className="size-4.5" />
					)}
				</span>
				<div className="min-w-0">
					<h3 className="truncate font-semibold text-sm">{category.name}</h3>
					<p className="font-mono text-[11px] text-muted-foreground leading-snug">
						{countLabel(t, category.count, compatible)}
					</p>
				</div>
			</div>
			{category.path.length > 1 && (
				<p className="mt-2 text-[11.5px] text-muted-foreground leading-snug">
					{category.path.join(" › ")}
				</p>
			)}
			{children.length > 0 && (
				<DetailSection title={t("paletteSubcategories", "Subcategories")}>
					<ul>
						{children.map((child) => (
							<li key={child.key}>
								<button
									type="button"
									className={DETAIL_ROW}
									onClick={() => onDrill(child.path)}
								>
									<FolderIcon className="size-3.5 shrink-0 text-muted-foreground" />
									<span className="truncate">{child.name}</span>
									<span className="ml-auto font-mono text-[11px] text-muted-foreground">
										{formatCount(child.count)}
									</span>
								</button>
							</li>
						))}
					</ul>
					{category.children.length > children.length && (
						<p className="px-1.5 pt-1 text-[11px] text-muted-foreground leading-snug">
							{t("paletteMore", "+{{count}} more", {
								count: category.children.length - children.length,
							})}
						</p>
					)}
				</DetailSection>
			)}
			{recentHere.length > 0 && (
				<DetailSection title={t("paletteRecentHere", "Recent here")}>
					<EntryList entries={recentHere} onPlace={onPlace} />
				</DetailSection>
			)}
			{sample.length > 0 && (
				<DetailSection title={t("paletteIncludes", "Includes")}>
					<EntryList entries={sample} onPlace={onPlace} />
				</DetailSection>
			)}
		</>
	);
}
