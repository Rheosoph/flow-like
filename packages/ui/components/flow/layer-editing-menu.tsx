"use client";

import { Trans, i18n as i18next, useTranslation } from "@flow-like/locales";
import {
	DndContext,
	type DragEndEvent,
	PointerSensor,
	closestCenter,
	useSensor,
	useSensors,
} from "@dnd-kit/core";
import {
	SortableContext,
	arrayMove,
	useSortable,
	verticalListSortingStrategy,
} from "@dnd-kit/sortable";
import { CSS } from "@dnd-kit/utilities";
import { createId } from "@paralleldrive/cuid2";
import {
	ArrowDownIcon,
	ArrowUpIcon,
	DatabaseIcon,
	EllipsisVerticalIcon,
	GripIcon,
	GripVerticalIcon,
	InfoIcon,
	ListIcon,
	PlusIcon,
	SaveIcon,
	SlidersHorizontalIcon,
	Trash2Icon,
} from "lucide-react";
import {
	type RefObject,
	useCallback,
	useEffect,
	useMemo,
	useState,
} from "react";
import {
	type IPin,
	type IPinOptions,
	IValueType,
	IVariableType,
	describeCacheLifetime,
} from "../../lib";
import {
	type IBoard,
	type ILayer,
	type ILayerCache,
	ILayerCacheScope,
	ILayerType,
	IPinType,
} from "../../lib/schema/flow/board";
import type { INode } from "../../lib/schema/flow/node";
import {
	Button,
	Dialog,
	DialogContent,
	DialogDescription,
	DialogFooter,
	DialogHeader,
	DialogTitle,
	DropdownMenu,
	DropdownMenuContent,
	DropdownMenuItem,
	DropdownMenuTrigger,
	Input,
	Label,
	ScrollArea,
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
	Separator,
	Switch,
	Tabs,
	TabsContent,
	TabsList,
	TabsTrigger,
	Textarea,
} from "../ui";
import { typeToColor } from "./utils";
import { ValueTypeIcon } from "./variables/variables-menu";

export type PinEdit = {
	id: string;
	name: string;
	friendly_name: string;
	description: string;
	data_type: IVariableType;
	options?: IPinOptions | null;
	schema?: string | null;
	pin_type: IPinType;
	index: number;
	value_type: IValueType;
	default_value?: number[] | null;
};

function selectPreviewElement(type: IVariableType) {
	return (
		<div className="flex items-center gap-2">
			<div
				className={`size-2 rounded-full`}
				style={{ backgroundColor: typeToColor(type) }}
			/>
			<span>{type}</span>
		</div>
	);
}

const normalizeValueType = (vt: any): IValueType => {
	const s = String(vt ?? "").toLowerCase();
	if (s === "array") return IValueType.Array;
	if (s === "hashmap" || s === "map") return IValueType.HashMap;
	if (s === "hashset" || s === "set") return IValueType.HashSet;
	return IValueType.Normal;
};

const toMachineName = (s: string) =>
	s.trim().toLowerCase().replace(/\s+/g, "_");

const sortByIndex = <T extends { index: number }>(arr: T[]) =>
	[...arr].sort((a, b) => a.index - b.index);

const reindex = <T extends { index: number }>(arr: T[]) =>
	arr.map((p, i) => ({ ...p, index: i + 1 }));

const DEFAULT_LAYER_CACHE: ILayerCache = {
	enabled: false,
	prefix: "",
	ttl_seconds: null,
	scope: ILayerCacheScope.App,
};

const buildInitialCache = (layer?: ILayer): ILayerCache => ({
	...DEFAULT_LAYER_CACHE,
	...(layer?.cache ?? {}),
});

const useGroupedPins = (edits: Record<string, PinEdit>) => {
	return useMemo(() => {
		const all = Object.values(edits);
		const inputs = sortByIndex(
			all.filter((p) => p.pin_type === IPinType.Input),
		);
		const outputs = sortByIndex(
			all.filter((p) => p.pin_type === IPinType.Output),
		);
		return { inputs, outputs };
	}, [edits]);
};

const buildInitialEdits = (
	entity: ILayer | INode,
	boardRef?: RefObject<IBoard | undefined>,
): Record<string, PinEdit> => {
	const out: Record<string, PinEdit> = {};
	const isNodeMode = "friendly_name" in entity;
	const isGenericEvent =
		isNodeMode && (entity as INode).name === "events_generic";

	for (const pin of Object.values(entity.pins)) {
		const p: any = pin;
		const friendly = p?.friendly_name ?? p?.name ?? pin.id;
		let description = p?.description ?? "";
		let schema = p?.schema ?? null;

		// Resolve description ref if it's a hash
		const descRef = boardRef?.current?.refs?.[description];
		if (descRef) {
			description = descRef;
		}

		// Resolve schema ref if it's a hash
		if (schema && typeof schema === "string") {
			const schemaRef = boardRef?.current?.refs?.[schema];
			if (schemaRef) {
				schema = schemaRef;
			}
		}

		// Hash for empty string, empty string will not be taken into the refs.
		if (description === "16248035215404677707") {
			description = "";
		}

		// Override description for payload pin on generic_event nodes
		if (isGenericEvent && p?.name === "payload") {
			description = i18next.t('catchAllForAdditionalMetadata', '(Catch all for additional metadata)');
		}

		out[pin.id] = {
			id: pin.id,
			name: toMachineName(friendly),
			friendly_name: friendly,
			description: description,
			data_type: p?.data_type ?? IVariableType.Generic,
			options: p?.options ?? null,
			schema: schema,
			pin_type: pin.pin_type,
			index: pin.index ?? 1,
			value_type: normalizeValueType(p?.value_type ?? p?.valueType),
			default_value: p?.default_value ?? null,
		};
	}
	return out;
};

