import { useReactFlow, useStore } from "@xyflow/react";
import { ChevronDown, RefreshCw } from "lucide-react";
import { type RefObject, useCallback, useEffect, useState } from "react";
import { toast } from "sonner";
import { useBackend } from "../../../..";
import {
	Select,
	SelectContent,
	SelectGroup,
	SelectItem,
	SelectLabel,
	SelectTrigger,
} from "../../../../components/ui/select";
import { useInvalidateInvoke } from "../../../../hooks";
import { updateNodeCommand, upsertLayerCommand } from "../../../../lib";
import type { IBoard } from "../../../../lib/schema/flow/board";
import type { IPin } from "../../../../lib/schema/flow/pin";
import {
	convertJsonToUint8Array,
	parseUint8ArrayToJson,
} from "../../../../lib/uint8";
import type {
	GraphOverlay,
	NodeLabelMapping,
	OntologyActionDefinition,
	RemoteOntologyImport,
} from "../../../../state/backend-state/graph-state";
import { useUndoRedo } from "../../flow-history";

// ─── Shared helpers ───

function normalizeStringValue(value: number[] | undefined | null): string {
	const parsed = parseUint8ArrayToJson(value);
	return typeof parsed === "string" ? parsed : "";
}

function objectIdentifier(object: NodeLabelMapping): string {
	return object.id ?? object.api_name ?? object.label;
}

function jsonType(dataType: string): string {
	const normalized = dataType.toLowerCase();
	if (normalized.includes("bool")) return "boolean";
	if (normalized.includes("int") || normalized.includes("uint"))
		return "integer";
	if (
		normalized.includes("float") ||
		normalized.includes("double") ||
		normalized.includes("decimal")
	)
		return "number";
	return "string";
}

/** Mirrors the Rust `object_schema` used by generated ontology bindings so
 * dropdown selection yields the same typed contract downstream. */
function objectSchemaString(object: NodeLabelMapping): string {
	const properties: Record<string, unknown> = {};
	for (const property of object.property_columns) {
		const type = jsonType(property.data_type);
		properties[property.name] = property.nullable
			? { type: [type, "null"] }
			: { type };
	}
	return JSON.stringify({
		$schema: "https://json-schema.org/draft/2020-12/schema",
		title: object.label,
		type: "object",
		properties,
	});
}

function resolveObject(
	overlay: GraphOverlay | undefined,
	objectType: string,
): NodeLabelMapping | undefined {
	return overlay?.nodes.find(
		(node) =>
			node.id === objectType ||
			node.api_name === objectType ||
			node.label === objectType,
	);
}

type SchemaUpdate = { pinName: string; schema: string };

/** Persists a pin's value plus any dependent pin default/schema changes in one
 * undoable board command — the same mechanism RemoteEventSelect uses. */
function useOntologyPinPersist(
	appId: string,
	boardId: string | undefined,
	nodeId: string,
	currentLayerId: string | undefined,
	boardRef: RefObject<IBoard | undefined> | undefined,
	setValue: (value: number[] | undefined) => void,
) {
	const backend = useBackend();
	const invalidate = useInvalidateInvoke();
	const { getNode } = useReactFlow();
	const { pushCommand } = useUndoRedo(appId, boardId ?? "");

	return useCallback(
		async (
			pin: IPin,
			value: string | undefined,
			options?: {
				clearPins?: string[];
				schemaUpdates?: SchemaUpdate[];
			},
		) => {
			const encoded =
				value === undefined ? undefined : convertJsonToUint8Array(value);
			const board = boardRef?.current;
			const currentLayer = currentLayerId
				? board?.layers?.[currentLayerId]
				: undefined;
			const layerNode = currentLayer?.nodes?.[nodeId];
			const boardNode = layerNode ?? board?.nodes?.[nodeId];
			if (!boardId || !boardNode) {
				setValue(encoded ?? undefined);
				return;
			}

			const flowNode = getNode(nodeId);
			const coordinates = flowNode
				? [flowNode.position.x, flowNode.position.y, 0]
				: (boardNode.coordinates ?? [0, 0, 0]);

			const pins = {
				...boardNode.pins,
				[pin.id]: { ...pin, default_value: encoded },
			};
			for (const name of options?.clearPins ?? []) {
				const target = Object.values(boardNode.pins ?? {}).find(
					(candidate) => candidate.name === name,
				);
				if (target) pins[target.id] = { ...target, default_value: undefined };
			}
			for (const update of options?.schemaUpdates ?? []) {
				const target = Object.values(boardNode.pins ?? {}).find(
					(candidate) => candidate.name === update.pinName,
				);
				if (target) pins[target.id] = { ...target, schema: update.schema };
			}

			const updatedNode = {
				...boardNode,
				hash: undefined,
				coordinates,
				pins,
			};
			const command =
				currentLayer && layerNode
					? upsertLayerCommand({
							current_layer: currentLayer.parent_id ?? null,
							layer: {
								...currentLayer,
								nodes: {
									...currentLayer.nodes,
									[nodeId]: updatedNode,
								},
							},
							node_ids: [],
						})
					: updateNodeCommand({ node: updatedNode });
			try {
				const result = await backend.boardState.executeCommand(
					appId,
					boardId,
					command,
				);
				await pushCommand(result, false);
			} catch {
				toast.error("Failed to save ontology selection");
			} finally {
				await invalidate(backend.boardState.getBoard, [appId, boardId]);
			}
		},
		[
			appId,
			backend.boardState,
			boardId,
			boardRef,
			currentLayerId,
			getNode,
			invalidate,
			nodeId,
			pushCommand,
			setValue,
		],
	);
}

