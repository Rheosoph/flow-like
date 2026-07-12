"use client";

import { createId } from "@paralleldrive/cuid2";
import {
	ArrowRight,
	Box,
	Braces,
	CheckCircle2,
	ChevronRight,
	CircleDot,
	Database,
	ExternalLink,
	FileKey,
	GitBranch,
	Layers3,
	Loader2,
	Network,
	Play,
	Plus,
	RefreshCw,
	Search,
	Share2,
	ShieldCheck,
	Workflow,
	X,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { IBoard } from "../../../lib/schema/flow/board";
import type {
	GraphOverlay,
	InvokeOntologyActionPayload,
	NodeLabelMapping,
	OntologyActionDefinition,
	OntologyActionRun,
	RemoteOntologyImport,
} from "../../../state/backend-state/graph-state";
import type { IAppConnection } from "../../../state/backend-state/types";
import {
	AlertDialog,
	AlertDialogAction,
	AlertDialogCancel,
	AlertDialogContent,
	AlertDialogDescription,
	AlertDialogFooter,
	AlertDialogHeader,
	AlertDialogTitle,
	AlertDialogTrigger,
} from "../../ui/alert-dialog";
import { Badge } from "../../ui/badge";
import { Button } from "../../ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "../../ui/card";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogFooter,
	DialogHeader,
	DialogTitle,
} from "../../ui/dialog";
import { Input } from "../../ui/input";
import { Label } from "../../ui/label";
import { ScrollArea } from "../../ui/scroll-area";
import {
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "../../ui/select";
import { Separator } from "../../ui/separator";
import { Sheet, SheetContent, SheetHeader, SheetTitle } from "../../ui/sheet";
import { Switch } from "../../ui/switch";
import { Textarea } from "../../ui/textarea";

interface StudioPanelBaseProps {
	ontologies: GraphOverlay[];
	onCreateOntology: () => void;
}

export function humanizeIdentifier(value: string): string {
	return value
		.replace(/([a-z0-9])([A-Z])/g, "$1 $2")
		.replace(/[_-]+/g, " ")
		.replace(/\b\w/g, (character) => character.toUpperCase());
}

function objectKey(object: NodeLabelMapping): string {
	return object.id ?? object.api_name ?? object.label;
}

function EmptyStudioState({
	title,
	description,
	onCreate,
}: Readonly<{
	title: string;
	description: string;
	onCreate: () => void;
}>) {
	return (
		<div className="flex min-h-72 flex-col items-center justify-center rounded-xl border border-dashed bg-muted/20 p-8 text-center">
			<div className="mb-4 rounded-2xl bg-primary/10 p-3 text-primary">
				<Layers3 className="h-6 w-6" />
			</div>
			<h3 className="font-semibold">{title}</h3>
			<p className="mt-1 max-w-md text-sm text-muted-foreground">
				{description}
			</p>
			<Button className="mt-5" onClick={onCreate}>
				<Plus className="h-4 w-4" /> Set up ontology
			</Button>
		</div>
	);
}

export function DataStudioOverview({
	ontologies,
	tableCount,
	onCreateOntology,
	onOpenOntology,
	onNavigate,
}: Readonly<
	StudioPanelBaseProps & {
		tableCount: number;
		onOpenOntology: (ontologyId: string) => void;
		onNavigate: (view: string) => void;
	}
>) {
	const objectCount = ontologies.reduce(
		(total, ontology) => total + ontology.nodes.length,
		0,
	);
	const actionCount = ontologies.reduce(
		(total, ontology) => total + (ontology.actions?.length ?? 0),
		0,
	);
	const exposedCount = ontologies.filter((ontology) => ontology.exposed).length;

	return (
		<div className="space-y-6">
			<div className="grid gap-3 sm:grid-cols-2 xl:grid-cols-4">
				{[
					{ label: "Ontologies", value: ontologies.length, icon: Layers3 },
					{ label: "Object types", value: objectCount, icon: Box },
					{ label: "Actions", value: actionCount, icon: Workflow },
					{ label: "Shared", value: exposedCount, icon: Share2 },
				].map(({ label, value, icon: Icon }) => (
					<Card key={label}>
						<CardContent className="flex items-center justify-between p-4">
							<div>
								<p className="text-xs font-medium text-muted-foreground">
									{label}
								</p>
								<p className="mt-1 text-2xl font-semibold">{value}</p>
							</div>
							<div className="rounded-xl bg-primary/10 p-2.5 text-primary">
								<Icon className="h-4 w-4" />
							</div>
						</CardContent>
					</Card>
				))}
			</div>

			<div className="grid gap-5 xl:grid-cols-[1.4fr_1fr]">
				<Card>
					<CardHeader className="flex-row items-center justify-between space-y-0 pb-3">
						<div>
							<CardTitle className="text-base">Your semantic layer</CardTitle>
							<p className="mt-1 text-sm text-muted-foreground">
								Objects, relationships, views, and operations over {tableCount}{" "}
								tables.
							</p>
						</div>
						<Button size="sm" onClick={onCreateOntology}>
							<Plus className="h-4 w-4" /> New ontology
						</Button>
					</CardHeader>
					<CardContent>
						{ontologies.length === 0 ? (
							<div className="rounded-xl border border-dashed p-8 text-center">
								<p className="text-sm font-medium">
									Model your first business object
								</p>
								<p className="mt-1 text-xs text-muted-foreground">
									Select tables and Data Studio will infer object IDs, display
									fields, and relationships.
								</p>
							</div>
						) : (
							<div className="space-y-2">
								{ontologies.slice(0, 5).map((ontology) => (
									<button
										type="button"
										key={ontology.id}
										onClick={() => onOpenOntology(ontology.id)}
										className="flex w-full items-center gap-3 rounded-lg border p-3 text-left transition-colors hover:bg-muted/50"
									>
										<div className="rounded-lg bg-primary/10 p-2 text-primary">
											<Network className="h-4 w-4" />
										</div>
										<div className="min-w-0 flex-1">
											<p className="truncate text-sm font-medium">
												{ontology.name}
											</p>
											<p className="text-xs text-muted-foreground">
												{ontology.nodes.length} objects ·{" "}
												{ontology.edges.length} relationships
											</p>
										</div>
										{ontology.bindings_enabled && (
											<Badge variant="secondary">Bindings</Badge>
										)}
										<ChevronRight className="h-4 w-4 text-muted-foreground" />
									</button>
								))}
							</div>
						)}
					</CardContent>
				</Card>

				<Card>
					<CardHeader className="pb-3">
						<CardTitle className="text-base">Start with a task</CardTitle>
					</CardHeader>
					<CardContent className="space-y-2">
						{[
							{
								view: "objects",
								title: "Explore business objects",
								description: "Search and inspect generated object views",
								icon: Search,
							},
							{
								view: "model",
								title: "Shape the model",
								description: "Review types, links, mappings, and health",
								icon: GitBranch,
							},
							{
								view: "actions",
								title: "Connect an action",
								description: "Bind an operation to a typed board entry",
								icon: Workflow,
							},
							{
								view: "sharing",
								title: "Expose a contract",
								description: "Share with projects through app connections",
								icon: Share2,
							},
						].map(({ view, title, description, icon: Icon }) => (
							<button
								type="button"
								key={view}
								onClick={() => onNavigate(view)}
								className="flex w-full items-center gap-3 rounded-lg p-2.5 text-left transition-colors hover:bg-muted"
							>
								<Icon className="h-4 w-4 text-muted-foreground" />
								<div className="min-w-0 flex-1">
									<p className="text-sm font-medium">{title}</p>
									<p className="truncate text-xs text-muted-foreground">
										{description}
									</p>
								</div>
								<ArrowRight className="h-3.5 w-3.5 text-muted-foreground" />
							</button>
						))}
					</CardContent>
				</Card>
			</div>
		</div>
	);
}

export function ObjectExplorerPanel({
	ontologies,
	onCreateOntology,
	onSample,
	onInvokeAction,
}: Readonly<
	StudioPanelBaseProps & {
		onSample: (
			ontologyId: string,
			objectType: string,
			limit: number,
		) => Promise<unknown[]>;
		onInvokeAction: (
			ontologyId: string,
			actionId: string,
			payload: InvokeOntologyActionPayload,
			onStatus?: (run: OntologyActionRun) => void,
		) => Promise<OntologyActionRun>;
	}
>) {
	const [selectedOntologyId, setSelectedOntologyId] = useState(
		ontologies[0]?.id ?? "",
	);
	const [selectedObjectKey, setSelectedObjectKey] = useState("");
	const [rows, setRows] = useState<Record<string, unknown>[]>([]);
	const [selectedRow, setSelectedRow] = useState<Record<
		string,
		unknown
	> | null>(null);
	const [query, setQuery] = useState("");
	const [loading, setLoading] = useState(false);
	const [error, setError] = useState<string | null>(null);
	const loadGeneration = useRef(0);

	const ontology = useMemo(
		() =>
			ontologies.find((item) => item.id === selectedOntologyId) ??
			ontologies[0],
		[ontologies, selectedOntologyId],
	);
	const objectType = useMemo(
		() =>
			ontology?.nodes.find(
				(object) => objectKey(object) === selectedObjectKey,
			) ?? ontology?.nodes[0],
		[ontology, selectedObjectKey],
	);

	useEffect(() => {
		if (!ontology) return;
		setSelectedOntologyId(ontology.id);
		if (!objectType && ontology.nodes[0])
			setSelectedObjectKey(objectKey(ontology.nodes[0]));
		else if (objectType) setSelectedObjectKey(objectKey(objectType));
	}, [objectType, ontology]);

	const activeObjectKey = objectType ? objectKey(objectType) : "";
	const activeSelectionKey = `${ontology?.id ?? ""}:${activeObjectKey}`;
	const activeSelectionRef = useRef(activeSelectionKey);
	useEffect(() => {
		activeSelectionRef.current = activeSelectionKey;
		loadGeneration.current += 1;
		setSelectedRow(null);
		setRows([]);
		setError(null);
	}, [activeSelectionKey]);

	const loadObjects = useCallback(async () => {
		if (!ontology || !objectType) return;
		const generation = ++loadGeneration.current;
		const selectionKey = activeSelectionKey;
		setLoading(true);
		setError(null);
		try {
			const result = await onSample(ontology.id, objectType.label, 100);
			if (
				generation !== loadGeneration.current ||
				selectionKey !== activeSelectionRef.current
			)
				return;
			const nextRows = result.filter(
				(row): row is Record<string, unknown> =>
					typeof row === "object" && row !== null && !Array.isArray(row),
			);
			setRows(nextRows);
			setSelectedRow((current) => {
				if (!current) return current;
				const currentId = current[objectType.id_column];
				return (
					nextRows.find((row) => row[objectType.id_column] === currentId) ??
					null
				);
			});
		} catch (loadError) {
			if (
				generation !== loadGeneration.current ||
				selectionKey !== activeSelectionRef.current
			)
				return;
			setError(
				loadError instanceof Error
					? loadError.message
					: "Could not load objects.",
			);
			setRows([]);
		} finally {
			if (generation === loadGeneration.current) setLoading(false);
		}
	}, [activeSelectionKey, objectType, onSample, ontology]);

	useEffect(() => {
		loadObjects();
	}, [loadObjects]);

	const visibleRows = useMemo(() => {
		const normalized = query.trim().toLowerCase();
		if (!normalized) return rows;
		return rows.filter((row) =>
			Object.values(row).some((value) =>
				String(value ?? "")
					.toLowerCase()
					.includes(normalized),
			),
		);
	}, [query, rows]);
	const columns = useMemo(() => {
		if (!objectType) return [];
		const preferred = [objectType.id_column, objectType.display_column].filter(
			(value): value is string => Boolean(value),
		);
		const rest = objectType.property_columns.map((property) => property.name);
		return Array.from(new Set([...preferred, ...rest])).slice(0, 8);
	}, [objectType]);

	if (ontologies.length === 0) {
		return (
			<EmptyStudioState
				title="No objects to explore"
				description="Set up an ontology to turn native tables into searchable business objects and standard object views."
				onCreate={onCreateOntology}
			/>
		);
	}

	return (
		<div className="grid h-full min-h-0 grid-cols-1 overflow-hidden rounded-xl border lg:grid-cols-[260px_minmax(0,1fr)]">
			<aside className="min-h-0 border-b bg-muted/20 lg:border-r lg:border-b-0">
				<div className="border-b p-3">
					<Select
						value={ontology?.id}
						onValueChange={(value) => {
							setSelectedOntologyId(value);
							setSelectedObjectKey("");
						}}
					>
						<SelectTrigger
							className="bg-background"
							aria-label="Select ontology"
						>
							<SelectValue />
						</SelectTrigger>
						<SelectContent>
							{ontologies.map((item) => (
								<SelectItem key={item.id} value={item.id}>
									{item.name}
								</SelectItem>
							))}
						</SelectContent>
					</Select>
				</div>
				<ScrollArea className="h-[180px] lg:h-[calc(100%-61px)]">
					<div className="space-y-1 p-2">
						<p className="px-2 py-1 text-[10px] font-semibold uppercase tracking-wider text-muted-foreground">
							Object types
						</p>
						{ontology?.nodes.map((object) => {
							const active =
								objectKey(object) === objectKey(objectType ?? object);
							return (
								<button
									type="button"
									key={objectKey(object)}
									onClick={() => setSelectedObjectKey(objectKey(object))}
									className={`flex w-full items-center gap-2 rounded-lg px-2.5 py-2 text-left text-sm ${active ? "bg-primary text-primary-foreground" : "hover:bg-muted"}`}
								>
									<CircleDot className="h-3.5 w-3.5" />
									<span className="truncate">{object.label}</span>
								</button>
							);
						})}
					</div>
				</ScrollArea>
			</aside>

			<section className="flex min-h-0 min-w-0 flex-col">
				<header className="flex flex-col gap-3 border-b p-4 sm:flex-row sm:items-center sm:justify-between">
					<div>
						<div className="flex items-center gap-2">
							<h2 className="font-semibold">{objectType?.label}</h2>
							<Badge variant="outline">{visibleRows.length} preview</Badge>
						</div>
						<p className="text-xs text-muted-foreground">
							Standard object view · source {objectType?.table}
						</p>
					</div>
					<div className="flex items-center gap-2">
						<div className="relative min-w-56">
							<Search className="absolute left-2.5 top-2.5 h-4 w-4 text-muted-foreground" />
							<Input
								value={query}
								onChange={(event) => setQuery(event.target.value)}
								placeholder="Filter loaded objects"
								aria-label="Filter loaded objects"
								className="pl-8"
							/>
							{query && (
								<Button
									variant="ghost"
									size="icon"
									className="absolute right-0 top-0 h-9 w-9"
									onClick={() => setQuery("")}
									aria-label="Clear object filter"
								>
									<X className="h-3.5 w-3.5" />
								</Button>
							)}
						</div>
						<Button
							variant="outline"
							size="icon"
							onClick={loadObjects}
							disabled={loading}
							aria-label="Refresh objects"
						>
							<RefreshCw
								className={`h-4 w-4 ${loading ? "animate-spin" : ""}`}
							/>
						</Button>
					</div>
				</header>

				<div className="min-h-0 flex-1 overflow-auto">
					{loading ? (
						<div className="flex h-full items-center justify-center">
							<Loader2 className="h-5 w-5 animate-spin text-muted-foreground" />
						</div>
					) : error ? (
						<div
							role="alert"
							className="m-4 rounded-lg border border-destructive/30 bg-destructive/10 p-4 text-sm text-destructive"
						>
							{error}
						</div>
					) : visibleRows.length === 0 ? (
						<div className="flex h-full items-center justify-center p-8 text-sm text-muted-foreground">
							No objects in this preview.
						</div>
					) : (
						<table className="w-full text-sm">
							<thead className="sticky top-0 z-10 bg-background">
								<tr className="border-b">
									{columns.map((column) => (
										<th
											key={column}
											className="px-4 py-2.5 text-left text-xs font-medium text-muted-foreground"
										>
											{humanizeIdentifier(column)}
										</th>
									))}
									<th className="w-10">
										<span className="sr-only">Open object</span>
									</th>
								</tr>
							</thead>
							<tbody>
								{visibleRows.map((row, index) => (
									<tr
										key={String(row[objectType?.id_column ?? ""] ?? index)}
										className="border-b transition-colors hover:bg-muted/50"
									>
										{columns.map((column) => (
											<td
												key={column}
												className="max-w-64 truncate px-4 py-2.5"
											>
												{typeof row[column] === "object"
													? JSON.stringify(row[column])
													: String(row[column] ?? "—")}
											</td>
										))}
										<td className="pr-3">
											<Button
												variant="ghost"
												size="icon"
												className="h-8 w-8"
												onClick={() => setSelectedRow(row)}
												aria-label={`Open ${objectType?.label ?? "object"} ${String(
													row[objectType?.display_column ?? ""] ??
														row[objectType?.id_column ?? ""] ??
														index + 1,
												)}`}
											>
												<ChevronRight className="h-4 w-4 text-muted-foreground" />
											</Button>
										</td>
									</tr>
								))}
							</tbody>
						</table>
					)}
				</div>
			</section>

			<ObjectViewSheet
				ontology={ontology}
				objectType={objectType}
				row={selectedRow}
				onClose={() => setSelectedRow(null)}
				onInvokeAction={onInvokeAction}
				onActionApplied={loadObjects}
			/>
		</div>
	);
}

function ObjectViewSheet({
	ontology,
	objectType,
	row,
	onClose,
	onInvokeAction,
	onActionApplied,
}: Readonly<{
	ontology?: GraphOverlay;
	objectType?: NodeLabelMapping;
	row: Record<string, unknown> | null;
	onClose: () => void;
	onInvokeAction: (
		ontologyId: string,
		actionId: string,
		payload: InvokeOntologyActionPayload,
		onStatus?: (run: OntologyActionRun) => void,
	) => Promise<OntologyActionRun>;
	onActionApplied: () => Promise<void>;
}>) {
	const [selectedAction, setSelectedAction] =
		useState<OntologyActionDefinition | null>(null);
	const view = ontology?.object_views?.find(
		(item) =>
			item.object_type ===
			objectKey(
				objectType ?? {
					label: "",
					table: "",
					id_column: "",
					property_columns: [],
					style: { color: "", icon: "", size: { mode: "fixed" } },
				},
			),
	);
	const titleProperty =
		view?.title_property ?? objectType?.display_column ?? objectType?.id_column;
	const prominent =
		view?.prominent_properties ??
		objectType?.property_columns.slice(0, 4).map((property) => property.name) ??
		[];
	const actions =
		ontology?.actions?.filter(
			(action) =>
				action.enabled &&
				action.object_type ===
					objectKey(
						objectType ?? {
							label: "",
							table: "",
							id_column: "",
							property_columns: [],
							style: { color: "", icon: "", size: { mode: "fixed" } },
						},
					),
		) ?? [];
	return (
		<Sheet open={Boolean(row)} onOpenChange={(open) => !open && onClose()}>
			<SheetContent className="w-full overflow-y-auto sm:max-w-xl">
				<SheetHeader className="border-b pb-4 text-left">
					<div className="flex items-center gap-2 text-xs text-muted-foreground">
						<Box className="h-3.5 w-3.5" />
						{objectType?.label}
					</div>
					<SheetTitle className="text-xl">
						{String(
							row?.[titleProperty ?? ""] ??
								row?.[objectType?.id_column ?? ""] ??
								"Object",
						)}
					</SheetTitle>
				</SheetHeader>
				{row && (
					<div className="space-y-6 py-5">
						{prominent.length > 0 && (
							<div className="grid grid-cols-2 gap-3">
								{prominent.map((property) => (
									<div
										key={property}
										className="rounded-xl border bg-muted/20 p-3"
									>
										<p className="text-[10px] font-medium uppercase tracking-wider text-muted-foreground">
											{humanizeIdentifier(property)}
										</p>
										<p className="mt-1 break-words text-sm font-medium">
											{String(row[property] ?? "—")}
										</p>
									</div>
								))}
							</div>
						)}
						{actions.length > 0 && (
							<div>
								<p className="mb-2 text-xs font-medium text-muted-foreground">
									Available actions
								</p>
								<div className="flex flex-wrap gap-2">
									{actions.map((action) => (
										<Button
											key={action.id}
											variant="outline"
											size="sm"
											className="gap-1.5"
											onClick={() => setSelectedAction(action)}
										>
											<Workflow className="h-3 w-3" />
											{action.name}
										</Button>
									))}
								</div>
							</div>
						)}
						<Separator />
						<div className="space-y-1">
							{Object.entries(row).map(([property, value]) => (
								<div
									key={property}
									className="grid grid-cols-[minmax(120px,0.8fr)_minmax(0,1.4fr)] gap-4 border-b py-2.5"
								>
									<p className="text-xs text-muted-foreground">
										{humanizeIdentifier(property)}
									</p>
									<p className="break-words text-sm">
										{typeof value === "object"
											? JSON.stringify(value)
											: String(value ?? "—")}
									</p>
								</div>
							))}
						</div>
						<div className="rounded-lg border bg-muted/20 p-3 text-xs text-muted-foreground">
							<div className="flex items-center gap-1.5 font-medium text-foreground">
								<Database className="h-3.5 w-3.5" />
								Lineage
							</div>
							<p className="mt-1">
								{objectType?.table} · ontology {ontology?.name}
							</p>
						</div>
					</div>
				)}
				<OntologyActionDialog
					key={selectedAction?.id ?? "no-action"}
					open={Boolean(selectedAction)}
					action={selectedAction}
					ontology={ontology}
					objectType={objectType}
					row={row}
					onOpenChange={(open) => !open && setSelectedAction(null)}
					onInvokeAction={onInvokeAction}
					onActionApplied={onActionApplied}
				/>
			</SheetContent>
		</Sheet>
	);
}

interface ActionSchemaProperty {
	type?: string | string[];
	title?: string;
	description?: string;
	default?: unknown;
	enum?: unknown[];
}

const SUCCESSFUL_ACTION_STATUSES = new Set([
	"complete",
	"completed",
	"success",
	"succeeded",
	"applied",
]);

function actionSucceeded(status: string): boolean {
	return SUCCESSFUL_ACTION_STATUSES.has(status.trim().toLowerCase());
}

interface ActionParameterSchema {
	properties?: Record<string, ActionSchemaProperty>;
	required?: string[];
}

function toActionParameterSchema(
	schema?: Record<string, unknown>,
): ActionParameterSchema | undefined {
	if (!schema || typeof schema !== "object") return undefined;
	return schema as ActionParameterSchema;
}

function parameterType(property: ActionSchemaProperty): string {
	if (Array.isArray(property.type)) {
		return property.type.find((type) => type !== "null") ?? "string";
	}
	return property.type ?? "string";
}

function initialActionParameters(
	schema?: Record<string, unknown>,
): Record<string, unknown> {
	const definition = toActionParameterSchema(schema);
	const properties = definition?.properties ?? {};
	const required = new Set(definition?.required ?? []);
	return Object.fromEntries(
		Object.entries(properties).flatMap(([name, property]) => {
			if (property.default !== undefined) return [[name, property.default]];
			if (required.has(name) && parameterType(property) === "boolean") {
				return [[name, false]];
			}
			if (required.has(name) && parameterType(property) === "array") {
				return [[name, []]];
			}
			if (required.has(name) && parameterType(property) === "object") {
				return [[name, {}]];
			}
			return [];
		}),
	);
}

function OntologyActionParameterForm({
	actionId,
	schema,
	parameters,
	disabled,
	onChange,
	onValidityChange,
}: Readonly<{
	actionId: string;
	schema?: Record<string, unknown>;
	parameters: Record<string, unknown>;
	disabled: boolean;
	onChange: (parameters: Record<string, unknown>) => void;
	onValidityChange: (valid: boolean) => void;
}>) {
	const definition = toActionParameterSchema(schema);
	const properties = definition?.properties ?? {};
	const [jsonDrafts, setJsonDrafts] = useState<Record<string, string>>({});
	const [jsonErrors, setJsonErrors] = useState<Record<string, boolean>>({});
	const required = new Set(definition?.required ?? []);
	const missingRequired = [...required].some((name) => {
		const value = parameters[name];
		const property = properties[name];
		const allowsNull = Array.isArray(property?.type)
			? property.type.includes("null")
			: property?.type === "null";
		return (
			value === undefined || value === "" || (value === null && !allowsNull)
		);
	});
	const valid = !missingRequired && !Object.values(jsonErrors).some(Boolean);

	useEffect(() => {
		onValidityChange(valid);
	}, [onValidityChange, valid]);

	const update = useCallback(
		(name: string, value: unknown) =>
			onChange({ ...parameters, [name]: value }),
		[onChange, parameters],
	);

	if (Object.keys(properties).length === 0) return null;

	return (
		<div className="space-y-3">
			<div>
				<Label>Parameters</Label>
				<p className="text-xs text-muted-foreground">
					Values are validated against the saved action contract.
				</p>
			</div>
			<div className="space-y-3 rounded-lg border p-3">
				{Object.entries(properties).map(([name, property]) => {
					const type = parameterType(property);
					const fieldId = `ontology-action-${actionId}-${name}`;
					const label = property.title ?? humanizeIdentifier(name);
					const requiredField = required.has(name);

					if (property.enum?.length) {
						return (
							<div key={name} className="grid gap-1.5">
								<Label htmlFor={fieldId}>
									{label}
									{requiredField ? " *" : ""}
								</Label>
								<Select
									disabled={disabled}
									value={
										parameters[name] === undefined
											? undefined
											: String(parameters[name])
									}
									onValueChange={(value) =>
										update(
											name,
											property.enum?.find(
												(option) => String(option) === value,
											) ?? value,
										)
									}
								>
									<SelectTrigger id={fieldId}>
										<SelectValue
											placeholder={`Choose ${label.toLowerCase()}`}
										/>
									</SelectTrigger>
									<SelectContent>
										{property.enum.map((option) => (
											<SelectItem key={String(option)} value={String(option)}>
												{String(option)}
											</SelectItem>
										))}
									</SelectContent>
								</Select>
								{property.description && (
									<p className="text-xs text-muted-foreground">
										{property.description}
									</p>
								)}
							</div>
						);
					}

					if (type === "boolean") {
						return (
							<div
								key={name}
								className="flex items-center justify-between gap-4 rounded-md bg-muted/30 p-2.5"
							>
								<div>
									<Label htmlFor={fieldId}>{label}</Label>
									{property.description && (
										<p className="text-xs text-muted-foreground">
											{property.description}
										</p>
									)}
								</div>
								<Switch
									id={fieldId}
									disabled={disabled}
									checked={Boolean(parameters[name])}
									onCheckedChange={(checked) => update(name, checked)}
								/>
							</div>
						);
					}

					if (type === "array" || type === "object") {
						const draft =
							jsonDrafts[name] ??
							JSON.stringify(
								parameters[name] ?? (type === "array" ? [] : {}),
								null,
								2,
							);
						return (
							<div key={name} className="grid gap-1.5">
								<Label htmlFor={fieldId}>
									{label}
									{requiredField ? " *" : ""}
								</Label>
								<Textarea
									id={fieldId}
									disabled={disabled}
									className="min-h-24 font-mono text-xs"
									value={draft}
									onChange={(event) => {
										const nextDraft = event.target.value;
										setJsonDrafts((current) => ({
											...current,
											[name]: nextDraft,
										}));
										try {
											update(name, JSON.parse(nextDraft));
											setJsonErrors((current) => ({
												...current,
												[name]: false,
											}));
										} catch {
											setJsonErrors((current) => ({
												...current,
												[name]: true,
											}));
										}
									}}
								/>
								{jsonErrors[name] ? (
									<p role="alert" className="text-xs text-destructive">
										Enter valid JSON.
									</p>
								) : (
									property.description && (
										<p className="text-xs text-muted-foreground">
											{property.description}
										</p>
									)
								)}
							</div>
						);
					}

					return (
						<div key={name} className="grid gap-1.5">
							<Label htmlFor={fieldId}>
								{label}
								{requiredField ? " *" : ""}
							</Label>
							<Input
								id={fieldId}
								disabled={disabled}
								type={
									type === "integer" || type === "number" ? "number" : "text"
								}
								step={
									type === "integer" ? 1 : type === "number" ? "any" : undefined
								}
								value={String(parameters[name] ?? "")}
								onChange={(event) => {
									const value = event.target.value;
									update(
										name,
										type === "integer"
											? value === ""
												? ""
												: Number.parseInt(value, 10)
											: type === "number"
												? value === ""
													? ""
													: Number.parseFloat(value)
												: value,
									);
								}}
								placeholder={property.description}
							/>
							{property.description && (
								<p className="text-xs text-muted-foreground">
									{property.description}
								</p>
							)}
						</div>
					);
				})}
			</div>
			{missingRequired && (
				<p role="alert" className="text-xs text-destructive">
					Complete all required parameters.
				</p>
			)}
		</div>
	);
}

function OntologyActionDialog({
	open,
	action,
	ontology,
	objectType,
	row,
	onOpenChange,
	onInvokeAction,
	onActionApplied,
}: Readonly<{
	open: boolean;
	action: OntologyActionDefinition | null;
	ontology?: GraphOverlay;
	objectType?: NodeLabelMapping;
	row: Record<string, unknown> | null;
	onOpenChange: (open: boolean) => void;
	onInvokeAction: (
		ontologyId: string,
		actionId: string,
		payload: InvokeOntologyActionPayload,
		onStatus?: (run: OntologyActionRun) => void,
	) => Promise<OntologyActionRun>;
	onActionApplied: () => Promise<void>;
}>) {
	const [parameters, setParameters] = useState<Record<string, unknown>>(() =>
		initialActionParameters(action?.parameter_schema),
	);
	const [formValid, setFormValid] = useState(true);
	const [submitting, setSubmitting] = useState(false);
	const [run, setRun] = useState<OntologyActionRun | null>(null);
	const [error, setError] = useState<string | null>(null);
	const [idempotencyKey] = useState(createId);
	const titleProperty =
		ontology?.object_views?.find(
			(view) => view.object_type === (objectType ? objectKey(objectType) : ""),
		)?.title_property ??
		objectType?.display_column ??
		objectType?.id_column;
	const objectId = row?.[objectType?.id_column ?? ""];
	const succeeded = Boolean(run && actionSucceeded(run.status));
	const failed = Boolean(
		run &&
			!succeeded &&
			/fail|error|cancel|interrupt|timeout/i.test(run.status),
	);

	const invoke = useCallback(async () => {
		if (!action || !ontology || !objectType || objectId === undefined) return;
		setSubmitting(true);
		setRun(null);
		setError(null);
		try {
			const result = await onInvokeAction(
				ontology.id,
				action.id,
				{
					object_refs: [
						{
							object_type: objectKey(objectType),
							id: objectId,
						},
					],
					parameters,
					idempotency_key: idempotencyKey,
				},
				(nextRun) => setRun(nextRun),
			);
			setRun(result);
			if (!actionSucceeded(result.status)) {
				setError(
					result.error_message ??
						`The action ended with status ${result.status.toLowerCase()}.`,
				);
				return;
			}
			try {
				await onActionApplied();
			} catch {
				// The action succeeded; a preview refresh can be retried independently.
			}
		} catch (invokeError) {
			setRun((current) => (current?.run_id ? current : null));
			setError(
				invokeError instanceof Error
					? invokeError.message
					: "The action could not be started.",
			);
		} finally {
			setSubmitting(false);
		}
	}, [
		action,
		idempotencyKey,
		objectId,
		objectType,
		onActionApplied,
		onInvokeAction,
		ontology,
		parameters,
	]);

	return (
		<Dialog
			open={open}
			onOpenChange={(nextOpen) => {
				if (!submitting) onOpenChange(nextOpen);
			}}
		>
			<DialogContent className="max-h-[85vh] overflow-y-auto sm:max-w-lg">
				<DialogHeader>
					<DialogTitle className="flex items-center gap-2">
						<Workflow className="h-4 w-4 text-primary" />
						{action?.name ?? "Apply action"}
					</DialogTitle>
					<DialogDescription>
						{action?.description ??
							"Run this governed operation through its saved workflow binding."}
					</DialogDescription>
				</DialogHeader>
				<div className="space-y-4 py-1" aria-busy={submitting}>
					<div className="rounded-lg border bg-muted/30 p-3">
						<p className="text-[10px] font-medium uppercase tracking-wider text-muted-foreground">
							Target {objectType?.label ?? "object"}
						</p>
						<p className="mt-1 font-medium">
							{String(row?.[titleProperty ?? ""] ?? objectId ?? "Object")}
						</p>
						<p className="mt-0.5 font-mono text-[10px] text-muted-foreground">
							{String(objectId ?? "Missing object ID")}
						</p>
					</div>
					<OntologyActionParameterForm
						actionId={action?.id ?? "action"}
						schema={action?.parameter_schema}
						parameters={parameters}
						disabled={submitting || Boolean(run)}
						onChange={setParameters}
						onValidityChange={setFormValid}
					/>
					<div className="rounded-lg border border-amber-500/30 bg-amber-500/5 p-3 text-xs text-muted-foreground">
						<p className="font-medium text-foreground">
							Confirm before applying
						</p>
						<p className="mt-1">
							The server reloads this object, validates the saved contract, and
							runs only the pinned action implementation.
						</p>
					</div>
					<div aria-live="polite">
						{submitting && !succeeded && !failed && (
							<div className="flex items-center gap-2 rounded-lg bg-primary/5 p-3 text-sm">
								<Loader2 className="h-4 w-4 animate-spin text-primary" />
								<div>
									<p>
										{run?.status === "Running"
											? "Action running…"
											: "Submitting action…"}
									</p>
									{run?.run_id && (
										<p className="font-mono text-[10px] text-muted-foreground">
											Run {run.run_id}
										</p>
									)}
								</div>
							</div>
						)}
						{run && succeeded && (
							<div className="flex items-start gap-2 rounded-lg border border-emerald-500/30 bg-emerald-500/5 p-3 text-sm">
								<CheckCircle2 className="mt-0.5 h-4 w-4 text-emerald-500" />
								<div>
									<p className="font-medium">
										{succeeded
											? "Action applied"
											: humanizeIdentifier(run.status)}
									</p>
									{run.run_id && (
										<p className="font-mono text-[10px] text-muted-foreground">
											Run {run.run_id}
										</p>
									)}
								</div>
							</div>
						)}
						{error && (
							<div
								role="alert"
								className="rounded-lg border border-destructive/30 bg-destructive/5 p-3 text-sm text-destructive"
							>
								{error}
								{run?.run_id && (
									<p className="mt-1 font-mono text-[10px]">Run {run.run_id}</p>
								)}
							</div>
						)}
					</div>
				</div>
				<DialogFooter>
					<Button
						variant="ghost"
						onClick={() => onOpenChange(false)}
						disabled={submitting}
					>
						{run && !failed ? "Done" : "Cancel"}
					</Button>
					{!run && (
						<Button
							onClick={invoke}
							disabled={
								submitting || !formValid || objectId === undefined || !action
							}
						>
							{submitting ? (
								<Loader2 className="h-4 w-4 animate-spin" />
							) : (
								<Play className="h-4 w-4" />
							)}
							Confirm {action?.name ?? "action"}
						</Button>
					)}
				</DialogFooter>
			</DialogContent>
		</Dialog>
	);
}

export function OntologyModelPanel({
	ontologies,
	onCreateOntology,
	onOpenOntology,
}: Readonly<
	StudioPanelBaseProps & { onOpenOntology: (ontologyId: string) => void }
>) {
	const [selectedId, setSelectedId] = useState(ontologies[0]?.id ?? "");
	const selected =
		ontologies.find((ontology) => ontology.id === selectedId) ?? ontologies[0];
	useEffect(() => {
		if (selected) setSelectedId(selected.id);
	}, [selected]);
	if (ontologies.length === 0)
		return (
			<EmptyStudioState
				title="Build the shared model"
				description="Choose native tables and Data Studio will infer stable object identities, display fields, and foreign-key relationships."
				onCreate={onCreateOntology}
			/>
		);
	return (
		<div className="grid gap-5 xl:grid-cols-[320px_minmax(0,1fr)]">
			<div className="space-y-3">
				<div className="flex items-center justify-between">
					<div>
						<h3 className="font-semibold">Ontologies</h3>
						<p className="text-xs text-muted-foreground">
							Saved semantic contracts
						</p>
					</div>
					<Button size="sm" onClick={onCreateOntology}>
						<Plus className="h-4 w-4" /> New
					</Button>
				</div>
				{ontologies.map((ontology) => (
					<button
						type="button"
						key={ontology.id}
						onClick={() => setSelectedId(ontology.id)}
						className={`w-full rounded-xl border p-4 text-left transition-colors ${ontology.id === selected?.id ? "border-primary bg-primary/5" : "hover:bg-muted/40"}`}
					>
						<div className="flex items-start justify-between gap-2">
							<div className="min-w-0">
								<p className="truncate font-medium">{ontology.name}</p>
								<p className="mt-1 line-clamp-2 text-xs text-muted-foreground">
									{ontology.description ?? "No description yet"}
								</p>
							</div>
							{ontology.exposed ? (
								<Badge variant="secondary">Shared</Badge>
							) : (
								<Badge variant="outline">Private</Badge>
							)}
						</div>
						<div className="mt-3 flex gap-2 text-xs text-muted-foreground">
							<span>{ontology.nodes.length} objects</span>
							<span>·</span>
							<span>{ontology.edges.length} links</span>
						</div>
					</button>
				))}
			</div>
			{selected && (
				<div className="space-y-5 rounded-xl border p-5">
					<div className="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
						<div>
							<div className="flex items-center gap-2">
								<h2 className="text-lg font-semibold">{selected.name}</h2>
								{selected.bindings_enabled && (
									<Badge className="gap-1">
										<Braces className="h-3 w-3" />
										Bindings generated
									</Badge>
								)}
							</div>
							<p className="mt-1 text-sm text-muted-foreground">
								{selected.description ?? "A semantic model over project data."}
							</p>
						</div>
						<Button
							variant="outline"
							onClick={() => onOpenOntology(selected.id)}
						>
							<Network className="h-4 w-4" /> Explore data graph{" "}
							<ExternalLink className="h-3.5 w-3.5" />
						</Button>
					</div>
					<Separator />
					<div>
						<div className="mb-3 flex items-center justify-between">
							<div>
								<h3 className="text-sm font-medium">Object types</h3>
								<p className="text-xs text-muted-foreground">
									Business objects compiled from native tables
								</p>
							</div>
							<Badge variant="secondary">{selected.nodes.length}</Badge>
						</div>
						<div className="grid gap-3 md:grid-cols-2">
							{selected.nodes.map((object) => (
								<div key={objectKey(object)} className="rounded-xl border p-4">
									<div className="flex items-start gap-3">
										<span
											className="mt-1 h-3 w-3 rounded-full"
											style={{ backgroundColor: object.style.color }}
										/>
										<div className="min-w-0 flex-1">
											<div className="flex items-center justify-between gap-2">
												<p className="font-medium">{object.label}</p>
												<code className="truncate text-[10px] text-muted-foreground">
													{object.api_name ?? object.label}
												</code>
											</div>
											<p className="mt-1 text-xs text-muted-foreground">
												{object.table} · ID {object.id_column}
											</p>
											<div className="mt-3 flex flex-wrap gap-1.5">
												{object.property_columns.slice(0, 5).map((property) => (
													<Badge
														key={property.name}
														variant="outline"
														className="text-[10px]"
													>
														{humanizeIdentifier(property.name)}
													</Badge>
												))}
												{object.property_columns.length > 5 && (
													<Badge variant="outline" className="text-[10px]">
														+{object.property_columns.length - 5}
													</Badge>
												)}
											</div>
										</div>
									</div>
								</div>
							))}
						</div>
					</div>
					<div>
						<div className="mb-3 flex items-center justify-between">
							<div>
								<h3 className="text-sm font-medium">Relationships</h3>
								<p className="text-xs text-muted-foreground">
									Inferred links can be refined in the graph editor
								</p>
							</div>
							<Badge variant="secondary">{selected.edges.length}</Badge>
						</div>
						{selected.edges.length === 0 ? (
							<div className="rounded-lg border border-dashed p-5 text-center text-sm text-muted-foreground">
								No relationships inferred yet.
							</div>
						) : (
							<div className="space-y-2">
								{selected.edges.map((edge) => (
									<div
										key={edge.id ?? edge.api_name ?? edge.label}
										className="flex items-center gap-2 rounded-lg border px-3 py-2 text-sm"
									>
										<Badge variant="outline">{edge.src_label}</Badge>
										<ArrowRight className="h-3.5 w-3.5 text-muted-foreground" />
										<span className="font-medium">
											{humanizeIdentifier(edge.label)}
										</span>
										<ArrowRight className="h-3.5 w-3.5 text-muted-foreground" />
										<Badge variant="outline">{edge.dst_label}</Badge>
										<code className="ml-auto hidden text-[10px] text-muted-foreground sm:block">
											{edge.table}.{edge.dst_column}
										</code>
									</div>
								))}
							</div>
						)}
					</div>
				</div>
			)}
		</div>
	);
}

export function OntologyActionsPanel({
	ontologies,
	boards,
	onCreateOntology,
	onNeedBoards,
	onSaveActions,
}: Readonly<
	StudioPanelBaseProps & {
		boards: IBoard[];
		onNeedBoards: () => void;
		onSaveActions: (
			ontologyId: string,
			actions: OntologyActionDefinition[],
		) => Promise<void>;
	}
>) {
	const [dialogOpen, setDialogOpen] = useState(false);
	const [ontologyId, setOntologyId] = useState(ontologies[0]?.id ?? "");
	const [name, setName] = useState("");
	const [description, setDescription] = useState("");
	const [objectType, setObjectType] = useState("");
	const [boardId, setBoardId] = useState("");
	const [startNodeId, setStartNodeId] = useState("");
	const [editingActionId, setEditingActionId] = useState<string | null>(null);
	const [actionEnabled, setActionEnabled] = useState(true);
	const [allowBulk, setAllowBulk] = useState(false);
	const [saving, setSaving] = useState(false);
	const [saveError, setSaveError] = useState<string | null>(null);
	const [repairingOntologyId, setRepairingOntologyId] = useState<string | null>(
		null,
	);
	const [repairError, setRepairError] = useState<string | null>(null);
	const ontology =
		ontologies.find((item) => item.id === ontologyId) ?? ontologies[0];
	const board = boards.find((item) => item.id === boardId);
	const startNodes = board
		? Object.values(board.nodes).filter((node) => node.start)
		: [];
	const startNode = startNodes.find((node) => node.id === startNodeId);
	const inferredParameterSchema = useMemo(() => {
		const parameterPin = Object.values(startNode?.pins ?? {}).find(
			(pin) =>
				pin.name === "parameters" && pin.data_type === "Struct" && pin.schema,
		);
		if (!parameterPin?.schema) return undefined;
		try {
			const schema = JSON.parse(parameterPin.schema);
			return schema && typeof schema === "object" && !Array.isArray(schema)
				? (schema as Record<string, unknown>)
				: undefined;
		} catch {
			return undefined;
		}
	}, [startNode]);
	const allActions = ontologies.flatMap((item) =>
		(item.actions ?? []).map((action) => ({ action, ontology: item })),
	);
	const resetActionEditor = useCallback(() => {
		setEditingActionId(null);
		setName("");
		setDescription("");
		setBoardId("");
		setStartNodeId("");
		setActionEnabled(true);
		setAllowBulk(false);
	}, []);
	const openActionEditor = useCallback(
		(owner?: GraphOverlay, action?: OntologyActionDefinition) => {
			onNeedBoards();
			setSaveError(null);
			if (owner && action) {
				setEditingActionId(action.id);
				setOntologyId(owner.id);
				setObjectType(action.object_type);
				setName(action.name);
				setDescription(action.description ?? "");
				setBoardId(action.board_id);
				setStartNodeId(action.start_node_id ?? "");
				setActionEnabled(action.enabled);
				setAllowBulk(action.allow_bulk);
			} else {
				resetActionEditor();
				const initialOntology = ontologies[0];
				setOntologyId(initialOntology?.id ?? "");
				setObjectType(
					initialOntology?.nodes[0] ? objectKey(initialOntology.nodes[0]) : "",
				);
			}
			setDialogOpen(true);
		},
		[onNeedBoards, ontologies, resetActionEditor],
	);

	useEffect(() => {
		if (!ontology) return;
		setOntologyId(ontology.id);
		if (!objectType && ontology.nodes[0])
			setObjectType(objectKey(ontology.nodes[0]));
	}, [objectType, ontology]);

	const saveAction = useCallback(async () => {
		if (!ontology || !name.trim() || !objectType || !boardId || !startNodeId)
			return;
		setSaving(true);
		setSaveError(null);
		try {
			const previous = (ontology.actions ?? []).find(
				(action) => action.id === editingActionId,
			);
			const version = board?.version;
			const boardVersion =
				Array.isArray(version) && version.length === 3
					? ([Number(version[0]), Number(version[1]), Number(version[2])] as [
							number,
							number,
							number,
						])
					: previous?.board_id === boardId
						? previous.board_version
						: undefined;
			const nextAction: OntologyActionDefinition = {
				...previous,
				id: editingActionId ?? createId(),
				name: name.trim(),
				description: description.trim() || undefined,
				object_type: objectType,
				board_id: boardId,
				board_version: boardVersion,
				start_node_id: startNodeId,
				enabled: actionEnabled,
				allow_bulk: allowBulk,
				parameter_schema:
					inferredParameterSchema ??
					(previous?.board_id === boardId &&
					previous.start_node_id === startNodeId
						? previous.parameter_schema
						: undefined),
			};
			const nextActions = editingActionId
				? (ontology.actions ?? []).map((action) =>
						action.id === editingActionId ? nextAction : action,
					)
				: [...(ontology.actions ?? []), nextAction];
			await onSaveActions(ontology.id, nextActions);
			setDialogOpen(false);
			resetActionEditor();
		} catch (error) {
			setSaveError(
				error instanceof Error
					? error.message
					: "The ontology action could not be saved.",
			);
		} finally {
			setSaving(false);
		}
	}, [
		board?.version,
		boardId,
		actionEnabled,
		allowBulk,
		description,
		editingActionId,
		inferredParameterSchema,
		name,
		objectType,
		onSaveActions,
		ontology,
		resetActionEditor,
		startNodeId,
	]);

	const repairActionBindings = useCallback(
		async (owner: GraphOverlay) => {
			setRepairingOntologyId(owner.id);
			setRepairError(null);
			try {
				await onSaveActions(owner.id, owner.actions ?? []);
			} catch (error) {
				setRepairError(
					error instanceof Error
						? error.message
						: "The action binding could not be refreshed.",
				);
			} finally {
				setRepairingOntologyId(null);
			}
		},
		[onSaveActions],
	);
	const removeAction = useCallback(
		async (owner: GraphOverlay, actionId: string) => {
			setRepairingOntologyId(owner.id);
			setRepairError(null);
			try {
				await onSaveActions(
					owner.id,
					(owner.actions ?? []).filter((action) => action.id !== actionId),
				);
			} catch (error) {
				setRepairError(
					error instanceof Error
						? error.message
						: "The action could not be removed.",
				);
			} finally {
				setRepairingOntologyId(null);
			}
		},
		[onSaveActions],
	);

	if (ontologies.length === 0)
		return (
			<EmptyStudioState
				title="Actions start with objects"
				description="Create an ontology first, then bind object-level operations to typed board entry nodes."
				onCreate={onCreateOntology}
			/>
		);
	return (
		<div className="space-y-5">
			<div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
				<div>
					<h3 className="font-semibold">Ontology actions</h3>
					<p className="text-sm text-muted-foreground">
						Governed object operations backed by a pinned board and start node.
					</p>
				</div>
				<Button onClick={() => openActionEditor()}>
					<Plus className="h-4 w-4" /> Define action
				</Button>
			</div>
			{allActions.length === 0 ? (
				<div className="rounded-xl border border-dashed p-10 text-center">
					<div className="mx-auto mb-3 w-fit rounded-xl bg-primary/10 p-3 text-primary">
						<Workflow className="h-5 w-5" />
					</div>
					<p className="font-medium">No actions defined</p>
					<p className="mt-1 text-sm text-muted-foreground">
						Bind an object operation to an existing board entry node.
					</p>
				</div>
			) : (
				<div className="space-y-3">
					{repairError && (
						<p role="alert" className="text-sm text-destructive">
							{repairError}
						</p>
					)}
					<div className="grid gap-3 lg:grid-cols-2">
						{allActions.map(({ action, ontology: owner }) => (
							<Card key={`${owner.id}:${action.id}`}>
								<CardContent className="p-4">
									<div className="flex items-start justify-between gap-3">
										<div className="flex gap-3">
											<div className="rounded-lg bg-primary/10 p-2 text-primary">
												<Workflow className="h-4 w-4" />
											</div>
											<div>
												<p className="font-medium">{action.name}</p>
												<p className="text-xs text-muted-foreground">
													{owner.name} ·{" "}
													{owner.nodes.find(
														(item) => objectKey(item) === action.object_type,
													)?.label ?? action.object_type}
												</p>
											</div>
										</div>
										<Badge variant={action.enabled ? "secondary" : "outline"}>
											{action.enabled ? "Active" : "Disabled"}
										</Badge>
									</div>
									{action.description && (
										<p className="mt-3 text-sm text-muted-foreground">
											{action.description}
										</p>
									)}
									<div className="mt-4 grid grid-cols-2 gap-2 text-xs">
										<div className="rounded-lg bg-muted/40 p-2">
											<span className="text-muted-foreground">Board</span>
											<p className="mt-0.5 truncate font-medium">
												{boards.find((item) => item.id === action.board_id)
													?.name ?? action.board_id}
											</p>
										</div>
										<div className="rounded-lg bg-muted/40 p-2">
											<span className="text-muted-foreground">Binding</span>
											<p className="mt-0.5 truncate font-mono text-[10px]">
												{action.start_node_id ?? "Not set"}
											</p>
										</div>
									</div>
									<div className="mt-3 flex justify-end gap-1">
										<Button
											variant="ghost"
											size="sm"
											disabled={repairingOntologyId === owner.id}
											onClick={() => openActionEditor(owner, action)}
										>
											Edit
										</Button>
										<Button
											variant="ghost"
											size="sm"
											disabled={repairingOntologyId === owner.id}
											onClick={() => repairActionBindings(owner)}
										>
											{repairingOntologyId === owner.id && (
												<Loader2 className="h-3.5 w-3.5 animate-spin" />
											)}
											{action.event_id ? "Refresh binding" : "Repair binding"}
										</Button>
										<AlertDialog>
											<AlertDialogTrigger asChild>
												<Button
													variant="ghost"
													size="sm"
													disabled={repairingOntologyId === owner.id}
													className="text-destructive hover:text-destructive"
												>
													Remove
												</Button>
											</AlertDialogTrigger>
											<AlertDialogContent>
												<AlertDialogHeader>
													<AlertDialogTitle>
														Remove {action.name}?
													</AlertDialogTitle>
													<AlertDialogDescription>
														The generated project binding and its managed event
														will be removed. Boards already using the binding
														will need to be updated.
													</AlertDialogDescription>
												</AlertDialogHeader>
												<AlertDialogFooter>
													<AlertDialogCancel>Keep action</AlertDialogCancel>
													<AlertDialogAction
														className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
														onClick={() => void removeAction(owner, action.id)}
													>
														Remove action
													</AlertDialogAction>
												</AlertDialogFooter>
											</AlertDialogContent>
										</AlertDialog>
									</div>
								</CardContent>
							</Card>
						))}
					</div>
				</div>
			)}
			<Dialog
				open={dialogOpen}
				onOpenChange={(open) => {
					if (!open && saving) return;
					if (open) setSaveError(null);
					else resetActionEditor();
					setDialogOpen(open);
				}}
			>
				<DialogContent className="max-w-xl">
					<DialogHeader>
						<DialogTitle>
							{editingActionId
								? "Edit ontology action"
								: "Define an ontology action"}
						</DialogTitle>
						<DialogDescription>
							Choose the object and the exact board entry that implements this
							operation.
						</DialogDescription>
					</DialogHeader>
					<div className="grid gap-4 py-2">
						<div className="grid gap-1.5">
							<Label>Ontology</Label>
							<Select
								value={ontology?.id}
								disabled={Boolean(editingActionId)}
								onValueChange={(value) => {
									setOntologyId(value);
									setObjectType("");
								}}
							>
								<SelectTrigger>
									<SelectValue />
								</SelectTrigger>
								<SelectContent>
									{ontologies.map((item) => (
										<SelectItem key={item.id} value={item.id}>
											{item.name}
										</SelectItem>
									))}
								</SelectContent>
							</Select>
						</div>
						<div className="grid gap-1.5">
							<Label>Object type</Label>
							<Select value={objectType} onValueChange={setObjectType}>
								<SelectTrigger>
									<SelectValue placeholder="Select object" />
								</SelectTrigger>
								<SelectContent>
									{ontology?.nodes.map((item) => (
										<SelectItem key={objectKey(item)} value={objectKey(item)}>
											{item.label}
										</SelectItem>
									))}
								</SelectContent>
							</Select>
						</div>
						<div className="grid gap-1.5">
							<Label>Action name</Label>
							<Input
								value={name}
								onChange={(event) => setName(event.target.value)}
								placeholder="Approve order"
							/>
						</div>
						<div className="grid gap-1.5">
							<Label>Description</Label>
							<Textarea
								value={description}
								onChange={(event) => setDescription(event.target.value)}
								placeholder="What changes when this action succeeds?"
							/>
						</div>
						<div className="grid gap-3 sm:grid-cols-2">
							<div className="grid gap-1.5">
								<Label>Board</Label>
								<Select
									value={boardId}
									onValueChange={(value) => {
										setBoardId(value);
										setStartNodeId("");
									}}
								>
									<SelectTrigger>
										<SelectValue placeholder="Select board" />
									</SelectTrigger>
									<SelectContent>
										{boards.map((item) => (
											<SelectItem key={item.id} value={item.id}>
												{item.name}
											</SelectItem>
										))}
									</SelectContent>
								</Select>
							</div>
							<div className="grid gap-1.5">
								<Label>Start node</Label>
								<Select
									value={startNodeId}
									onValueChange={setStartNodeId}
									disabled={!boardId}
								>
									<SelectTrigger>
										<SelectValue placeholder="Select entry" />
									</SelectTrigger>
									<SelectContent>
										{startNodes.map((node) => (
											<SelectItem key={node.id} value={node.id}>
												{node.friendly_name}
											</SelectItem>
										))}
									</SelectContent>
								</Select>
							</div>
						</div>
						<div className="rounded-lg border bg-muted/30 p-3 text-xs text-muted-foreground">
							<div className="flex items-center gap-1.5 font-medium text-foreground">
								<ShieldCheck className="h-3.5 w-3.5" />
								Pinned implementation
							</div>
							<p className="mt-1">
								The action resolves this saved binding server-side; object views
								and generated project nodes never trust an arbitrary board
								target.
							</p>
							{inferredParameterSchema && (
								<p className="mt-2 flex items-center gap-1.5 font-medium text-foreground">
									<CheckCircle2 className="h-3.5 w-3.5 text-emerald-500" />
									Typed parameters detected from this entry node.
								</p>
							)}
						</div>
						{saveError && (
							<p
								role="alert"
								className="rounded-lg border border-destructive/30 bg-destructive/5 p-3 text-sm text-destructive"
							>
								{saveError}
							</p>
						)}
					</div>
					<div className="grid gap-2 sm:grid-cols-2">
						<div className="flex items-center justify-between gap-3 rounded-lg border p-3">
							<div>
								<Label htmlFor="ontology-action-enabled">Enabled</Label>
								<p className="text-xs text-muted-foreground">
									Visible in object views
								</p>
							</div>
							<Switch
								id="ontology-action-enabled"
								checked={actionEnabled}
								onCheckedChange={setActionEnabled}
							/>
						</div>
						<div className="flex items-center justify-between gap-3 rounded-lg border p-3">
							<div>
								<Label htmlFor="ontology-action-bulk">Allow bulk</Label>
								<p className="text-xs text-muted-foreground">
									Up to 100 objects per run
								</p>
							</div>
							<Switch
								id="ontology-action-bulk"
								checked={allowBulk}
								onCheckedChange={setAllowBulk}
							/>
						</div>
					</div>
					<DialogFooter>
						<Button
							variant="ghost"
							onClick={() => setDialogOpen(false)}
							disabled={saving}
						>
							Cancel
						</Button>
						<Button
							onClick={saveAction}
							disabled={
								!name.trim() ||
								!objectType ||
								!boardId ||
								!startNodeId ||
								saving
							}
						>
							{saving && <Loader2 className="h-4 w-4 animate-spin" />}
							{editingActionId ? "Save changes" : "Save action"}
						</Button>
					</DialogFooter>
				</DialogContent>
			</Dialog>
		</div>
	);
}

function RemoteOntologyUninstallButton({
	ontologyName,
	sourceName,
	disabled,
	loading,
	onConfirm,
}: Readonly<{
	ontologyName: string;
	sourceName: string;
	disabled: boolean;
	loading: boolean;
	onConfirm: () => Promise<void>;
}>) {
	return (
		<AlertDialog>
			<AlertDialogTrigger asChild>
				<Button variant="ghost" size="sm" disabled={disabled}>
					{loading && <Loader2 className="h-3.5 w-3.5 animate-spin" />}
					Uninstall
				</Button>
			</AlertDialogTrigger>
			<AlertDialogContent>
				<AlertDialogHeader>
					<AlertDialogTitle>Uninstall remote ontology?</AlertDialogTitle>
					<AlertDialogDescription>
						This removes the installed {ontologyName} contract from {sourceName}
						. Existing board nodes that use its generated bindings will stop
						resolving until the ontology is installed again.
					</AlertDialogDescription>
				</AlertDialogHeader>
				<AlertDialogFooter>
					<AlertDialogCancel>Keep installed</AlertDialogCancel>
					<AlertDialogAction
						className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
						onClick={() => void onConfirm()}
					>
						Uninstall bindings
					</AlertDialogAction>
				</AlertDialogFooter>
			</AlertDialogContent>
		</AlertDialog>
	);
}

export function OntologySharingPanel({
	ontologies,
	connections,
	remoteConnections,
	installedOntologies,
	installedOntologiesLoading,
	installedOntologiesError,
	onCreateOntology,
	onUpdateOntology,
	onLoadRemoteOntologies,
	onInstallRemoteOntology,
	onUninstallRemoteOntology,
}: Readonly<
	StudioPanelBaseProps & {
		connections: IAppConnection[];
		remoteConnections: IAppConnection[];
		installedOntologies: RemoteOntologyImport[];
		installedOntologiesLoading: boolean;
		installedOntologiesError?: string;
		onUpdateOntology: (
			ontologyId: string,
			patch: Partial<Pick<GraphOverlay, "exposed" | "bindings_enabled">>,
		) => Promise<void>;
		onLoadRemoteOntologies: (targetAppId: string) => Promise<GraphOverlay[]>;
		onInstallRemoteOntology: (
			targetAppId: string,
			ontologyId: string,
		) => Promise<void>;
		onUninstallRemoteOntology: (
			targetAppId: string,
			ontologyId: string,
		) => Promise<void>;
	}
>) {
	const savingOntologyIdsRef = useRef(new Set<string>());
	const [savingOntologyIds, setSavingOntologyIds] = useState<Set<string>>(
		() => new Set(),
	);
	const [sharingErrors, setSharingErrors] = useState<Record<string, string>>(
		{},
	);
	const loadingConnectionIdsRef = useRef(new Set<string>());
	const remoteLoadGenerationRef = useRef<Record<string, number>>({});
	const [loadingConnectionIds, setLoadingConnectionIds] = useState<Set<string>>(
		() => new Set(),
	);
	const [remoteOntologies, setRemoteOntologies] = useState<
		Record<string, GraphOverlay[]>
	>({});
	const [remoteErrors, setRemoteErrors] = useState<Record<string, string>>({});
	const [mutatingImportId, setMutatingImportId] = useState<string | null>(null);
	const [importError, setImportError] = useState<string | null>(null);
	const update = useCallback(
		async (
			ontology: GraphOverlay,
			patch: Partial<Pick<GraphOverlay, "exposed" | "bindings_enabled">>,
		) => {
			if (savingOntologyIdsRef.current.has(ontology.id)) return;
			savingOntologyIdsRef.current.add(ontology.id);
			setSavingOntologyIds(new Set(savingOntologyIdsRef.current));
			setSharingErrors((current) => {
				const next = { ...current };
				delete next[ontology.id];
				return next;
			});
			try {
				await onUpdateOntology(ontology.id, patch);
			} catch (error) {
				setSharingErrors((current) => ({
					...current,
					[ontology.id]:
						error instanceof Error
							? error.message
							: "Could not update ontology sharing.",
				}));
			} finally {
				savingOntologyIdsRef.current.delete(ontology.id);
				setSavingOntologyIds(new Set(savingOntologyIdsRef.current));
			}
		},
		[onUpdateOntology],
	);
	const installedStateUnavailable =
		installedOntologiesLoading || Boolean(installedOntologiesError);
	const discoverRemoteOntologies = useCallback(
		async (connection: IAppConnection) => {
			if (loadingConnectionIdsRef.current.has(connection.id)) return;
			loadingConnectionIdsRef.current.add(connection.id);
			setLoadingConnectionIds(new Set(loadingConnectionIdsRef.current));
			const generation =
				(remoteLoadGenerationRef.current[connection.id] ?? 0) + 1;
			remoteLoadGenerationRef.current[connection.id] = generation;
			setRemoteErrors((current) => {
				const next = { ...current };
				delete next[connection.id];
				return next;
			});
			try {
				const contracts = await onLoadRemoteOntologies(
					connection.target_app_id,
				);
				if (remoteLoadGenerationRef.current[connection.id] === generation) {
					setRemoteOntologies((current) => ({
						...current,
						[connection.id]: contracts,
					}));
				}
			} catch (error) {
				if (remoteLoadGenerationRef.current[connection.id] === generation) {
					setRemoteErrors((current) => ({
						...current,
						[connection.id]:
							error instanceof Error
								? error.message
								: "Could not discover remote ontologies.",
					}));
				}
			} finally {
				if (remoteLoadGenerationRef.current[connection.id] === generation) {
					loadingConnectionIdsRef.current.delete(connection.id);
					setLoadingConnectionIds(new Set(loadingConnectionIdsRef.current));
				}
			}
		},
		[onLoadRemoteOntologies],
	);
	const mutateImport = useCallback(
		async (
			targetAppId: string,
			ontologyId: string,
			operation: "install" | "uninstall",
		) => {
			const importId = `${targetAppId}::${ontologyId}`;
			setMutatingImportId(importId);
			setImportError(null);
			try {
				if (operation === "install") {
					await onInstallRemoteOntology(targetAppId, ontologyId);
				} else {
					await onUninstallRemoteOntology(targetAppId, ontologyId);
				}
			} catch (error) {
				setImportError(
					error instanceof Error
						? error.message
						: "Could not update the remote ontology binding.",
				);
			} finally {
				setMutatingImportId(null);
			}
		},
		[onInstallRemoteOntology, onUninstallRemoteOntology],
	);
	return (
		<div className="grid gap-5 xl:grid-cols-[minmax(0,1.4fr)_minmax(320px,0.8fr)]">
			<div className="space-y-3">
				<div>
					<h3 className="font-semibold">Ontology contracts</h3>
					<p className="text-sm text-muted-foreground">
						Exposure controls discovery; existing connection roles still govern
						every data read and action.
					</p>
				</div>
				{ontologies.length === 0 && (
					<EmptyStudioState
						title="Nothing to expose yet"
						description="Set up a local ontology, or install a contract from a connected project."
						onCreate={onCreateOntology}
					/>
				)}
				{ontologies.map((ontology) => (
					<Card key={ontology.id}>
						<CardContent className="space-y-4 p-4">
							<div className="flex items-start justify-between gap-3">
								<div className="flex gap-3">
									<div className="rounded-lg bg-primary/10 p-2 text-primary">
										<Share2 className="h-4 w-4" />
									</div>
									<div>
										<p className="font-medium">{ontology.name}</p>
										<p className="text-xs text-muted-foreground">
											{ontology.nodes.length} object contracts ·{" "}
											{ontology.actions?.length ?? 0} actions
										</p>
									</div>
								</div>
								{savingOntologyIds.has(ontology.id) && (
									<Loader2 className="h-4 w-4 animate-spin text-muted-foreground" />
								)}
							</div>
							{sharingErrors[ontology.id] && (
								<p
									role="alert"
									className="rounded-lg border border-destructive/30 bg-destructive/5 p-3 text-sm text-destructive"
								>
									{sharingErrors[ontology.id]}
								</p>
							)}
							<Separator />
							<div className="flex items-center justify-between gap-4">
								<div>
									<Label htmlFor={`expose-${ontology.id}`}>
										Expose to connected projects
									</Label>
									<p className="text-xs text-muted-foreground">
										Allows permitted projects to discover this contract.
									</p>
								</div>
								<Switch
									id={`expose-${ontology.id}`}
									checked={ontology.exposed}
									disabled={savingOntologyIds.has(ontology.id)}
									onCheckedChange={(checked) =>
										update(ontology, { exposed: checked })
									}
								/>
							</div>
							<div className="flex items-center justify-between gap-4">
								<div>
									<Label htmlFor={`bindings-${ontology.id}`}>
										Generate board bindings
									</Label>
									<p className="text-xs text-muted-foreground">
										Adds object and action bindings to this project&apos;s node
										catalog.
									</p>
								</div>
								<Switch
									id={`bindings-${ontology.id}`}
									checked={ontology.bindings_enabled}
									disabled={savingOntologyIds.has(ontology.id)}
									onCheckedChange={(checked) =>
										update(ontology, { bindings_enabled: checked })
									}
								/>
							</div>
						</CardContent>
					</Card>
				))}
			</div>
			<div className="space-y-4">
				<Card>
					<CardHeader>
						<CardTitle className="flex items-center gap-2 text-base">
							<FileKey className="h-4 w-4" />
							Connected projects
						</CardTitle>
					</CardHeader>
					<CardContent className="space-y-3">
						{connections.filter((connection) => connection.status === "ACTIVE")
							.length === 0 ? (
							<div className="rounded-lg border border-dashed p-5 text-center text-sm text-muted-foreground">
								No active app connections. Create one from Team → Connections.
							</div>
						) : (
							connections
								.filter((connection) => connection.status === "ACTIVE")
								.map((connection) => (
									<div
										key={connection.id}
										className="flex items-center gap-3 rounded-lg border p-3"
									>
										<div className="rounded-lg bg-muted p-2">
											<Database className="h-4 w-4" />
										</div>
										<div className="min-w-0 flex-1">
											<p className="truncate text-sm font-medium">
												{connection.app_name ??
													connection.source_app_id ??
													connection.target_app_id}
											</p>
											<p className="text-xs text-muted-foreground">
												{connection.role_name ?? "Connection role"}
											</p>
										</div>
										<CheckCircle2 className="h-4 w-4 text-emerald-500" />
									</div>
								))
						)}
						<div className="rounded-lg bg-muted/40 p-3 text-xs text-muted-foreground">
							<p className="font-medium text-foreground">Defense in depth</p>
							<p className="mt-1">
								ReadDatabase controls object access. Any event execution remains
								separately permissioned; exposure never widens the assigned
								role.
							</p>
						</div>
					</CardContent>
				</Card>
				{importError && (
					<p
						role="alert"
						className="rounded-lg border border-destructive/30 bg-destructive/5 p-3 text-sm text-destructive"
					>
						{importError}
					</p>
				)}
				{installedOntologiesLoading && (
					<Card>
						<CardContent
							className="flex items-center gap-2 p-4 text-sm text-muted-foreground"
							aria-live="polite"
						>
							<Loader2 className="h-4 w-4 animate-spin" />
							Loading installed ontology bindings…
						</CardContent>
					</Card>
				)}
				{installedOntologiesError && (
					<Card>
						<CardContent className="p-4">
							<p
								role="alert"
								className="rounded-lg border border-destructive/30 bg-destructive/5 p-3 text-sm text-destructive"
							>
								Could not load installed ontology bindings:{" "}
								{installedOntologiesError}
							</p>
						</CardContent>
					</Card>
				)}
				{!installedStateUnavailable && installedOntologies.length > 0 && (
					<Card>
						<CardHeader>
							<CardTitle className="flex items-center gap-2 text-base">
								<Layers3 className="h-4 w-4" />
								Installed bindings
							</CardTitle>
						</CardHeader>
						<CardContent className="space-y-2">
							{installedOntologies.map((installed) => {
								const importId = `${installed.target_app_id}::${installed.remote_ontology_id}`;
								const source = remoteConnections.find(
									(connection) =>
										connection.target_app_id === installed.target_app_id,
								);
								const sourceName = source?.app_name ?? installed.target_app_id;
								return (
									<div
										key={installed.id}
										className="flex items-center gap-2 rounded-lg border p-3"
									>
										<div className="min-w-0 flex-1">
											<p className="truncate text-sm font-medium">
												{installed.contract.name}
											</p>
											<p className="truncate text-xs text-muted-foreground">
												Remote · {sourceName} ·{" "}
												{installed.contract.nodes.length} objects
											</p>
										</div>
										<Badge variant="secondary">Installed</Badge>
										<RemoteOntologyUninstallButton
											ontologyName={installed.contract.name}
											sourceName={sourceName}
											disabled={Boolean(mutatingImportId)}
											loading={mutatingImportId === importId}
											onConfirm={() =>
												mutateImport(
													installed.target_app_id,
													installed.remote_ontology_id,
													"uninstall",
												)
											}
										/>
									</div>
								);
							})}
						</CardContent>
					</Card>
				)}
				<Card>
					<CardHeader>
						<CardTitle className="flex items-center gap-2 text-base">
							<Network className="h-4 w-4" />
							Available remote ontologies
						</CardTitle>
					</CardHeader>
					<CardContent className="space-y-3">
						{remoteConnections.filter(
							(connection) => connection.status === "ACTIVE",
						).length === 0 ? (
							<p className="text-sm text-muted-foreground">
								No outgoing project connections can expose contracts yet.
							</p>
						) : (
							remoteConnections
								.filter((connection) => connection.status === "ACTIVE")
								.map((connection) => {
									const contracts = remoteOntologies[connection.id];
									const loadError = remoteErrors[connection.id];
									return (
										<div key={connection.id} className="rounded-lg border p-3">
											<div className="flex items-center gap-3">
												<div className="min-w-0 flex-1">
													<p className="truncate text-sm font-medium">
														{connection.app_name ?? connection.target_app_id}
													</p>
													<p className="text-xs text-muted-foreground">
														Only explicitly exposed contracts are returned.
													</p>
												</div>
												<Button
													variant="outline"
													size="sm"
													disabled={loadingConnectionIds.has(connection.id)}
													onClick={() => discoverRemoteOntologies(connection)}
												>
													{loadingConnectionIds.has(connection.id) && (
														<Loader2 className="h-3.5 w-3.5 animate-spin" />
													)}
													{contracts ? "Refresh" : "Discover"}
												</Button>
											</div>
											{loadError && (
												<p className="mt-2 text-xs text-destructive">
													{loadError}
												</p>
											)}
											{contracts && (
												<div className="mt-3 space-y-2 border-t pt-3">
													{contracts.length === 0 ? (
														<p className="text-xs text-muted-foreground">
															No contracts are exposed by this project.
														</p>
													) : (
														contracts.map((contract) => {
															const installed = installedStateUnavailable
																? undefined
																: installedOntologies.find(
																		(item) =>
																			item.target_app_id ===
																				connection.target_app_id &&
																			item.remote_ontology_id === contract.id,
																	);
															const importId = `${connection.target_app_id}::${contract.id}`;
															const updating = mutatingImportId === importId;
															const updateAvailable = Boolean(
																installed &&
																	installed.source_updated_at !==
																		contract.updated_at,
															);
															return (
																<div
																	key={contract.id}
																	className="space-y-2 rounded-md bg-muted/40 px-2.5 py-2"
																>
																	<div className="flex items-start justify-between gap-2">
																		<div className="min-w-0">
																			<p className="truncate text-xs font-medium">
																				{contract.name}
																			</p>
																			<p className="text-[10px] text-muted-foreground">
																				{contract.nodes.length} object types ·
																				object bindings only
																			</p>
																		</div>
																		<Badge
																			variant={
																				installed ? "secondary" : "outline"
																			}
																		>
																			{installedOntologiesLoading
																				? "Checking installation"
																				: installedOntologiesError
																					? "Status unavailable"
																					: updateAvailable
																						? "Update available"
																						: installed
																							? "Installed"
																							: "Remote"}
																		</Badge>
																	</div>
																	<div className="flex items-center justify-end gap-2">
																		{installed && (
																			<RemoteOntologyUninstallButton
																				ontologyName={contract.name}
																				sourceName={
																					connection.app_name ??
																					connection.target_app_id
																				}
																				disabled={Boolean(mutatingImportId)}
																				loading={updating}
																				onConfirm={() =>
																					mutateImport(
																						connection.target_app_id,
																						contract.id,
																						"uninstall",
																					)
																				}
																			/>
																		)}
																		<Button
																			variant={
																				installed ? "outline" : "default"
																			}
																			size="sm"
																			disabled={
																				Boolean(mutatingImportId) ||
																				installedStateUnavailable
																			}
																			onClick={() =>
																				mutateImport(
																					connection.target_app_id,
																					contract.id,
																					"install",
																				)
																			}
																		>
																			{updating && (
																				<Loader2 className="h-3.5 w-3.5 animate-spin" />
																			)}
																			{installed
																				? "Refresh object bindings"
																				: "Install object bindings"}
																		</Button>
																	</div>
																</div>
															);
														})
													)}
												</div>
											)}
										</div>
									);
								})
						)}
					</CardContent>
				</Card>
			</div>
		</div>
	);
}