export interface IPinEditor {
	edits: Record<string, PinEdit>;
	inputs: PinEdit[];
	outputs: PinEdit[];
	reset: () => void;
	editPin: (id: string, patch: Partial<PinEdit>) => void;
	addPin: (pin_type: IPinType) => void;
	removePin: (id: string) => void;
	movePin: (id: string, dir: "up" | "down") => void;
	reorderByIds: (orderedIds: string[]) => void;
	/** Serializes the edits back onto the entity's pin map, re-indexed per group. */
	buildPins: () => Record<string, IPin>;
}

/**
 * The pin editing state machine, shared by the layer/node dialog and the function
 * overlay so both surfaces stay on one pin model.
 */
export function usePinEditor(
	entity: ILayer | INode | undefined,
	boardRef?: RefObject<IBoard | undefined>,
): IPinEditor {
	const isGenericEvent =
		entity !== undefined &&
		"friendly_name" in entity &&
		(entity as INode).name === "events_generic";

	const [edits, setEdits] = useState<Record<string, PinEdit>>(() =>
		entity ? buildInitialEdits(entity, boardRef) : {},
	);
	const { inputs, outputs } = useGroupedPins(edits);

	const reset = useCallback(() => {
		setEdits(entity ? buildInitialEdits(entity, boardRef) : {});
	}, [entity, boardRef]);

	const patchPin = useCallback(
		(id: string, updater: (p: PinEdit) => PinEdit) => {
			setEdits((prev) => {
				const curr = prev[id];
				if (!curr) return prev;
				return { ...prev, [id]: updater(curr) };
			});
		},
		[],
	);

	const editPin = useCallback(
		(id: string, patch: Partial<PinEdit>) => {
			patchPin(id, (p) => {
				const next: PinEdit = { ...p, ...patch };
				if (patch.friendly_name !== undefined) {
					next.name = toMachineName(patch.friendly_name);
				}
				return next;
			});
		},
		[patchPin],
	);

	const reindexGroup = useCallback((group: PinEdit[]) => {
		const re = reindex(group);
		setEdits((prev) => {
			const copy = { ...prev };
			for (const p of re) copy[p.id] = { ...copy[p.id], index: p.index };
			return copy;
		});
	}, []);

	const movePin = useCallback(
		(id: string, dir: "up" | "down") => {
			const group = edits[id]?.pin_type === IPinType.Input ? inputs : outputs;
			const idx = group.findIndex((p) => p.id === id);
			if (idx < 0) return;

			const nextIdx = dir === "up" ? idx - 1 : idx + 1;
			if (nextIdx < 0 || nextIdx >= group.length) return;

			const swapped = [...group];
			[swapped[idx], swapped[nextIdx]] = [swapped[nextIdx], swapped[idx]];
			reindexGroup(swapped);
		},
		[edits, inputs, outputs, reindexGroup],
	);

	const addPin = useCallback((pin_type: IPinType) => {
		setEdits((prev) => {
			const id = createId();
			const group = Object.values(prev).filter((p) => p.pin_type === pin_type);
			const next: PinEdit = {
				id,
				name: "new_pin",
				friendly_name: "New Pin",
				description: "",
				data_type: IVariableType.Generic,
				options: null,
				schema: null,
				pin_type,
				index: group.length + 1,
				value_type: IValueType.Normal,
				default_value: null,
			};
			return { ...prev, [id]: next };
		});
	}, []);

	const removePin = useCallback(
		(id: string) => {
			setEdits((prev) => {
				const pin = prev[id];
				if (!pin) return prev;

				// The payload pin is the contract of a generic event — it cannot be removed.
				if (isGenericEvent && pin.name === "payload") {
					return prev;
				}

				const copy = { ...prev };
				delete copy[id];

				const remaining = Object.values(copy).filter(
					(p) => p.pin_type === pin.pin_type,
				);
				const re = reindex(sortByIndex(remaining));
				for (const p of re) copy[p.id] = { ...copy[p.id], index: p.index };
				return copy;
			});
		},
		[isGenericEvent],
	);

	const reorderByIds = useCallback((orderedIds: string[]) => {
		setEdits((prev) => {
			const copy = { ...prev };
			orderedIds.forEach((id, i) => {
				if (copy[id]) copy[id] = { ...copy[id], index: i + 1 };
			});
			return copy;
		});
	}, []);

	const buildPins = useCallback((): Record<string, IPin> => {
		const original = (entity?.pins ?? {}) as Record<string, IPin>;
		const nextPins: Record<string, IPin> = {};
		const zeroIndexed = Object.values(edits).find((p) => p.index <= 0);

		for (const edit of Object.values(edits)) {
			const prev = original[edit.id];

			nextPins[edit.id] = {
				...(prev as IPin),
				id: edit.id,
				pin_type: edit.pin_type,
				index: zeroIndexed ? (edit.index ?? 0) + 1 : (edit.index ?? 1),
				connected_to: prev?.connected_to ?? [],
				depends_on: prev?.depends_on ?? [],
				default_value: edit.default_value ?? null,
				data_type: edit.data_type,
				description: edit.description ?? "",
				friendly_name: edit.friendly_name ?? edit.name,
				name: toMachineName(edit.friendly_name ?? edit.name),
				options: edit.options ?? null,
				schema: edit.schema ?? null,
				value_type: edit.value_type ?? IValueType.Normal,
			};
		}

		const nextInputs = reindex(
			sortByIndex(
				Object.values(nextPins).filter((p) => p.pin_type === IPinType.Input),
			),
		);
		const nextOutputs = reindex(
			sortByIndex(
				Object.values(nextPins).filter((p) => p.pin_type === IPinType.Output),
			),
		);
		for (const p of nextInputs)
			nextPins[p.id] = { ...nextPins[p.id], index: p.index };
		for (const p of nextOutputs)
			nextPins[p.id] = { ...nextPins[p.id], index: p.index };

		return nextPins;
	}, [edits, entity]);

	return {
		edits,
		inputs,
		outputs,
		reset,
		editPin,
		addPin,
		removePin,
		movePin,
		reorderByIds,
		buildPins,
	};
}