function useOverlays(appId: string, open: boolean) {
	const backend = useBackend();
	const [overlays, setOverlays] = useState<GraphOverlay[]>([]);
	const [loading, setLoading] = useState(false);
	const [error, setError] = useState(false);

	useEffect(() => {
		if (!appId || !open) return;
		let cancelled = false;
		setLoading(true);
		setError(false);
		backend.graphState
			.listOverlays(appId)
			.then((result) => {
				if (!cancelled) setOverlays(result);
			})
			.catch(() => {
				if (!cancelled) setError(true);
			})
			.finally(() => {
				if (!cancelled) setLoading(false);
			});
		return () => {
			cancelled = true;
		};
	}, [appId, backend.graphState, open]);

	return { overlays, loading, error };
}

const EMPTY_PINS: Record<string, IPin> = {};

/** Reads the live pins of a node off the ReactFlow store. Reading from the
 * store (rather than the non-reactive `boardRef.current`) is what re-renders a
 * dependent selector the moment its parent pin changes: the FlowPin memo won't
 * re-render it otherwise, because only the parent pin's value changed, not its
 * own. */
function useStoreNodePins(nodeId: string): Record<string, IPin> {
	return useStore((state) => {
		const data = state.nodeLookup.get(nodeId)?.data as
			| { node?: { pins?: Record<string, IPin> } }
			| undefined;
		return data?.node?.pins ?? EMPTY_PINS;
	});
}

/** Reactively reads a sibling pin's string value off the live board node. */
function useSiblingPinValue(nodeId: string, pinName: string): string {
	const pins = useStoreNodePins(nodeId);
	const pin = Object.values(pins).find(
		(candidate) => candidate.name === pinName,
	);
	return normalizeStringValue(pin?.default_value);
}

/** Reactively reads a sibling pin's baked struct schema off the live board
 * node. `undefined` means the pin is absent (e.g. a per-property binding). */
function useSiblingPinSchema(
	nodeId: string,
	pinName: string,
): string | undefined {
	const pins = useStoreNodePins(nodeId);
	return (
		Object.values(pins).find((candidate) => candidate.name === pinName)
			?.schema ?? undefined
	);
}

/** Order-independent JSON serialization for stable schema comparison. */
function stableStringify(value: unknown): string {
	if (value === null || typeof value !== "object") return JSON.stringify(value);
	if (Array.isArray(value)) return `[${value.map(stableStringify).join(",")}]`;
	const entries = Object.entries(value as Record<string, unknown>).sort(
		([left], [right]) => (left < right ? -1 : left > right ? 1 : 0),
	);
	return `{${entries
		.map(([key, val]) => `${JSON.stringify(key)}:${stableStringify(val)}`)
		.join(",")}}`;
}

function schemaStrEqual(a: string | undefined, b: string | undefined): boolean {
	const normalize = (schema: string | undefined) => {
		if (!schema) return "";
		try {
			return stableStringify(JSON.parse(schema));
		} catch {
			return schema;
		}
	};
	return normalize(a) === normalize(b);
}

/** The schema pins an action selection should write, derived from the live
 * (producer or installed) contract. Shared by the local and remote selectors. */
