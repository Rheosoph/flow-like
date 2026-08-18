"use client";

import { i18n as i18next } from "@flow-like/locales";
import {
	ChevronLeftIcon,
	ChevronRightIcon,
	FocusIcon,
	LayersIcon,
	MessageSquareTextIcon,
	PanelRightCloseIcon,
	SearchIcon,
	VariableIcon,
	XIcon,
} from "lucide-react";
import MiniSearch from "minisearch";
import { memo, useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { IBoard, ILayer, INode, IPin } from "../../lib/schema/flow/board";
import { parseUint8ArrayToJson } from "../../lib/uint8";
import { cn } from "../../lib/utils";
import { Badge } from "../ui/badge";
import { Button } from "../ui/button";
import {
	CommandDialog,
	CommandEmpty,
	CommandGroup,
	CommandInput,
	CommandItem,
	CommandList,
} from "../ui/command";
import { Input } from "../ui/input";
import {
	Tooltip,
	TooltipContent,
	TooltipProvider,
	TooltipTrigger,
} from "../ui/tooltip";

export interface SearchResult {
	id: string;
	type: "node" | "layer" | "pin" | "pin-value" | "comment" | "variable";
	nodeId: string;
	layerId?: string;
	layerPath?: string[];
	name: string;
	nodeName: string;
	description: string;
	matchedField: string;
	matchedValue: string;
	category?: string;
	pinName?: string;
	pinType?: string;
	dataType?: string;
	icon?: string | null;
	searchText: string;
}

interface FlowSearchProps {
	board: IBoard | undefined;
	open: boolean;
	onOpenChange: (open: boolean) => void;
	onNavigate: (nodeId: string) => void;
	mode?: "dialog" | "sidebar";
	onSwitchToSidebar?: () => void;
}

function getLayerPath(
	layers: Record<string, ILayer>,
	layerId: string | null | undefined,
): string[] {
	if (!layerId) return [];
	const path: string[] = [];
	let currentId: string | null | undefined = layerId;
	let iteration = 0;

	while (currentId && iteration < 40) {
		iteration++;
		const layer: ILayer | undefined = layers[currentId];
		if (!layer) break;
		path.unshift(layer.name);
		currentId = layer.parent_id;
	}
	return path;
}

function decodeDefaultValue(pin: IPin): string | undefined {
	if (!pin.default_value || pin.default_value.length === 0) return undefined;
	try {
		const parsed = parseUint8ArrayToJson(pin.default_value);
		if (parsed === null || parsed === undefined) return undefined;
		if (typeof parsed === "string") return parsed;
		if (typeof parsed === "number" || typeof parsed === "boolean")
			return String(parsed);
		if (Array.isArray(parsed)) return JSON.stringify(parsed);
		if (typeof parsed === "object") return JSON.stringify(parsed);
		return String(parsed);
	} catch {
		return undefined;
	}
}

function buildSearchDocuments(board: IBoard | undefined): SearchResult[] {
	if (!board) return [];

	const results: SearchResult[] = [];
	const layers = board.layers ?? {};

	const processNode = (node: INode, layerId?: string, layerPath?: string[]) => {
		const nodeName = node.friendly_name || node.name;

		// Index the node itself with all searchable fields
		const nodeSearchText = [
			node.friendly_name,
			node.name,
			node.description,
			node.category,
			node.comment,
			node.docs,
			// Include all pin names for node-level search
			...Object.values(node.pins).flatMap((pin) => [
				pin.friendly_name,
				pin.name,
				pin.description,
			]),
		]
			.filter(Boolean)
			.join(" ");

		results.push({
			id: `node-${node.id}`,
			type: "node",
			nodeId: node.id,
			layerId,
			layerPath,
			name: nodeName,
			nodeName,
			description: node.description || "",
			matchedField: "node",
			matchedValue: nodeName,
			category: node.category,
			icon: node.icon,
			searchText: nodeSearchText,
		});

		// Index node comment separately for better matching
		if (node.comment) {
			results.push({
				id: `comment-${node.id}`,
				type: "comment",
				nodeId: node.id,
				layerId,
				layerPath,
				name: nodeName,
				nodeName,
				description: node.description || "",
				matchedField: "comment",
				matchedValue: node.comment,
				category: node.category,
				icon: node.icon,
				searchText: `${nodeName} ${node.comment}`,
			});
		}

		// Index EVERY pin (not just those with default values)
		for (const pin of Object.values(node.pins)) {
			// Skip execution pins as they're not searchable content
			if (pin.data_type === "Execution") continue;

			const pinDisplayName = pin.friendly_name || pin.name;
			const decodedValue = decodeDefaultValue(pin);

			// Index pin by name/friendly_name
			results.push({
				id: `pin-name-${node.id}-${pin.id}`,
				type: "pin",
				nodeId: node.id,
				layerId,
				layerPath,
				name: `${nodeName} → ${pinDisplayName}`,
				nodeName,
				description: pin.description || "",
				matchedField: "pin",
				matchedValue: pinDisplayName,
				pinName: pinDisplayName,
				pinType: pin.pin_type,
				dataType: pin.data_type,
				category: node.category,
				icon: node.icon,
				searchText: `${nodeName} ${pinDisplayName} ${pin.name} ${pin.description || ""} ${pin.data_type || ""}`,
			});

			// Index pin default value if exists
			if (decodedValue && decodedValue.trim()) {
				results.push({
					id: `pin-value-${node.id}-${pin.id}`,
					type: "pin-value",
					nodeId: node.id,
					layerId,
					layerPath,
					name: `${nodeName} → ${pinDisplayName}`,
					nodeName,
					description: decodedValue,
					matchedField: "value",
					matchedValue: decodedValue,
					pinName: pinDisplayName,
					pinType: pin.pin_type,
					dataType: pin.data_type,
					category: node.category,
					icon: node.icon,
					searchText: `${nodeName} ${pinDisplayName} ${decodedValue}`,
				});
			}
		}
	};

	// Process root-level nodes
	for (const node of Object.values(board.nodes)) {
		const path = getLayerPath(layers, node.layer);
		processNode(
			node,
			node.layer ?? undefined,
			path.length > 0 ? path : undefined,
		);
	}

	// Process layers and their nodes
	for (const layer of Object.values(layers)) {
		const path = getLayerPath(layers, layer.id);

		// Index the layer itself
		results.push({
			id: `layer-${layer.id}`,
			type: "layer",
			nodeId: layer.id,
			layerId: layer.parent_id ?? undefined,
			layerPath: path.slice(0, -1).length > 0 ? path.slice(0, -1) : undefined,
			name: layer.name,
			nodeName: layer.name,
			description: layer.comment ?? "Layer",
			matchedField: "layer",
			matchedValue: layer.name,
			searchText: `${layer.name} ${layer.comment || ""} layer`,
		});

		// Process nodes inside the layer
		for (const node of Object.values(layer.nodes)) {
			processNode(node, layer.id, path);
		}
	}

	// Index board variables
	for (const variable of Object.values(board.variables ?? {})) {
		const decodedValue = variable.default_value
			? (() => {
					try {
						const parsed = parseUint8ArrayToJson(variable.default_value);
						if (parsed === null || parsed === undefined) return undefined;
						if (typeof parsed === "string") return parsed;
						return JSON.stringify(parsed);
					} catch {
						return undefined;
					}
				})()
			: undefined;

		results.push({
			id: `variable-${variable.id}`,
			type: "variable",
			nodeId: variable.id,
			name: variable.name,
			nodeName: variable.name,
			description: variable.description || i18next.t('data_typeVariable', '{{data_type}} variable', { data_type: variable.data_type }),
			matchedField: "variable",
			matchedValue: variable.name,
			category: variable.category ?? undefined,
			dataType: variable.data_type,
			searchText: `${variable.name} ${variable.description || ""} ${variable.data_type} ${decodedValue || ""} variable`,
		});
	}

	return results;
}

interface BuiltSearchIndex {
	board: IBoard | undefined;
	documents: SearchResult[];
	index: MiniSearch<SearchResult>;
	docMap: Map<string, SearchResult>;
}

/**
 * Indexing a large board blocks the main thread for tens to hundreds of
 * milliseconds, so the index is only (re)built while the search UI is open and
 * is reused across open/close cycles as long as the board object is unchanged.
 */
function useSearchIndex(board: IBoard | undefined, enabled: boolean) {
	const cache = useRef<BuiltSearchIndex | null>(null);

	const built = useMemo(() => {
		if (!enabled) return cache.current;
		if (cache.current?.board === board) return cache.current;
		const documents = buildSearchDocuments(board);
		const miniSearch = new MiniSearch<SearchResult>({
			fields: [
				"name",
				"nodeName",
				"searchText",
				"matchedValue",
				"description",
				"category",
				"pinName",
				"dataType",
			],
			storeFields: ["id"],
			searchOptions: {
				prefix: true,
				fuzzy: 0.3,
				boost: {
					name: 5,
					nodeName: 4,
					matchedValue: 3,
					pinName: 2.5,
					searchText: 2,
					category: 1.5,
					dataType: 1,
					description: 0.75,
				},
				combineWith: "OR",
			},
			tokenize: (text) => {
				// Split on common separators
				const tokens = text.toLowerCase().split(/[\s\-_./\\:,;'"()[\]{}|<>]+/);
				const additionalTokens: string[] = [];

				for (const token of tokens) {
					if (token.length > 2) {
						// Split camelCase and PascalCase
						const camelParts = token.split(/(?=[A-Z])/);
						if (camelParts.length > 1) {
							additionalTokens.push(...camelParts.map((p) => p.toLowerCase()));
						}
						// Also add substrings for partial matching
						if (token.length > 4) {
							// Add prefix substrings
							for (let i = 3; i < Math.min(token.length, 8); i++) {
								additionalTokens.push(token.slice(0, i));
							}
						}
					}
				}
				return [...tokens, ...additionalTokens].filter((t) => t.length > 0);
			},
		});

		const docMap = new Map<string, SearchResult>();
		for (const doc of documents) {
			docMap.set(doc.id, doc);
		}

		miniSearch.addAll(documents);
		cache.current = { board, documents, index: miniSearch, docMap };
		return cache.current;
	}, [board, enabled]);

	const search = useCallback(
		(query: string): SearchResult[] => {
			if (!built || !query.trim()) return [];
			const { index, docMap } = built;

			const results = index.search(query, {
				prefix: true,
				fuzzy: 0.3,
				combineWith: "OR",
			});

			// Deduplicate results by nodeId, keeping the highest scored one
			const seenNodes = new Map<
				string,
				{ score: number; result: SearchResult }
			>();

			for (const r of results) {
				const doc = docMap.get(r.id);
				if (!doc) continue;

				const key = `${doc.type}-${doc.nodeId}-${doc.pinName || ""}`;
				const existing = seenNodes.get(key);

				if (!existing || r.score > existing.score) {
					seenNodes.set(key, { score: r.score, result: doc });
				}
			}

			return Array.from(seenNodes.values())
				.sort((a, b) => b.score - a.score)
				.slice(0, 100)
				.map((item) => item.result);
		},
		[built],
	);

	return { search, totalDocuments: built?.documents.length ?? 0 };
}

function highlightMatch(text: string, query: string): React.ReactNode {
	if (!query.trim() || !text) return text;

	const queryTerms = query.toLowerCase().split(/\s+/).filter(Boolean);
	if (queryTerms.length === 0) return text;

	const regex = new RegExp(
		`(${queryTerms.map((t) => t.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")).join("|")})`,
		"gi",
	);
	const parts = text.split(regex);

	return parts.map((part, i) =>
		regex.test(part) ? (
			<mark key={i} className="bg-primary/20 text-primary rounded-sm px-0.5">
				{part}
			</mark>
		) : (
			part
		),
	);
}

const SearchResultItem = memo(
	({
		result,
		query,
		onSelect,
		isSelected,
	}: {
		result: SearchResult;
		query: string;
		onSelect: (result: SearchResult) => void;
		isSelected?: boolean;
	}) => {
		const icon = useMemo(() => {
			if (result.type === "layer") return <LayersIcon className="size-4" />;
			if (result.type === "pin" || result.type === "pin-value")
				return <VariableIcon className="size-4" />;
			if (result.type === "comment")
				return <MessageSquareTextIcon className="size-4" />;
			if (result.type === "variable")
				return <VariableIcon className="size-4 text-amber-500" />;
			return <FocusIcon className="size-4" />;
		}, [result.type]);

		const typeLabel = useMemo(() => {
			switch (result.type) {
				case "layer":
					return "Layer";
				case "pin":
					return "Pin";
				case "pin-value":
					return "Value";
				case "comment":
					return "Comment";
				case "variable":
					return "Variable";
				default:
					return "Node";
			}
		}, [result.type]);

		return (
			<CommandItem
				value={result.id}
				onSelect={() => onSelect(result)}
				className={cn(
					"flex items-start gap-3 py-2.5 cursor-pointer",
					isSelected && "bg-accent",
				)}
			>
				<div className="flex shrink-0 items-center justify-center mt-0.5 text-muted-foreground">
					{icon}
				</div>
				<div className="flex flex-col gap-1 overflow-hidden min-w-0 flex-1">
					<div className="flex items-center gap-2 flex-wrap">
						<span className="font-medium truncate">
							{highlightMatch(result.name, query)}
						</span>
						<Badge variant="secondary" className="text-[10px] px-1.5 py-0">
							{typeLabel}
						</Badge>
					</div>
					{result.layerPath && result.layerPath.length > 0 && (
						<div className="flex items-center gap-1 text-xs text-muted-foreground">
							<LayersIcon className="size-3 shrink-0" />
							<span className="truncate">{result.layerPath.join(" → ")}</span>
						</div>
					)}
					{result.type === "pin" && result.description && (
						<span className="text-xs text-muted-foreground truncate">
							{highlightMatch(result.description, query)}
						</span>
					)}
					{result.type === "pin-value" && (
						<span className="text-xs text-muted-foreground truncate">
							{highlightMatch(result.matchedValue, query)}
						</span>
					)}
					{result.type === "comment" && (
						<span className="text-xs text-muted-foreground truncate italic">
							"{highlightMatch(result.matchedValue, query)}"
						</span>
					)}
					{result.type === "variable" && result.dataType && (
						<span className="text-xs text-muted-foreground truncate">
							{result.dataType}
							{result.description && ` - ${result.description}`}
						</span>
					)}
					{result.category && (
						<span className="text-[10px] text-muted-foreground/70">
							{highlightMatch(result.category, query)}
						</span>
					)}
				</div>
				<TooltipProvider>
					<Tooltip>
						<TooltipTrigger asChild>
							<Button
								variant="ghost"
								size="icon"
								className="size-6 shrink-0"
								onClick={(e) => {
									e.stopPropagation();
									onSelect(result);
								}}
							>
								<FocusIcon className="size-3.5" />
							</Button>
						</TooltipTrigger>
						<TooltipContent side="left">{i18next.t('navigateToNode', 'Navigate to node')}</TooltipContent>
					</Tooltip>
				</TooltipProvider>
			</CommandItem>
		);
	},
);

SearchResultItem.displayName = "SearchResultItem";

const SidebarSearchResultItem = memo(
	({
		result,
		query,
		onSelect,
		isActive,
	}: {
		result: SearchResult;
		query: string;
		onSelect: (result: SearchResult) => void;
		isActive?: boolean;
	}) => {
		const icon = useMemo(() => {
			if (result.type === "layer")
				return <LayersIcon className="size-3.5 shrink-0" />;
			if (result.type === "pin" || result.type === "pin-value")
				return <VariableIcon className="size-3.5 shrink-0" />;
			if (result.type === "comment")
				return <MessageSquareTextIcon className="size-3.5 shrink-0" />;
			if (result.type === "variable")
				return <VariableIcon className="size-3.5 shrink-0" />;
			return <FocusIcon className="size-3.5 shrink-0" />;
		}, [result.type]);

		const typeColor = useMemo(() => {
			switch (result.type) {
				case "layer":
					return "text-blue-500";
				case "pin":
					return "text-purple-500";
				case "pin-value":
					return "text-amber-500";
				case "comment":
					return "text-green-500";
				case "variable":
					return "text-orange-500";
				default:
					return "text-foreground";
			}
		}, [result.type]);

		return (
			<button
				type="button"
				onClick={() => onSelect(result)}
				className={cn(
					"w-full text-left px-3 py-2 hover:bg-accent/50 transition-colors border-b border-border/50 last:border-b-0",
					isActive && "bg-accent",
				)}
			>
				<div className="flex items-start gap-2">
					<div className={cn("mt-0.5", typeColor)}>{icon}</div>
					<div className="flex-1 min-w-0 overflow-hidden">
						<div className="flex items-center gap-1.5">
							<span className="text-sm font-medium truncate">
								{highlightMatch(result.name, query)}
							</span>
						</div>
						{result.layerPath && result.layerPath.length > 0 && (
							<div className="flex items-center gap-1 text-[10px] text-muted-foreground mt-0.5">
								<LayersIcon className="size-2.5 shrink-0" />
								<span className="truncate">{result.layerPath.join(" / ")}</span>
							</div>
						)}
						{result.type === "pin" && result.description && (
							<div className="text-xs text-muted-foreground truncate mt-0.5">
								{highlightMatch(result.description, query)}
							</div>
						)}
						{result.type === "pin-value" && (
							<div className="text-xs text-muted-foreground truncate mt-0.5">
								{highlightMatch(result.matchedValue, query)}
							</div>
						)}
						{result.type === "comment" && (
							<div className="text-xs text-muted-foreground truncate mt-0.5 italic">
								"{highlightMatch(result.matchedValue, query)}"
							</div>
						)}
						{result.type === "variable" && (
							<div className="text-xs text-muted-foreground truncate mt-0.5">
								{result.dataType}
							</div>
						)}
						{result.category && (
							<div className="text-[10px] text-muted-foreground/70 truncate mt-0.5">
								{highlightMatch(result.category, query)}
							</div>
						)}
					</div>
				</div>
			</button>
		);
	},
);

SidebarSearchResultItem.displayName = "SidebarSearchResultItem";

export const FlowSearch = memo(
	({
		board,
		open,
		onOpenChange,
		onNavigate,
		mode = "dialog",
		onSwitchToSidebar,
	}: FlowSearchProps) => {
		const [query, setQuery] = useState("");
		const [selectedIndex, setSelectedIndex] = useState(0);
		const { search, totalDocuments } = useSearchIndex(board, open);
		const inputRef = useRef<HTMLInputElement>(null);

		const results = useMemo(() => search(query), [search, query]);

		const groupedResults = useMemo(() => {
			const groups: Record<SearchResult["type"], SearchResult[]> = {
				node: [],
				layer: [],
				pin: [],
				"pin-value": [],
				comment: [],
				variable: [],
			};
			for (const r of results) {
				groups[r.type]?.push(r);
			}
			return groups;
		}, [results]);

		const handleSelect = useCallback(
			(result: SearchResult) => {
				// Variables aren't nodes on the canvas, so we can't navigate to them
				if (result.type === "variable") {
					// Just close the search - user can use Variables panel
					if (mode === "dialog") {
						onOpenChange(false);
						setQuery("");
					}
					return;
				}

				onNavigate(result.nodeId);
				if (mode === "dialog") {
					onOpenChange(false);
					setQuery("");
				}
			},
			[onNavigate, onOpenChange, mode],
		);

		const handleOpenChange = useCallback(
			(open: boolean) => {
				onOpenChange(open);
				if (!open) {
					setQuery("");
					setSelectedIndex(0);
				}
			},
			[onOpenChange],
		);

		useEffect(() => {
			setSelectedIndex(0);
		}, [query]);

		useEffect(() => {
			if (open && mode === "sidebar" && inputRef.current) {
				inputRef.current.focus();
			}
		}, [open, mode]);

		const navigateResults = useCallback(
			(direction: "next" | "prev") => {
				if (results.length === 0) return;
				setSelectedIndex((prev) => {
					if (direction === "next") {
						return (prev + 1) % results.length;
					}
					return (prev - 1 + results.length) % results.length;
				});
			},
			[results.length],
		);

		const handleKeyDown = useCallback(
			(e: React.KeyboardEvent) => {
				if (e.key === "ArrowDown") {
					e.preventDefault();
					navigateResults("next");
				} else if (e.key === "ArrowUp") {
					e.preventDefault();
					navigateResults("prev");
				} else if (e.key === "Enter" && results[selectedIndex]) {
					e.preventDefault();
					handleSelect(results[selectedIndex]);
				} else if (e.key === "Escape") {
					handleOpenChange(false);
				}
			},
			[navigateResults, results, selectedIndex, handleSelect, handleOpenChange],
		);

		if (mode === "sidebar") {
			if (!open) return null;

			return (
				<div className="flex flex-col h-full border-l border-border bg-background">
					<div className="flex items-center gap-2 p-3 border-b border-border">
						<SearchIcon className="size-4 text-muted-foreground shrink-0" />
						<Input
							ref={inputRef}
							value={query}
							onChange={(e) => setQuery(e.target.value)}
							onKeyDown={handleKeyDown}
							placeholder={i18next.t('searchBoard', 'Search board...')}
							className="h-8 text-sm"
							autoFocus
						/>
						<Button
							variant="ghost"
							size="icon"
							className="size-8 shrink-0"
							onClick={() => handleOpenChange(false)}
						>
							<XIcon className="size-4" />
						</Button>
					</div>

					<div className="flex items-center justify-between px-3 py-1.5 text-xs text-muted-foreground border-b border-border">
						<span>
							{results.length > 0
								? i18next.t('countResults', {
										defaultValue_one: '{{count}} result',
										defaultValue_other: '{{count}} results',
										count: results.length,
									})
								: query
									? i18next.t('noResults', 'No results')
									: i18next.t('totaldocumentsItemsIndexed', '{{totalDocuments}} items indexed', { totalDocuments })}
						</span>
						{results.length > 0 && (
							<div className="flex items-center gap-1">
								<Button
									variant="ghost"
									size="icon"
									className="size-5"
									onClick={() => navigateResults("prev")}
									disabled={results.length === 0}
								>
									<ChevronLeftIcon className="size-3" />
								</Button>
								<span>
									{selectedIndex + 1}/{results.length}
								</span>
								<Button
									variant="ghost"
									size="icon"
									className="size-5"
									onClick={() => navigateResults("next")}
									disabled={results.length === 0}
								>
									<ChevronRightIcon className="size-3" />
								</Button>
							</div>
						)}
					</div>

					<div className="flex-1 overflow-y-auto">
						{!query && (
							<div className="p-4 text-center text-sm text-muted-foreground">
								{i18next.t('typeToSearchNodesLayersPinValuesAndComments', 'Type to search nodes, layers, pin values, and comments...')}
							</div>
						)}
						{query && results.length === 0 && (
							<div className="p-4 text-center text-sm text-muted-foreground">{i18next.t('noResultsFoundForQuery', 'No results found for "{{query}}"', { query })}</div>
						)}
						{results.map((result, index) => (
							<SidebarSearchResultItem
								key={result.id}
								result={result}
								query={query}
								onSelect={handleSelect}
								isActive={index === selectedIndex}
							/>
						))}
					</div>
				</div>
			);
		}

		return (
			<CommandDialog
				open={open}
				onOpenChange={handleOpenChange}
				title={i18next.t('searchBoard2', 'Search Board')}
				description={i18next.t('searchForNodesLayersPinValuesAndComments', 'Search for nodes, layers, pin values, and comments')}
				showCloseButton={false}
			>
				{/* Custom header with sidebar toggle and close button */}
				<div className="absolute right-2 top-2 flex items-center gap-1 z-10">
					{onSwitchToSidebar && (
						<TooltipProvider>
							<Tooltip>
								<TooltipTrigger asChild>
									<Button
										variant="ghost"
										size="icon"
										className="size-8 shrink-0"
										onClick={onSwitchToSidebar}
									>
										<PanelRightCloseIcon className="size-4" />
									</Button>
								</TooltipTrigger>
								<TooltipContent side="bottom">{i18next.t('openInSidebar', 'Open in sidebar')}</TooltipContent>
							</Tooltip>
						</TooltipProvider>
					)}
					<Button
						variant="ghost"
						size="icon"
						className="size-8 shrink-0"
						onClick={() => handleOpenChange(false)}
					>
						<XIcon className="size-4" />
					</Button>
				</div>
				<CommandInput
					placeholder={i18next.t('searchNodesLayersPinsVariables', 'Search nodes, layers, pins, variables...')}
					value={query}
					onValueChange={setQuery}
				/>
				<CommandList className="max-h-[400px]">
					{query && results.length === 0 && (
						<CommandEmpty>{i18next.t('noResultsFoundForQuery', 'No results found for "{{query}}"', { query })}</CommandEmpty>
					)}
					{!query && (
						<div className="py-6 text-center text-sm text-muted-foreground">
							<p>{i18next.t('typeToSearchNodesLayersPinsVariables', 'Type to search nodes, layers, pins, variables...')}</p>
							<p className="text-xs mt-1 text-muted-foreground/70">{i18next.t('totaldocumentsItemsIndexed', '{{totalDocuments}} items indexed', { totalDocuments })}</p>
						</div>
					)}
					{groupedResults.node.length > 0 && (
						<CommandGroup heading={i18next.t('nodesLength', 'Nodes ({{length}})', { length: groupedResults.node.length })}>
							{groupedResults.node.slice(0, 20).map((result) => (
								<SearchResultItem
									key={result.id}
									result={result}
									query={query}
									onSelect={handleSelect}
								/>
							))}
						</CommandGroup>
					)}
					{groupedResults.layer.length > 0 && (
						<CommandGroup heading={i18next.t('layersLength', 'Layers ({{length}})', { length: groupedResults.layer.length })}>
							{groupedResults.layer.slice(0, 10).map((result) => (
								<SearchResultItem
									key={result.id}
									result={result}
									query={query}
									onSelect={handleSelect}
								/>
							))}
						</CommandGroup>
					)}
					{groupedResults.pin.length > 0 && (
						<CommandGroup heading={i18next.t('pinsLength', 'Pins ({{length}})', { length: groupedResults.pin.length })}>
							{groupedResults.pin.slice(0, 15).map((result) => (
								<SearchResultItem
									key={result.id}
									result={result}
									query={query}
									onSelect={handleSelect}
								/>
							))}
						</CommandGroup>
					)}
					{groupedResults["pin-value"].length > 0 && (
						<CommandGroup
							heading={i18next.t('pinValuesLength', 'Pin Values ({{length}})', { length: groupedResults["pin-value"].length })}
						>
							{groupedResults["pin-value"].slice(0, 20).map((result) => (
								<SearchResultItem
									key={result.id}
									result={result}
									query={query}
									onSelect={handleSelect}
								/>
							))}
						</CommandGroup>
					)}
					{groupedResults.comment.length > 0 && (
						<CommandGroup
							heading={i18next.t('commentsLength', 'Comments ({{length}})', { length: groupedResults.comment.length })}
						>
							{groupedResults.comment.slice(0, 10).map((result) => (
								<SearchResultItem
									key={result.id}
									result={result}
									query={query}
									onSelect={handleSelect}
								/>
							))}
						</CommandGroup>
					)}
					{groupedResults.variable.length > 0 && (
						<CommandGroup
							heading={i18next.t('variablesLength', 'Variables ({{length}})', { length: groupedResults.variable.length })}
						>
							{groupedResults.variable.slice(0, 10).map((result) => (
								<SearchResultItem
									key={result.id}
									result={result}
									query={query}
									onSelect={handleSelect}
								/>
							))}
						</CommandGroup>
					)}
				</CommandList>
			</CommandDialog>
		);
	},
);

FlowSearch.displayName = "FlowSearch";