interface LayerEditMenuProps {
	open: boolean;
	onOpenChange: (open: boolean) => void;
	layer?: ILayer;
	node?: INode;
	onApply: (updated: ILayer | INode) => Promise<void>;
	boardRef?: RefObject<IBoard | undefined>;
	mode?: "layer" | "node";
}

export const LayerEditMenu: React.FC<LayerEditMenuProps> = ({
	open,
	onOpenChange,
	layer,
	node,
	onApply,
	boardRef,
	mode = "layer",
}) => {
	const { t } = useTranslation("flow");
	const entity = layer ?? node;
	const isNodeMode =
		mode === "node" || (node !== undefined && layer === undefined);
	const isGenericEvent = isNodeMode && node?.name === "events_generic";
	// Only function layers have a call boundary to cache at — a collapsed layer runs
	// inline with the rest of the graph.
	const isFunctionLayer = !isNodeMode && layer?.type === ILayerType.Function;

	const {
		inputs,
		outputs,
		reset: resetPins,
		editPin,
		addPin,
		removePin,
		movePin,
		reorderByIds,
		buildPins,
	} = usePinEditor(entity, boardRef);
	const [nodeName, setNodeName] = useState<string>("");
	const [nodeDescription, setNodeDescription] = useState<string>("");
	const [cache, setCache] = useState<ILayerCache>(() =>
		buildInitialCache(layer),
	);
	const [tab, setTab] = useState<"inputs" | "outputs" | "metadata" | "caching">(
		isGenericEvent ? "metadata" : "inputs",
	);

	useEffect(() => {
		if (open && entity) {
			resetPins();
			setCache(buildInitialCache(layer));
			setTab(isGenericEvent ? "metadata" : "inputs");
			if (isNodeMode && node) {
				setNodeName(node.friendly_name || "");
				// Resolve node description ref if it's a hash
				let desc = node.description || "";
				const descRef = boardRef?.current?.refs?.[desc];
				if (descRef) {
					desc = descRef;
				}
				// Hash for empty string
				if (desc === "16248035215404677707") {
					desc = "";
				}
				setNodeDescription(desc);
			}
		}
	}, [open, entity, boardRef, isNodeMode, node, isGenericEvent, layer, resetPins]);

	const applyChanges = useCallback(async () => {
		if (!entity) return;

		const nextPins = buildPins();

		if (isNodeMode && node) {
			const updated: INode = {
				...node,
				pins: nextPins as unknown as INode["pins"],
				friendly_name: nodeName || node.friendly_name,
				description: nodeDescription || node.description,
			};
			await onApply(updated);
		} else if (layer) {
			const updated: ILayer = {
				...layer,
				pins: nextPins as unknown as ILayer["pins"],
				// Collapsed layers never reach a call boundary, so they must not carry a
				// setting that silently does nothing.
				cache: isFunctionLayer
					? { ...cache, prefix: (cache.prefix ?? "").trim() }
					: null,
			};
			await onApply(updated);
		}
	}, [
		buildPins,
		entity,
		layer,
		node,
		onApply,
		isNodeMode,
		nodeName,
		nodeDescription,
		cache,
		isFunctionLayer,
	]);

	return (
		<Dialog open={open} onOpenChange={onOpenChange}>
			<DialogContent
				className="sm:max-w-5xl"
				onDoubleClick={(e) => e.stopPropagation()}
			>
				<DialogHeader>
					<DialogTitle className="flex items-center gap-2">
						<SlidersHorizontalIcon className="h-5 w-5 text-primary" />
						{isNodeMode ? t('editNodeProperties', 'Edit Node Properties') : t('editLayerPins', 'Edit Layer Pins')}
					</DialogTitle>
					<DialogDescription>
						{isNodeMode
							? t('configurePropertiesAndPinsForVal', 'Configure properties and pins for "{{val}}".', { val: node?.friendly_name || "Node" })
							: t('configurePinPropertiesAndOrderingForVal', 'Configure pin properties and ordering for "{{val}}".', { val: layer?.name || "Layer" })}
					</DialogDescription>
				</DialogHeader>
				<Tabs
					value={tab}
					onValueChange={(v) => setTab(v as any)}
					className="mt-2"
				>
					<TabsList
						className={`grid w-full ${isGenericEvent ? "grid-cols-2" : isNodeMode ? "grid-cols-1" : isFunctionLayer ? "grid-cols-3" : "grid-cols-2"}`}
					>
						{isGenericEvent && (
							<TabsTrigger value="metadata">{t('nodeInfo', 'Node Info')}</TabsTrigger>
						)}
						{!isNodeMode && <TabsTrigger value="inputs">{t('inputs', 'Inputs')}</TabsTrigger>}
						<TabsTrigger value="outputs">{t('outputs', 'Outputs')}</TabsTrigger>
						{isFunctionLayer && (
							<TabsTrigger value="caching" className="gap-1.5">
								<DatabaseIcon className="h-3.5 w-3.5" />
								{t('caching', 'Caching')}
							</TabsTrigger>
						)}
					</TabsList>
					{isGenericEvent && (
						<TabsContent value="metadata" className="mt-3 space-y-4">
							<div className="space-y-2">
								<label className="text-sm font-medium">{t('nodeName', 'Node Name')}</label>
								<Input
									value={nodeName}
									onChange={(e) => setNodeName(e.target.value)}
									placeholder={t('enterNodeName', 'Enter node name')}
								/>
							</div>
							<div className="space-y-2">
								<label className="text-sm font-medium">{t('nodeDescription', 'Node Description')}</label>
								<Textarea
									value={nodeDescription}
									onChange={(e) => setNodeDescription(e.target.value)}
									placeholder={t('enterNodeDescription', 'Enter node description')}
									rows={4}
								/>
							</div>
						</TabsContent>
					)}
					<TabsContent value="inputs" className="mt-3 space-y-2">
						<div className="flex justify-end">
							<Button
								size="sm"
								onClick={() => addPin(IPinType.Input)}
								className="gap-2"
							>
								<PlusIcon className="h-4 w-4" />
								{t('addInputPin', 'Add Input Pin')}
							</Button>
						</div>
						<PinList
							items={inputs}
							onEdit={editPin}
							onMoveUp={(id) => movePin(id, "up")}
							onMoveDown={(id) => movePin(id, "down")}
							onRemove={removePin}
							onReorder={reorderByIds}
							isGenericEvent={isGenericEvent}
						/>
					</TabsContent>{" "}
					<TabsContent value="outputs" className="mt-3 space-y-2">
						<div className="flex justify-end">
							<Button
								size="sm"
								onClick={() => addPin(IPinType.Output)}
								className="gap-2"
							>
								<PlusIcon className="h-4 w-4" />
								{t('addOutputPin', 'Add Output Pin')}
							</Button>
						</div>
						<PinList
							items={outputs}
							onEdit={editPin}
							onMoveUp={(id) => movePin(id, "up")}
							onMoveDown={(id) => movePin(id, "down")}
							onRemove={removePin}
							onReorder={reorderByIds}
							isGenericEvent={isGenericEvent}
						/>
					</TabsContent>
					{isFunctionLayer && (
						<TabsContent value="caching" className="mt-3">
							<CacheSettings cache={cache} onChange={setCache} />
						</TabsContent>
					)}
				</Tabs>{" "}
				<Separator className="my-3" />
				<DialogFooter className="gap-2">
					<Button variant="secondary" onClick={() => onOpenChange(false)}>
						{t('close', 'Close')}
					</Button>
					<Button onClick={applyChanges} className="gap-2">
						<SaveIcon className="h-4 w-4" />
						{t('save', 'Save')}
					</Button>
				</DialogFooter>
			</DialogContent>
		</Dialog>
	);
};