function actionSchemaUpdates(
	action: OntologyActionDefinition | undefined,
	contract: GraphOverlay | undefined,
): SchemaUpdate[] {
	const updates: SchemaUpdate[] = [];
	if (action?.parameter_schema) {
		updates.push({
			pinName: "parameters",
			schema: JSON.stringify(action.parameter_schema),
		});
	}
	const object = action
		? resolveObject(contract, action.object_type)
		: undefined;
	if (object) {
		const objectSchema = objectSchemaString(object);
		// `objects` (request/input nodes) and the singular `object` (the action
		// input node) share the object's schema; updates for absent pins are
		// ignored by the persist step.
		updates.push({ pinName: "objects", schema: objectSchema });
		updates.push({ pinName: "object", schema: objectSchema });
	}
	return updates;
}

/** True when a placed node's baked schema no longer matches the live contract.
 * The runtime stays safe regardless (the producer validates authoritatively);
 * this only surfaces a one-click resync so typed pins stay honest. */
function actionSchemaDrift(
	action: OntologyActionDefinition | undefined,
	contract: GraphOverlay | undefined,
	currentParametersSchema: string | undefined,
): boolean {
	if (!action) return false;
	// Only the governed `parameters` contract drives drift. The `objects` schema
	// is advisory typing whose formatting is noisy, and generated bindings drop
	// the `parameters` struct pin entirely (per-property pins) — so a missing
	// pin means "not applicable", never drift.
	const update = actionSchemaUpdates(action, contract).find(
		(candidate) => candidate.pinName === "parameters",
	);
	if (!update) return false;
	if (currentParametersSchema === undefined) return false;
	return !schemaStrEqual(currentParametersSchema, update.schema);
}

/** Advisory drift affordance: re-applies the current contract schema to the
 * placed node's typed pins. Never auto-fires — the user clicks to resync. */
function SchemaSyncButton({ onSync }: Readonly<{ onSync: () => void }>) {
	return (
		<button
			type="button"
			onMouseDown={(event) => event.stopPropagation()}
			onPointerDown={(event) => event.stopPropagation()}
			onClick={onSync}
			className="ml-1 mt-1 inline-flex w-fit items-center gap-1 rounded-sm border border-border bg-muted/60 px-1.5 py-0.5 text-[10px] font-medium text-muted-foreground hover:bg-muted hover:text-foreground"
			title="The contract changed since this node was configured. Re-apply the current typed schema."
		>
			<RefreshCw className="h-3 w-3" /> Sync schema
		</button>
	);
}

// ─── Compact select shell (matches the remote-event dropdown styling) ───

function CompactSelect({
	disabled,
	value,
	placeholder,
	label,
	onChange,
	children,
	onOpenChange,
	open,
}: Readonly<{
	disabled?: boolean;
	value: string | undefined;
	placeholder: string;
	label: string;
	onChange: (value: string) => void;
	children: React.ReactNode;
	onOpenChange?: (open: boolean) => void;
	open?: boolean;
}>) {
	return (
		<div
			className="flex flex-row items-center justify-start max-w-full ml-1 overflow-hidden"
			onMouseDown={(event) => event.stopPropagation()}
			onPointerDown={(event) => event.stopPropagation()}
		>
			<Select
				disabled={disabled}
				open={open}
				onOpenChange={onOpenChange}
				value={value || undefined}
				onValueChange={onChange}
			>
				<SelectTrigger
					noChevron
					size="sm"
					className="w-fit! max-w-full! p-0 border-0 text-xs bg-card! text-start max-h-fit h-4 gap-0.5 flex-row items-center overflow-hidden"
				>
					<small className="text-start text-[10px] m-0! truncate">
						{value || placeholder}
					</small>
					<ChevronDown className="size-2 min-w-2 min-h-2 text-card-foreground shrink-0" />
				</SelectTrigger>
				<SelectContent>
					<SelectGroup>
						<SelectLabel>{label}</SelectLabel>
						{children}
					</SelectGroup>
				</SelectContent>
			</Select>
		</div>
	);
}

// ─── Local ontology selectors ───

