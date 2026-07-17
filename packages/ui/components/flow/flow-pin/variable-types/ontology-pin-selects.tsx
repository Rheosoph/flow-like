import { useReactFlow } from "@xyflow/react";
import { ChevronDown } from "lucide-react";
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
import { updateNodeCommand } from "../../../../lib";
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
			const boardNode = boardRef?.current?.nodes?.[nodeId];
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

			const command = updateNodeCommand({
				node: { ...boardNode, hash: undefined, coordinates, pins },
			});
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

function siblingValue(
	boardRef: RefObject<IBoard | undefined> | undefined,
	nodeId: string,
	pinName: string,
): string {
	const pins = boardRef?.current?.nodes?.[nodeId]?.pins ?? {};
	const pin = Object.values(pins).find(
		(candidate) => candidate.name === pinName,
	);
	return normalizeStringValue(pin?.default_value);
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
	boardRef,
	setValue,
}: Readonly<{
	pin: IPin;
	value: number[] | undefined | null;
	appId: string;
	boardId?: string;
	nodeId: string;
	boardRef?: RefObject<IBoard | undefined>;
	setValue: (value: number[] | undefined) => void;
}>) {
	const [open, setOpen] = useState(false);
	const { overlays, loading, error } = useOverlays(appId, open);
	const persist = useOntologyPinPersist(
		appId,
		boardId,
		nodeId,
		boardRef,
		setValue,
	);
	const selectedId = normalizeStringValue(value);
	const selected = overlays.find((overlay) => overlay.id === selectedId);

	return (
		<CompactSelect
			open={open}
			onOpenChange={setOpen}
			value={selected?.name ?? selectedId}
			placeholder="Select ontology"
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
	boardRef,
	setValue,
}: Readonly<{
	pin: IPin;
	value: number[] | undefined | null;
	appId: string;
	boardId?: string;
	nodeId: string;
	boardRef?: RefObject<IBoard | undefined>;
	setValue: (value: number[] | undefined) => void;
}>) {
	const [open, setOpen] = useState(false);
	const { overlays, loading, error } = useOverlays(appId, open);
	const persist = useOntologyPinPersist(
		appId,
		boardId,
		nodeId,
		boardRef,
		setValue,
	);
	const ontologyId = siblingValue(boardRef, nodeId, "ontology_id");
	const overlay = overlays.find((item) => item.id === ontologyId);
	const selected = normalizeStringValue(value);

	return (
		<CompactSelect
			open={open}
			onOpenChange={setOpen}
			disabled={!ontologyId}
			value={selected}
			placeholder={ontologyId ? "Select object type" : "Pick an ontology first"}
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
	boardRef,
	setValue,
}: Readonly<{
	pin: IPin;
	value: number[] | undefined | null;
	appId: string;
	boardId?: string;
	nodeId: string;
	boardRef?: RefObject<IBoard | undefined>;
	setValue: (value: number[] | undefined) => void;
}>) {
	const [open, setOpen] = useState(false);
	const { overlays, loading, error } = useOverlays(appId, open);
	const persist = useOntologyPinPersist(
		appId,
		boardId,
		nodeId,
		boardRef,
		setValue,
	);
	const ontologyId = siblingValue(boardRef, nodeId, "ontology_id");
	const overlay = overlays.find((item) => item.id === ontologyId);
	const actions = (overlay?.actions ?? []).filter((action) => action.enabled);
	const selectedId = normalizeStringValue(value);
	const selected = actions.find((action) => action.id === selectedId);

	const selectAction = useCallback(
		(action: OntologyActionDefinition | undefined, actionId: string) => {
			const schemaUpdates: SchemaUpdate[] = [];
			// Type the parameters pin from the action's contract so downstream
			// pins and the struct editor reflect the action's real inputs.
			if (action?.parameter_schema) {
				schemaUpdates.push({
					pinName: "parameters",
					schema: JSON.stringify(action.parameter_schema),
				});
			}
			const object = action
				? resolveObject(overlay, action.object_type)
				: undefined;
			if (object) {
				schemaUpdates.push({
					pinName: "objects",
					schema: objectSchemaString(object),
				});
			}
			void persist(pin, actionId, {
				schemaUpdates: schemaUpdates.length > 0 ? schemaUpdates : undefined,
			});
		},
		[overlay, persist, pin],
	);

	return (
		<CompactSelect
			open={open}
			onOpenChange={setOpen}
			disabled={!ontologyId}
			value={selected?.name ?? selectedId}
			placeholder={ontologyId ? "Select action" : "Pick an ontology first"}
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
	boardRef,
	setValue,
}: Readonly<{
	pin: IPin;
	value: number[] | undefined | null;
	appId: string;
	boardId?: string;
	nodeId: string;
	boardRef?: RefObject<IBoard | undefined>;
	setValue: (value: number[] | undefined) => void;
}>) {
	const [open, setOpen] = useState(false);
	const { imports, loading, error } = useImports(appId, open);
	const persist = useOntologyPinPersist(
		appId,
		boardId,
		nodeId,
		boardRef,
		setValue,
	);
	const selectedId = normalizeStringValue(value);
	const selected = imports.find((item) => item.id === selectedId);

	return (
		<CompactSelect
			open={open}
			onOpenChange={setOpen}
			value={selected?.contract.name ?? selectedId}
			placeholder="Select installed ontology"
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
	boardRef,
	setValue,
}: Readonly<{
	pin: IPin;
	value: number[] | undefined | null;
	appId: string;
	boardId?: string;
	nodeId: string;
	boardRef?: RefObject<IBoard | undefined>;
	setValue: (value: number[] | undefined) => void;
}>) {
	const [open, setOpen] = useState(false);
	const { imports, loading, error } = useImports(appId, open);
	const persist = useOntologyPinPersist(
		appId,
		boardId,
		nodeId,
		boardRef,
		setValue,
	);
	const bindingId = siblingValue(boardRef, nodeId, "binding_id");
	const contract = imports.find((item) => item.id === bindingId)?.contract;
	const selected = normalizeStringValue(value);

	return (
		<CompactSelect
			open={open}
			onOpenChange={setOpen}
			disabled={!bindingId}
			value={selected}
			placeholder={bindingId ? "Select object type" : "Pick an ontology first"}
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