export const CacheSettings: React.FC<{
	cache: ILayerCache;
	onChange: (next: ILayerCache) => void;
}> = ({ cache, onChange }) => {
	const { t } = useTranslation("flow");
	const enabled = Boolean(cache.enabled);
	const patch = useCallback(
		(next: Partial<ILayerCache>) => onChange({ ...cache, ...next }),
		[cache, onChange],
	);

	return (
		<div className="space-y-4">
			<div className="flex items-start justify-between gap-4 rounded-md border p-3">
				<div className="space-y-1">
					<Label htmlFor="layer-cache-enabled" className="text-sm">
						{t('cacheResults', 'Cache results')}
					</Label>
					<p className="text-xs text-muted-foreground">
						{t('reuseAPreviousResultWheneverThisLayerIsCalledWithTheSameInputs', "Reuse a previous result whenever this layer is called with the same inputs.")}
					</p>
				</div>
				<Switch
					id="layer-cache-enabled"
					checked={enabled}
					onCheckedChange={(checked) => patch({ enabled: checked })}
				/>
			</div>

			<div
				className={`space-y-4 ${enabled ? "" : "pointer-events-none opacity-50"}`}
			>
				<div className="space-y-1.5">
					<Label className="text-xs">{t('prefix', 'Prefix')}</Label>
					<Input
						className="h-8"
						value={cache.prefix ?? ""}
						onChange={(e) => patch({ prefix: e.target.value })}
						placeholder={t('egPricing', 'e.g. pricing')}
						disabled={!enabled}
					/>
					<p className="text-xs text-muted-foreground">
						{t('groupsThisLayersEntriesSoTheyCanBeClearedInOneGoWithTheInvalidateCacheNamespaceNodeWithoutAPrefixTheyCanOnlyBeDeletedOneKeyAtATimeChangingItAlsoDropsEverythingCachedSoFar', "Groups this layer's entries so they can be cleared in one go with the Invalidate Cache Namespace node — without a prefix they can only be deleted one key at a time. Changing it also drops everything cached so far.")}
					</p>
				</div>

				<div className="grid grid-cols-1 gap-4 md:grid-cols-2">
					<div className="space-y-1.5">
						<Label className="text-xs">{t('lifetimeSeconds', 'Lifetime (seconds)')}</Label>
						<Input
							className="h-8"
							type="number"
							min={0}
							value={cache.ttl_seconds ?? ""}
							onChange={(e) =>
								patch({
									ttl_seconds:
										e.target.value === ""
											? null
											: Math.max(0, Number(e.target.value)),
								})
							}
							placeholder="0"
							disabled={!enabled}
						/>
						<p className="text-xs text-muted-foreground">
							{describeCacheLifetime(cache.ttl_seconds)}
						</p>
					</div>

					<div className="space-y-1.5">
						<Label className="text-xs">{t('scope', 'Scope')}</Label>
						<Select
							value={cache.scope ?? ILayerCacheScope.App}
							onValueChange={(value) =>
								patch({ scope: value as ILayerCacheScope })
							}
							disabled={!enabled}
						>
							<SelectTrigger className="h-8 w-full text-xs">
								<SelectValue placeholder="Scope" />
							</SelectTrigger>
							<SelectContent>
								<SelectItem value={ILayerCacheScope.App}>
									{t('appSharedByEveryone', 'App — shared by everyone')}
								</SelectItem>
								<SelectItem value={ILayerCacheScope.User}>
									{t('userPrivatePerUser', 'User — private per user')}
								</SelectItem>
							</SelectContent>
						</Select>
						<p className="text-xs text-muted-foreground">
							{t('pickUserWhenTheResultDependsOnWhoTriggeredTheRun', 'Pick User when the result depends on who triggered the run.')}
						</p>
					</div>
				</div>

				{enabled && (
					<div className="flex gap-2 rounded-md border border-destructive/40 bg-destructive/10 p-3 text-xs">
						<InfoIcon className="h-4 w-4 shrink-0 text-destructive" />
						<span className="text-muted-foreground">
							{t('aCacheHitReplacesTheWholeCallTheLayersNodesNeverRunSoAnySideEffectsTheyHaveAreSkippedTooOnlyCacheLayersWhoseOutputsDependSolelyOnTheirInputs', "A cache hit replaces the whole call — the layer's nodes never run, so any side effects they have are skipped too. Only cache layers whose outputs depend solely on their inputs.")}
						</span>
					</div>
				)}
			</div>
		</div>
	);
};