export function OntologySelect({
	pin,
	value,
	appId,
	boardId,
	nodeId,
	currentLayerId,
	boardRef,
	setValue,
}: Readonly<{
	pin: IPin;
	value: number[] | undefined | null;
	appId: string;
	boardId?: string;
	nodeId: string;
	currentLayerId?: string;
	boardRef?: RefObject<IBoard | undefined>;
	setValue: (value: number[] | undefined) => void;
}>) {
	const [open, setOpen] = useState(false);
	const selectedId = normalizeStringValue(value);
	// Fetch eagerly when a value is set so the id resolves to a friendly name
	// instead of showing a raw UUID until the dropdown is opened.
	const { overlays, loading, error } = useOverlays(
		appId,
		open || Boolean(selectedId),
	);
	const persist = useOntologyPinPersist(
		appId,
		boardId,
		nodeId,
		currentLayerId,
		boardRef,
		setValue,
	);
	const selected = overlays.find((overlay) => overlay.id === selectedId);

	return (
		<CompactSelect
			open={open}
			onOpenChange={setOpen}
			value={selected?.name ?? selectedId}
			placeholder="Ontology"
			label={pin.friendly_name}
			onChange={(id) =>
				void persist(pin, id, { clearPins: ["object_type", "action_id"] })
			}
		>
			{loading && overlays.length === 0 && (
				<SelectLabel>Loading ontologies…</SelectLabel>
			)}
			{error && <SelectLabel>Could not load ontologies</SelectLabel>}
			{!loading && !error && overlays.length === 0 && (
				<SelectLabel>No ontologies defined</SelectLabel>
			)}
			{overlays.map((overlay) => (
				<SelectItem key={overlay.id} value={overlay.id}>
					{overlay.name}
				</SelectItem>
			))}
		</CompactSelect>
	);
}

export function OntologyObjectSelect({
	pin,
	value,
	appId,
	boardId,
	nodeId,
	currentLayerId,
	boardRef,
	setValue,
}: Readonly<{
	pin: IPin;
	value: number[] | undefined | null;
	appId: string;
	boardId?: string;
	nodeId: string;
	currentLayerId?: string;
	boardRef?: RefObject<IBoard | undefined>;
	setValue: (value: number[] | undefined) => void;
}>) {
	const [open, setOpen] = useState(false);
	const ontologyId = useSiblingPinValue(nodeId, "ontology_id");
	const { overlays, loading, error } = useOverlays(
		appId,
		open || Boolean(ontologyId),
	);
	const persist = useOntologyPinPersist(
		appId,
		boardId,
		nodeId,
		currentLayerId,
		boardRef,
		setValue,
	);
	const overlay = overlays.find((item) => item.id === ontologyId);
	const selected = normalizeStringValue(value);
	// Object bindings persist the stable identifier (id/api_name), so resolve it
	// back to the object's label for display instead of showing the raw slug.
	const selectedObject = resolveObject(overlay, selected);

	return (
		<CompactSelect
			open={open}
			onOpenChange={setOpen}
			disabled={!ontologyId}
			value={selectedObject?.label ?? selected}
			placeholder={ontologyId ? "Object type" : "Pick ontology"}
			label={pin.friendly_name}
			onChange={(objectType) => {
				const object = resolveObject(overlay, objectType);
				void persist(pin, objectType, {
					schemaUpdates: object
						? [{ pinName: "objects", schema: objectSchemaString(object) }]
						: undefined,
				});
			}}
		>
			{loading && <SelectLabel>Loading object types…</SelectLabel>}
			{error && <SelectLabel>Could not load object types</SelectLabel>}
			{overlay?.nodes.map((object) => (
				<SelectItem
					key={objectIdentifier(object)}
					value={objectIdentifier(object)}
				>
					{object.label}
				</SelectItem>
			))}
			{overlay && overlay.nodes.length === 0 && (
				<SelectLabel>No object types</SelectLabel>
			)}
		</CompactSelect>
	);
}

export function OntologyActionSelect({
	pin,
	value,
	appId,
	boardId,
	nodeId,
	currentLayerId,
	boardRef,
	setValue,
}: Readonly<{
	pin: IPin;
	value: number[] | undefined | null;
	appId: string;
	boardId?: string;
	nodeId: string;
	currentLayerId?: string;
	boardRef?: RefObject<IBoard | undefined>;
	setValue: (value: number[] | undefined) => void;
}>) {
	const [open, setOpen] = useState(false);
	const selectedId = normalizeStringValue(value);
	// Fetch eagerly when a value is already selected so schema drift surfaces
	// without opening the dropdown first.
	const { overlays, loading, error } = useOverlays(
		appId,
		open || Boolean(selectedId),
	);
	const persist = useOntologyPinPersist(
		appId,
		boardId,
		nodeId,
		currentLayerId,
		boardRef,
		setValue,
	);
	const ontologyId = useSiblingPinValue(nodeId, "ontology_id");
	const parametersSchema = useSiblingPinSchema(nodeId, "parameters");
	const overlay = overlays.find((item) => item.id === ontologyId);
	const actions = (overlay?.actions ?? []).filter((action) => action.enabled);
	const selected = actions.find((action) => action.id === selectedId);

	const selectAction = useCallback(
		(action: OntologyActionDefinition | undefined, actionId: string) => {
			const schemaUpdates = actionSchemaUpdates(action, overlay);
			void persist(pin, actionId, {
				schemaUpdates: schemaUpdates.length > 0 ? schemaUpdates : undefined,
			});
		},
		[overlay, persist, pin],
	);

	const drift = actionSchemaDrift(selected, overlay, parametersSchema);

	return (
		<div className="flex flex-col items-start max-w-full overflow-hidden">
			<CompactSelect
				open={open}
				onOpenChange={setOpen}
				disabled={!ontologyId}
				value={selected?.name ?? selectedId}
				placeholder={ontologyId ? "Action" : "Pick ontology"}
				label={pin.friendly_name}
				onChange={(actionId) =>
					selectAction(
						actions.find((action) => action.id === actionId),
						actionId,
					)
				}
			>
				{loading && <SelectLabel>Loading actions…</SelectLabel>}
				{error && <SelectLabel>Could not load actions</SelectLabel>}
				{!loading && !error && actions.length === 0 && (
					<SelectLabel>No enabled actions</SelectLabel>
				)}
				{actions.map((action) => (
					<SelectItem key={action.id} value={action.id}>
						{action.name}
					</SelectItem>
				))}
			</CompactSelect>
			{drift && selected && (
				<SchemaSyncButton onSync={() => selectAction(selected, selectedId)} />
			)}
		</div>
	);
}

// ─── Remote ontology selectors ───

function useImports(appId: string, open: boolean) {
	const backend = useBackend();
	const [imports, setImports] = useState<RemoteOntologyImport[]>([]);
	const [loading, setLoading] = useState(false);
	const [error, setError] = useState(false);

	useEffect(() => {
		if (!appId || !open) return;
		let cancelled = false;
		setLoading(true);
		setError(false);
		backend.graphState
			.listRemoteOntologyImports(appId)
			.then((result) => {
				if (!cancelled) setImports(result);
			})
			.catch(() => {
				if (!cancelled) setError(true);
			})
			.finally(() => {
				if (!cancelled) setLoading(false);
			});
		return () => {
			cancelled = true;
		};
	}, [appId, backend.graphState, open]);

	return { imports, loading, error };
}

export function RemoteOntologySelect({
	pin,
	value,
	appId,
	boardId,
	nodeId,
	currentLayerId,
	boardRef,
	setValue,
}: Readonly<{
	pin: IPin;
	value: number[] | undefined | null;
	appId: string;
	boardId?: string;
	nodeId: string;
	currentLayerId?: string;
	boardRef?: RefObject<IBoard | undefined>;
	setValue: (value: number[] | undefined) => void;
}>) {
	const [open, setOpen] = useState(false);
	const selectedId = normalizeStringValue(value);
	const { imports, loading, error } = useImports(
		appId,
		open || Boolean(selectedId),
	);
	const persist = useOntologyPinPersist(
		appId,
		boardId,
		nodeId,
		currentLayerId,
		boardRef,
		setValue,
	);
	const selected = imports.find((item) => item.id === selectedId);

	return (
		<CompactSelect
			open={open}
			onOpenChange={setOpen}
			value={selected?.contract.name ?? selectedId}
			placeholder="Installed ontology"
			label={pin.friendly_name}
			onChange={(id) => void persist(pin, id, { clearPins: ["object_type"] })}
		>
			{loading && imports.length === 0 && (
				<SelectLabel>Loading installed ontologies…</SelectLabel>
			)}
			{error && <SelectLabel>Could not load imports</SelectLabel>}
			{!loading && !error && imports.length === 0 && (
				<SelectLabel>No installed ontologies</SelectLabel>
			)}
			{imports.map((item) => (
				<SelectItem key={item.id} value={item.id}>
					{item.contract.name}
				</SelectItem>
			))}
		</CompactSelect>
	);
}