// Helpers
const toCSV = (arr?: string[] | null) =>
	arr && arr.length > 0 ? arr.join(", ") : "";
const fromCSVStrings = (s: string): string[] =>
	s
		.split(",")
		.map((x) => x.trim())
		.filter((x) => x.length > 0);

interface PinListProps {
	items: PinEdit[];
	onEdit: (id: string, patch: Partial<PinEdit>) => void;
	onMoveUp: (id: string) => void;
	onMoveDown: (id: string) => void;
	onRemove: (id: string) => void;
	onReorder: (orderedIds: string[]) => void;
	isGenericEvent?: boolean;
}

const PinValueTypeDropdown: React.FC<{
	value_type: IValueType;
	data_type: IVariableType;
	onChange: (vt: IValueType) => void;
	className?: string;
}> = ({ value_type, data_type, onChange, className }) => {
	const { t } = useTranslation("flow");
	const color = typeToColor(data_type);
	return (
		<DropdownMenu>
			<DropdownMenuTrigger asChild>
				<button
					type="button"
					className={`inline-flex items-center justify-center rounded p-1 hover:bg-accent/50 ${className ?? ""}`}
					aria-label={t('changeValueType', 'Change value type')}
				>
					<ValueTypeIcon value_type={value_type} data_type={data_type} />
				</button>
			</DropdownMenuTrigger>
			<DropdownMenuContent align="start">
				<DropdownMenuItem
					className="gap-2"
					onClick={() => onChange(IValueType.Normal)}
				><Trans i18nKey="divClassnamew4H2RoundedfullStyleBackgroundcolorColorSingle"><div
						className="w-4 h-2 rounded-full"
						style={{ backgroundColor: color }}
					/>
					Single</Trans></DropdownMenuItem>
				<DropdownMenuItem
					className="gap-2"
					onClick={() => onChange(IValueType.Array)}
				>
					<GripIcon className="w-4 h-4" style={{ color }} />
					{t('array', 'Array')}
				</DropdownMenuItem>
				<DropdownMenuItem
					className="gap-2"
					onClick={() => onChange(IValueType.HashSet)}
				>
					<EllipsisVerticalIcon className="w-4 h-4" style={{ color }} />
					{t('set', 'Set')}
				</DropdownMenuItem>
				<DropdownMenuItem
					className="gap-2"
					onClick={() => onChange(IValueType.HashMap)}
				>
					<ListIcon className="w-4 h-4" style={{ color }} />
					{t('map', 'Map')}
				</DropdownMenuItem>
			</DropdownMenuContent>
		</DropdownMenu>
	);
};

const PinDataTypeSelectInline: React.FC<{
	value: IVariableType;
	onChange: (dt: IVariableType) => void;
	className?: string;
}> = ({ value, onChange, className }) => {
	const { t } = useTranslation("flow");
	return (
		<Select
			value={value}
			onValueChange={(val) => onChange(val as IVariableType)}
		>
			<SelectTrigger className={`h-7 w-[140px] text-xs ${className ?? ""}`}>
				<SelectValue placeholder={t('dataType', 'Data Type')} />
			</SelectTrigger>
			<SelectContent>
				<SelectItem value={IVariableType.Boolean}>
					{selectPreviewElement(IVariableType.Boolean)}
				</SelectItem>
				<SelectItem value={IVariableType.Byte}>
					{selectPreviewElement(IVariableType.Byte)}
				</SelectItem>
				<SelectItem value={IVariableType.Date}>
					{selectPreviewElement(IVariableType.Date)}
				</SelectItem>
				<SelectItem value={IVariableType.Execution}>
					{selectPreviewElement(IVariableType.Execution)}
				</SelectItem>
				<SelectItem value={IVariableType.Float}>
					{selectPreviewElement(IVariableType.Float)}
				</SelectItem>
				<SelectItem value={IVariableType.Generic}>
					{selectPreviewElement(IVariableType.Generic)}
				</SelectItem>
				<SelectItem value={IVariableType.Integer}>
					{selectPreviewElement(IVariableType.Integer)}
				</SelectItem>
				<SelectItem value={IVariableType.PathBuf}>
					{selectPreviewElement(IVariableType.PathBuf)}
				</SelectItem>
				<SelectItem value={IVariableType.String}>
					{selectPreviewElement(IVariableType.String)}
				</SelectItem>
				<SelectItem value={IVariableType.Struct}>
					{selectPreviewElement(IVariableType.Struct)}
				</SelectItem>
			</SelectContent>
		</Select>
	);
};

export const PinList: React.FC<PinListProps> = ({
	items,
	onEdit,
	onMoveUp,
	onMoveDown,
	onRemove,
	onReorder,
	isGenericEvent = false,
}) => {
	const { t } = useTranslation("flow");
	const sensors = useSensors(
		useSensor(PointerSensor, { activationConstraint: { distance: 6 } }),
	);
	const ids = useMemo(() => items.map((p) => p.id), [items]);

	const handleDragEnd = useCallback(
		(e: DragEndEvent) => {
			const { active, over } = e;
			if (!over || active.id === over.id) return;
			const from = ids.indexOf(String(active.id));
			const to = ids.indexOf(String(over.id));
			if (from === -1 || to === -1) return;
			const next = arrayMove(ids, from, to);
			onReorder(next);
		},
		[ids, onReorder],
	);

	return (
		<ScrollArea className="h-96 max-h-96 overflow-auto rounded-md border">
			<div className="p-2 space-y-2">
				{items.length === 0 && (
					<div className="text-sm text-muted-foreground px-2 py-8 text-center">
						{t('noPinsInThisGroup', 'No pins in this group.')}
					</div>
				)}
				<DndContext
					sensors={sensors}
					collisionDetection={closestCenter}
					onDragEnd={handleDragEnd}
				>
					<SortableContext items={ids} strategy={verticalListSortingStrategy}>
						{items.map((pin, idx) => (
							<SortablePinRow
								key={pin.id}
								pin={pin}
								idx={idx}
								total={items.length}
								onEdit={onEdit}
								onMoveUp={onMoveUp}
								onMoveDown={onMoveDown}
								onRemove={onRemove}
								isGenericEvent={isGenericEvent}
							/>
						))}
					</SortableContext>
				</DndContext>
			</div>
		</ScrollArea>
	);
};