export function RemoteOntologyObjectSelect({
	pin,
	value,
	appId,
	boardId,
	nodeId,
	currentLayerId,
	boardRef,
	setValue,
}: Readonly<{
	pin: IPin;
	value: number[] | undefined | null;
	appId: string;
	boardId?: string;
	nodeId: string;
	currentLayerId?: string;
	boardRef?: RefObject<IBoard | undefined>;
	setValue: (value: number[] | undefined) => void;
}>) {
	const [open, setOpen] = useState(false);
	const bindingId = useSiblingPinValue(nodeId, "binding_id");
	const { imports, loading, error } = useImports(
		appId,
		open || Boolean(bindingId),
	);
	const persist = useOntologyPinPersist(
		appId,
		boardId,
		nodeId,
		currentLayerId,
		boardRef,
		setValue,
	);
	const contract = imports.find((item) => item.id === bindingId)?.contract;
	const selected = normalizeStringValue(value);
	// Remote bindings persist the stable object identifier (id/api_name), so
	// resolve it back to the object's label for display; without this the pin
	// shows the raw slug (e.g. "wmylhbri2juwcid…") instead of the object name.
	const selectedObject = resolveObject(contract, selected);

	return (
		<CompactSelect
			open={open}
			onOpenChange={setOpen}
			disabled={!bindingId}
			value={selectedObject?.label ?? selected}
			placeholder={bindingId ? "Object type" : "Pick ontology"}
			label={pin.friendly_name}
			onChange={(objectType) => {
				const object = resolveObject(contract, objectType);
				void persist(pin, objectType, {
					schemaUpdates: object
						? [{ pinName: "objects", schema: objectSchemaString(object) }]
						: undefined,
				});
			}}
		>
			{loading && <SelectLabel>Loading object types…</SelectLabel>}
			{error && <SelectLabel>Could not load object types</SelectLabel>}
			{contract?.nodes.map((object) => (
				<SelectItem
					key={objectIdentifier(object)}
					value={objectIdentifier(object)}
				>
					{object.label}
				</SelectItem>
			))}
			{contract && contract.nodes.length === 0 && (
				<SelectLabel>No object types</SelectLabel>
			)}
		</CompactSelect>
	);
}

export function RemoteOntologyActionSelect({
	pin,
	value,
	appId,
	boardId,
	nodeId,
	currentLayerId,
	boardRef,
	setValue,
}: Readonly<{
	pin: IPin;
	value: number[] | undefined | null;
	appId: string;
	boardId?: string;
	nodeId: string;
	currentLayerId?: string;
	boardRef?: RefObject<IBoard | undefined>;
	setValue: (value: number[] | undefined) => void;
}>) {
	const [open, setOpen] = useState(false);
	const selectedId = normalizeStringValue(value);
	const { imports, loading, error } = useImports(
		appId,
		open || Boolean(selectedId),
	);
	const persist = useOntologyPinPersist(
		appId,
		boardId,
		nodeId,
		currentLayerId,
		boardRef,
		setValue,
	);
	const bindingId = useSiblingPinValue(nodeId, "binding_id");
	const parametersSchema = useSiblingPinSchema(nodeId, "parameters");
	const contract = imports.find((item) => item.id === bindingId)?.contract;
	const actions = (contract?.actions ?? []).filter((action) => action.enabled);
	const selected = actions.find((action) => action.id === selectedId);

	const selectAction = useCallback(
		(action: OntologyActionDefinition | undefined, actionId: string) => {
			const schemaUpdates = actionSchemaUpdates(action, contract);
			void persist(pin, actionId, {
				schemaUpdates: schemaUpdates.length > 0 ? schemaUpdates : undefined,
			});
		},
		[contract, persist, pin],
	);

	const drift = actionSchemaDrift(selected, contract, parametersSchema);

	return (
		<div className="flex flex-col items-start max-w-full overflow-hidden">
			<CompactSelect
				open={open}
				onOpenChange={setOpen}
				disabled={!bindingId}
				value={selected?.name ?? selectedId}
				placeholder={bindingId ? "Action" : "Pick ontology"}
				label={pin.friendly_name}
				onChange={(actionId) =>
					selectAction(
						actions.find((action) => action.id === actionId),
						actionId,
					)
				}
			>
				{loading && <SelectLabel>Loading actions…</SelectLabel>}
				{error && <SelectLabel>Could not load actions</SelectLabel>}
				{!loading && !error && actions.length === 0 && (
					<SelectLabel>No enabled actions</SelectLabel>
				)}
				{actions.map((action) => (
					<SelectItem key={action.id} value={action.id}>
						{action.name}
					</SelectItem>
				))}
			</CompactSelect>
			{drift && selected && (
				<SchemaSyncButton onSync={() => selectAction(selected, selectedId)} />
			)}
		</div>
	);
}