const SortablePinRow: React.FC<{
	pin: PinEdit;
	idx: number;
	total: number;
	onEdit: (id: string, patch: Partial<PinEdit>) => void;
	onMoveUp: (id: string) => void;
	onMoveDown: (id: string) => void;
	onRemove: (id: string) => void;
	isGenericEvent?: boolean;
}> = ({
	pin,
	idx,
	total,
	onEdit,
	onMoveUp,
	onMoveDown,
	onRemove,
	isGenericEvent = false,
}) => {
	const { t } = useTranslation("flow");
	const {
		attributes,
		listeners,
		setNodeRef,
		transform,
		transition,
		isDragging,
	} = useSortable({
		id: pin.id,
	});
	const style: React.CSSProperties = {
		transform: transform ? CSS.Transform.toString(transform) : undefined,
		transition,
	};

	const [expanded, setExpanded] = useState(false);
	const [editingName, setEditingName] = useState(false);
	const [nameDraft, setNameDraft] = useState(pin.friendly_name);

	useEffect(() => {
		if (!editingName) setNameDraft(pin.friendly_name);
	}, [pin.friendly_name, editingName]);

	const commitName = useCallback(() => {
		const next = (nameDraft ?? "").trim();
		onEdit(pin.id, { friendly_name: next === "" ? pin.friendly_name : next });
		setEditingName(false);
	}, [nameDraft, onEdit, pin.id, pin.friendly_name]);

	const cancelName = useCallback(() => {
		setNameDraft(pin.friendly_name);
		setEditingName(false);
	}, [pin.friendly_name]);

	return (
		<div
			ref={setNodeRef}
			style={style}
			className={`group rounded-md border bg-card hover:bg-accent/30 transition-colors ${isDragging ? "opacity-60" : ""}`}
			onDoubleClick={(e) => e.stopPropagation()}
		>
			<div className="flex items-center gap-2 px-2 py-1 border-b">
				<button
					type="button"
					title={t('dragToReorder', 'Drag to reorder')}
					className="inline-flex h-5 w-5 items-center justify-center text-muted-foreground cursor-grab active:cursor-grabbing"
					{...attributes}
					{...listeners}
					onClick={(e) => e.stopPropagation()}
				>
					<GripVerticalIcon className="h-4 w-4" />
				</button>
				<PinValueTypeDropdown
					value_type={pin.value_type}
					data_type={pin.data_type}
					onChange={(vt) => onEdit(pin.id, { value_type: vt })}
					className="shrink-0"
				/>
				<button
					type="button"
					aria-expanded={expanded}
					onClick={() => setExpanded((v) => !v)}
					className="px-1 text-xs text-muted-foreground hover:text-foreground"
					title={expanded ? "Collapse" : "Expand"}
				>
					{expanded ? "▾" : "▸"}
				</button>
				<div className="flex-1 truncate text-sm font-medium">
					{editingName ? (
						<Input
							className="h-7"
							autoFocus
							value={nameDraft}
							onChange={(e) => setNameDraft(e.target.value)}
							onBlur={commitName}
							onKeyDown={(e) => {
								if (e.key === "Enter") {
									commitName();
								}
								if (e.key === "Escape") {
									e.preventDefault();
									e.stopPropagation();
									cancelName();
								}
							}}
						/>
					) : (
						<button
							type="button"
							className="text-left truncate w-full"
							onClick={(e) => {
								e.stopPropagation();
								// Prevent editing payload pin name on generic_event nodes
								if (!(isGenericEvent && pin.name === "payload")) {
									setEditingName(true);
								}
							}}
							title={
								isGenericEvent && pin.name === "payload"
									? t('payloadPinCannotBeRenamed', 'Payload pin cannot be renamed')
									: t('clickToRename', 'Click to rename')
							}
						>
							{pin.friendly_name ?? pin.name ?? pin.id}
						</button>
					)}
					<span className="ml-2 text-[10px] text-muted-foreground">{`(${pin.id})`}</span>
				</div>{" "}
				<PinDataTypeSelectInline
					value={pin.data_type}
					onChange={(dt) => onEdit(pin.id, { data_type: dt })}
					className="hidden sm:flex"
				/>
				<Button
					variant="ghost"
					size="icon"
					onClick={() => onMoveUp(pin.id)}
					disabled={idx === 0}
					title={t('moveUp', 'Move up')}
				>
					<ArrowUpIcon className="h-4 w-4" />
				</Button>
				<Button
					variant="ghost"
					size="icon"
					onClick={() => onMoveDown(pin.id)}
					disabled={idx === total - 1}
					title={t('moveDown', 'Move down')}
				>
					<ArrowDownIcon className="h-4 w-4" />
				</Button>
				<Button
					variant="destructive"
					size="icon"
					onClick={() => onRemove(pin.id)}
					disabled={isGenericEvent && pin.name === "payload"}
					title={
						isGenericEvent && pin.name === "payload"
							? t('payloadPinCannotBeRemoved', 'Payload pin cannot be removed')
							: t('removePin', 'Remove pin')
					}
				>
					<Trash2Icon className="h-4 w-4 text-destructive-foreground" />
				</Button>
				<Button
					variant="ghost"
					size="icon"
					onClick={() =>
						(
							document.getElementById(`opts-${pin.id}`) as HTMLButtonElement
						)?.click()
					}
					title="Options"
				>
					<SlidersHorizontalIcon className="h-4 w-4" />
				</Button>
			</div>
			{expanded && (
				<div className="p-2 space-y-3">
					<div className="space-y-1.5">
						<Label className="text-xs">{t('description', 'Description')}</Label>
						<Input
							className="h-8"
							value={pin.description}
							onChange={(e) => onEdit(pin.id, { description: e.target.value })}
							placeholder="Optional"
							disabled={isGenericEvent && pin.name === "payload"}
							title={
								isGenericEvent && pin.name === "payload"
									? t('payloadPinDescriptionCannotBeEdited', 'Payload pin description cannot be edited')
									: undefined
							}
						/>
					</div>
				</div>
			)}{" "}
			<div className="sr-only">
				<PinOptionsButton
					pin={pin}
					onApply={(opts) => onEdit(pin.id, { options: opts })}
					onSchemaChange={(schema) => onEdit(pin.id, { schema })}
				/>
			</div>
		</div>
	);
};

interface PinOptionsButtonProps {
	pin: PinEdit;
	onApply: (opts: IPinOptions | null) => void;
	onSchemaChange: (schema: string | null) => void;
}

const PinOptionsButton: React.FC<PinOptionsButtonProps> = ({
	pin,
	onApply,
	onSchemaChange,
}) => {
	const { t } = useTranslation("flow");
	const [open, setOpen] = useState(false);
	const [local, setLocal] = useState<IPinOptions | null>(pin.options ?? null);
	const [localSchema, setLocalSchema] = useState<string>(pin.schema ?? "");

	useEffect(() => {
		if (open) {
			setLocal(pin.options ?? null);
			setLocalSchema(pin.schema ?? "");
		}
	}, [open, pin.options, pin.schema]);

	return (
		<>
			<Button
				id={`opts-${pin.id}`}
				variant="outline"
				size="sm"
				className="h-7 px-2"
				onClick={() => setOpen(true)}
				title={t('editPinOptions', 'Edit pin options')}
			>
				{t('options', 'Options…')}
			</Button>
			<Dialog open={open} onOpenChange={setOpen}>
				<DialogContent
					className="sm:max-w-lg"
					onDoubleClick={(e) => e.stopPropagation()}
				>
					<DialogHeader>
						<DialogTitle>{t('pinOptionsFriendly_name', 'Pin Options — {{friendly_name}}', { friendly_name: pin.friendly_name })}</DialogTitle>
						<DialogDescription>{t('advancedOptionalSettings', 'Advanced, optional settings.')}</DialogDescription>
					</DialogHeader>

					<div className="grid grid-cols-1 md:grid-cols-6 gap-3">
						<div className="space-y-1.5 md:col-span-6">
							<Label className="text-xs">{t('schema', 'Schema')}</Label>
							<Input
								className="h-8"
								value={localSchema}
								onChange={(e) => setLocalSchema(e.target.value)}
								placeholder={t('egMyschemaidentifier', 'e.g. my.schema.Identifier')}
							/>
						</div>

						<div className="flex items-center gap-2 md:col-span-3">
							<input
								id={`opt-egvt-${pin.id}`}
								type="checkbox"
								className="h-4 w-4"
								checked={Boolean(local?.enforce_generic_value_type)}
								onChange={(e) =>
									setLocal({
										...(local ?? {}),
										enforce_generic_value_type: e.target.checked,
									} as IPinOptions)
								}
							/>
							<Label htmlFor={`opt-egvt-${pin.id}`} className="text-xs">
								{t('enforceGenericVt', 'Enforce Generic VT')}
							</Label>
						</div>

						<div className="flex items-center gap-2 md:col-span-3">
							<input
								id={`opt-es-${pin.id}`}
								type="checkbox"
								className="h-4 w-4"
								checked={Boolean(local?.enforce_schema)}
								onChange={(e) =>
									setLocal({
										...(local ?? {}),
										enforce_schema: e.target.checked,
									} as IPinOptions)
								}
							/>
							<Label htmlFor={`opt-es-${pin.id}`} className="text-xs">
								{t('enforceSchema', 'Enforce Schema')}
							</Label>
						</div>

						<div className="flex items-center gap-2 md:col-span-3">
							<input
								id={`opt-sens-${pin.id}`}
								type="checkbox"
								className="h-4 w-4"
								checked={Boolean(local?.sensitive)}
								onChange={(e) =>
									setLocal({
										...(local ?? {}),
										sensitive: e.target.checked,
									} as IPinOptions)
								}
							/>
							<Label htmlFor={`opt-sens-${pin.id}`} className="text-xs">
								{t('sensitive', 'Sensitive')}
							</Label>
						</div>

						<div className="space-y-1.5 md:col-span-3">
							<Label className="text-xs">{t('step', 'Step')}</Label>
							<Input
								className="h-8"
								type="number"
								value={local?.step ?? ""}
								onChange={(e) =>
									setLocal({
										...(local ?? {}),
										step: e.target.value === "" ? null : Number(e.target.value),
									} as IPinOptions)
								}
							/>
						</div>

						<div className="space-y-1.5 md:col-span-3">
							<Label className="text-xs">{t('rangeMin', 'Range Min')}</Label>
							<Input
								className="h-8"
								type="number"
								value={local?.range?.[0] ?? ""}
								onChange={(e) => {
									const min =
										e.target.value === "" ? undefined : Number(e.target.value);
									const max = local?.range?.[1];
									const nextRange = [
										Number.isFinite(min as number)
											? (min as number)
											: undefined,
										Number.isFinite(max as number)
											? (max as number)
											: undefined,
									].filter((x) => typeof x === "number") as number[];
									setLocal({
										...(local ?? {}),
										range:
											nextRange.length === 2
												? nextRange
												: nextRange.length === 1
													? [nextRange[0]]
													: null,
									} as IPinOptions);
								}}
							/>
						</div>

						<div className="space-y-1.5 md:col-span-3">
							<Label className="text-xs">{t('rangeMax', 'Range Max')}</Label>
							<Input
								className="h-8"
								type="number"
								value={local?.range?.[1] ?? ""}
								onChange={(e) => {
									const min = local?.range?.[0];
									const max =
										e.target.value === "" ? undefined : Number(e.target.value);
									const nextRange = [
										Number.isFinite(min as number)
											? (min as number)
											: undefined,
										Number.isFinite(max as number)
											? (max as number)
											: undefined,
									].filter((x) => typeof x === "number") as number[];
									setLocal({
										...(local ?? {}),
										range:
											nextRange.length === 2
												? nextRange
												: nextRange.length === 1
													? [nextRange[0]]
													: null,
									} as IPinOptions);
								}}
							/>
						</div>

						<div className="space-y-1.5 md:col-span-6">
							<Label className="text-xs">{t('validValuesCommaseparated', 'Valid Values (comma-separated)')}</Label>
							<Input
								className="h-8"
								value={toCSV(local?.valid_values ?? null)}
								onChange={(e) =>
									setLocal({
										...(local ?? {}),
										valid_values:
											e.target.value.trim() === ""
												? null
												: fromCSVStrings(e.target.value),
									} as IPinOptions)
								}
							/>
						</div>
					</div>

					<DialogFooter className="gap-2">
						<Button variant="secondary" onClick={() => setOpen(false)}>
							{t('close', 'Close')}
						</Button>
						<Button
							onClick={() => {
								onApply(local ?? null);
								onSchemaChange(
									localSchema.trim() === "" ? null : localSchema.trim(),
								);
								setOpen(false);
							}}
						>
							{t('save', 'Save')}
						</Button>
					</DialogFooter>
				</DialogContent>
			</Dialog>
		</>
	);
};
