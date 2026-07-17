"use client";

import { createId } from "@paralleldrive/cuid2";
import {
	ArrowRight,
	Check,
	Database,
	Loader2,
	Sparkles,
	Trash2,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { useBackend } from "../../../state/backend-state";
import type {
	CreateOverlayPayload,
	EdgeLabelMapping,
	GraphOverlay,
	NodeLabelMapping,
	PropertyColumn,
	ValidationResult,
} from "../../../state/backend-state/graph-state";
import { Badge } from "../../ui/badge";
import { Button } from "../../ui/button";
import { Checkbox } from "../../ui/checkbox";
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
import type { ArrowSchemaJSON } from "../../ui/lance-viewer";
import { ScrollArea } from "../../ui/scroll-area";
import {
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "../../ui/select";

export interface DataStudioTableInfo {
	name: string;
	userScoped?: boolean;
}

interface InferredObject extends NodeLabelMapping {
	columns: PropertyColumn[];
	api_name_touched?: boolean;
}

interface OntologySetupDialogProps {
	open: boolean;
	onOpenChange: (open: boolean) => void;
	tables: DataStudioTableInfo[];
	loadSchema: (table: DataStudioTableInfo) => Promise<ArrowSchemaJSON>;
	onCreate: (payload: CreateOverlayPayload) => Promise<void>;
	appId?: string;
	userScoped?: boolean;
}

const STEPS = ["sources", "objects", "relationships", "publish"] as const;
type Step = (typeof STEPS)[number];

const OBJECT_COLORS = [
	"#2563eb",
	"#7c3aed",
	"#0891b2",
	"#059669",
	"#d97706",
	"#dc2626",
] as const;

function dataTypeToString(dataType: unknown): string {
	if (typeof dataType === "string") return dataType;
	if (dataType && typeof dataType === "object") {
		return Object.keys(dataType)[0] ?? "Unknown";
	}
	return "Unknown";
}

function schemaColumns(schema: ArrowSchemaJSON): PropertyColumn[] {
	return (schema.fields ?? []).map((rawField: unknown) => {
		const field = rawField as {
			name?: unknown;
			data_type?: unknown;
			nullable?: boolean;
		};
		return {
			name: String(field.name ?? ""),
			data_type: dataTypeToString(field.data_type),
			nullable: field.nullable ?? true,
		};
	});
}

function singularize(value: string): string {
	if (value.endsWith("ies") && value.length > 3)
		return `${value.slice(0, -3)}y`;
	if (value.endsWith("sses")) return value.slice(0, -2);
	if (value.endsWith("s") && !value.endsWith("ss")) return value.slice(0, -1);
	return value;
}

function apiName(value: string): string {
	return singularize(
		value
			.trim()
			.replace(/([a-z0-9])([A-Z])/g, "$1_$2")
			.replace(/[^a-zA-Z0-9]+/g, "_")
			.replace(/^_+|_+$/g, "")
			.toLowerCase(),
	);
}

function displayName(value: string): string {
	return apiName(value)
		.split("_")
		.filter(Boolean)
		.map((part) => `${part.charAt(0).toUpperCase()}${part.slice(1)}`)
		.join(" ");
}

function inferIdColumn(columns: PropertyColumn[], typeApiName: string): string {
	const names = columns.map((column) => column.name);
	return (
		names.find((name) => name.toLowerCase() === "id") ??
		names.find((name) => name.toLowerCase() === `${typeApiName}_id`) ??
		names.find((name) => name.toLowerCase().endsWith("_id")) ??
		names[0] ??
		""
	);
}

function inferDisplayColumn(
	columns: PropertyColumn[],
	idColumn: string,
): string | undefined {
	const preferred = ["name", "title", "label", "display_name", "email", "code"];
	for (const candidate of preferred) {
		const match = columns.find(
			(column) => column.name.toLowerCase() === candidate,
		);
		if (match) return match.name;
	}
	return columns.find((column) => column.name !== idColumn)?.name;
}

function inferEdges(objects: InferredObject[]): EdgeLabelMapping[] {
	const edges: EdgeLabelMapping[] = [];
	for (const source of objects) {
		for (const column of source.columns) {
			const columnName = column.name.toLowerCase();
			if (!columnName.endsWith("_id") || column.name === source.id_column) {
				continue;
			}
			const targetApiName = columnName.slice(0, -3);
			const target = objects.find(
				(object) => object.api_name === targetApiName,
			);
			if (!target || target.id === source.id) continue;
			edges.push({
				id: createId(),
				api_name: `${source.api_name}_to_${target.api_name}`,
				label: `has_${target.api_name}`,
				table: source.table,
				src_column: source.id_column,
				dst_column: column.name,
				src_label: source.label,
				dst_label: target.label,
				property_columns: [],
				style: {
					color: source.style.color,
					icon: "arrow-right",
					size: { mode: "fixed", value: 2 },
				},
			});
		}
	}
	return edges;
}

function edgeKey(edge: EdgeLabelMapping): string {
	return `${edge.src_label}::${edge.dst_label}::${edge.dst_column}`;
}

export function OntologySetupDialog({
	open,
	onOpenChange,
	tables,
	loadSchema,
	onCreate,
	appId,
	userScoped,
}: Readonly<OntologySetupDialogProps>) {
	const backend = useBackend();
	const graphState = backend.graphState;
	const projectTables = useMemo(
		() => tables.filter((table) => !table.userScoped),
		[tables],
	);
	const [step, setStep] = useState<Step>("sources");
	const [name, setName] = useState("");
	const [description, setDescription] = useState("");
	const [selectedTables, setSelectedTables] = useState<Set<string>>(new Set());
	const [objects, setObjects] = useState<InferredObject[]>([]);
	const [edges, setEdges] = useState<EdgeLabelMapping[]>([]);
	const [removedEdgeKeys, setRemovedEdgeKeys] = useState<Set<string>>(
		new Set(),
	);
	const [loadingSchemas, setLoadingSchemas] = useState(false);
	const [submitting, setSubmitting] = useState(false);
	const [error, setError] = useState<string | null>(null);
	const [validation, setValidation] = useState<ValidationResult | null>(null);
	const [validationWarning, setValidationWarning] = useState<string | null>(
		null,
	);

	useEffect(() => {
		if (open) return;
		setStep("sources");
		setName("");
		setDescription("");
		setSelectedTables(new Set());
		setObjects([]);
		setEdges([]);
		setRemovedEdgeKeys(new Set());
		setLoadingSchemas(false);
		setSubmitting(false);
		setError(null);
		setValidation(null);
		setValidationWarning(null);
	}, [open]);

	const inferredEdges = useMemo(() => inferEdges(objects), [objects]);

	useEffect(() => {
		setEdges((current) => {
			const byKey = new Map(current.map((edge) => [edgeKey(edge), edge]));
			return inferredEdges
				.filter((candidate) => !removedEdgeKeys.has(edgeKey(candidate)))
				.map((candidate) => {
					const existing = byKey.get(edgeKey(candidate));
					return existing
						? { ...candidate, id: existing.id, label: existing.label }
						: candidate;
				});
		});
	}, [inferredEdges, removedEdgeKeys]);

	const toggleTable = useCallback((tableName: string) => {
		setSelectedTables((current) => {
			const next = new Set(current);
			if (next.has(tableName)) next.delete(tableName);
			else next.add(tableName);
			return next;
		});
	}, []);

	const inferObjects = useCallback(async () => {
		setLoadingSchemas(true);
		setError(null);
		try {
			const selected = projectTables.filter((table) =>
				selectedTables.has(table.name),
			);
			const existingByTable = new Map(
				objects.map((object) => [object.table, object]),
			);
			const inferred = await Promise.all(
				selected.map(async (table, index): Promise<InferredObject> => {
					const existing = existingByTable.get(table.name);
					if (existing) return existing;
					const columns = schemaColumns(await loadSchema(table));
					const typeApiName = apiName(table.name);
					const idColumn = inferIdColumn(columns, typeApiName);
					return {
						id: createId(),
						api_name: typeApiName,
						label: displayName(table.name),
						table: table.name,
						id_column: idColumn,
						display_column: inferDisplayColumn(columns, idColumn),
						property_columns: columns,
						columns,
						style: {
							color: OBJECT_COLORS[index % OBJECT_COLORS.length] ?? "#2563eb",
							icon: "database",
							size: { mode: "fixed", value: 10 },
						},
					};
				}),
			);
			setObjects(inferred);
			if (!name.trim()) {
				setName(
					inferred.length === 1
						? `${inferred[0]?.label ?? "Data"} Ontology`
						: "Operations Ontology",
				);
			}
			setStep("objects");
		} catch (schemaError) {
			setError(
				schemaError instanceof Error
					? schemaError.message
					: "Could not inspect the selected tables.",
			);
		} finally {
			setLoadingSchemas(false);
		}
	}, [loadSchema, name, objects, projectTables, selectedTables]);

	const updateObject = useCallback(
		(index: number, patch: Partial<InferredObject>) => {
			setObjects((current) =>
				current.map((object, objectIndex) =>
					objectIndex === index ? { ...object, ...patch } : object,
				),
			);
		},
		[],
	);

	const updateEdgeLabel = useCallback(
		(target: EdgeLabelMapping, label: string) => {
			setEdges((current) =>
				current.map((edge) => (edge === target ? { ...edge, label } : edge)),
			);
		},
		[],
	);

	const removeEdge = useCallback((target: EdgeLabelMapping) => {
		setRemovedEdgeKeys((keys) => new Set(keys).add(edgeKey(target)));
		setEdges((current) => current.filter((edge) => edge !== target));
	}, []);

	const duplicateLabels = useMemo(() => {
		const counts = new Map<string, number>();
		for (const object of objects) {
			const key = object.label.trim().toLowerCase();
			if (key) counts.set(key, (counts.get(key) ?? 0) + 1);
		}
		return new Set(
			[...counts].filter(([, count]) => count > 1).map(([key]) => key),
		);
	}, [objects]);

	const duplicateApiNames = useMemo(() => {
		const counts = new Map<string, number>();
		for (const object of objects) {
			const key = apiName(object.api_name ?? "");
			if (key) counts.set(key, (counts.get(key) ?? 0) + 1);
		}
		return new Set(
			[...counts].filter(([, count]) => count > 1).map(([key]) => key),
		);
	}, [objects]);

	const hasDuplicateLabel = useCallback(
		(object: InferredObject) =>
			duplicateLabels.has(object.label.trim().toLowerCase()),
		[duplicateLabels],
	);

	const hasDuplicateApiName = useCallback(
		(object: InferredObject) =>
			duplicateApiNames.has(apiName(object.api_name ?? "")),
		[duplicateApiNames],
	);

	const validObjects =
		objects.length > 0 &&
		objects.every(
			(object) =>
				object.label.trim() &&
				apiName(object.api_name ?? "") &&
				object.id_column,
		) &&
		duplicateLabels.size === 0 &&
		duplicateApiNames.size === 0;

	const handleBack = useCallback(() => {
		if (step === "objects") setStep("sources");
		else if (step === "relationships") setStep("objects");
		else if (step === "publish") {
			setValidation(null);
			setValidationWarning(null);
			setStep("relationships");
		} else onOpenChange(false);
	}, [step, onOpenChange]);

	const handleCreate = useCallback(async () => {
		setSubmitting(true);
		setError(null);
		setValidation(null);
		setValidationWarning(null);
		try {
			const finalObjects = objects.map((object) => ({
				...object,
				api_name: apiName(object.api_name ?? ""),
			}));
			const nodes = finalObjects.map(
				({ columns: _columns, api_name_touched: _touched, ...object }) =>
					object,
			);
			const objectViews = finalObjects.map((object) => ({
				object_type: object.id ?? object.api_name ?? object.label,
				title_property: object.display_column,
				prominent_properties: object.property_columns
					.filter((property) => property.name !== object.id_column)
					.slice(0, 4)
					.map((property) => property.name),
			}));

			if (appId) {
				const nowIso = new Date().toISOString();
				const draftOverlay: GraphOverlay = {
					id: "draft",
					name: name.trim(),
					description: description.trim() || undefined,
					nodes,
					edges,
					object_views: objectViews,
					actions: [],
					exposed: false,
					bindings_enabled: true,
					default_limit: 200,
					created_at: nowIso,
					updated_at: nowIso,
				};
				try {
					const result = await graphState.validateOverlay(
						appId,
						"draft",
						userScoped,
						draftOverlay,
					);
					if (!result.ok) {
						setValidation(result);
						setSubmitting(false);
						return;
					}
				} catch {
					setValidationWarning(
						"Could not validate the ontology before creating. Proceeding without validation.",
					);
				}
			}

			await onCreate({
				name: name.trim(),
				description: description.trim() || undefined,
				nodes,
				edges,
				object_views: objectViews,
				actions: [],
				exposed: false,
				bindings_enabled: true,
				default_limit: 200,
			});
			onOpenChange(false);
		} catch (createError) {
			setError(
				createError instanceof Error
					? createError.message
					: "Could not create the ontology.",
			);
		} finally {
			setSubmitting(false);
		}
	}, [
		appId,
		description,
		edges,
		graphState,
		name,
		objects,
		onCreate,
		onOpenChange,
		userScoped,
	]);

	const failingMappings = validation?.mappings?.filter(
		(mapping) => !mapping.ok,
	);
	const activeIndex = STEPS.indexOf(step);

	return (
		<Dialog open={open} onOpenChange={onOpenChange}>
			<DialogContent className="flex max-h-[88vh] max-w-3xl flex-col overflow-hidden">
				<DialogHeader>
					<div className="flex items-center gap-2">
						<div className="rounded-lg bg-primary/10 p-2 text-primary">
							<Sparkles className="h-4 w-4" />
						</div>
						<div>
							<DialogTitle>Set up an ontology</DialogTitle>
							<DialogDescription>
								Turn existing project tables into objects and relationships.
							</DialogDescription>
						</div>
					</div>
				</DialogHeader>

				<div className="grid grid-cols-4 gap-2">
					{STEPS.map((item, index) => {
						const complete = index < activeIndex;
						return (
							<div
								key={item}
								className={`flex items-center gap-2 rounded-lg border px-3 py-2 text-xs ${
									index === activeIndex ? "border-primary bg-primary/5" : ""
								}`}
							>
								<span className="flex h-5 w-5 items-center justify-center rounded-full bg-muted font-medium">
									{complete ? <Check className="h-3 w-3" /> : index + 1}
								</span>
								<span className="capitalize">{item}</span>
							</div>
						);
					})}
				</div>

				{error && (
					<div className="rounded-lg border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive">
						{error}
					</div>
				)}

				<div className="min-h-0 flex-1 overflow-hidden">
					{step === "sources" && (
						<div className="flex h-full min-h-80 flex-col gap-4">
							<div className="space-y-1.5">
								<Label htmlFor="ontology-name">Ontology name</Label>
								<Input
									id="ontology-name"
									value={name}
									onChange={(event) => setName(event.target.value)}
									placeholder="Logistics operations"
								/>
							</div>
							<div className="flex items-center justify-between">
								<div>
									<p className="text-sm font-medium">Choose source tables</p>
									<p className="text-xs text-muted-foreground">
										Schemas load only after you continue.
									</p>
								</div>
								<Badge variant="secondary">
									{selectedTables.size} selected
								</Badge>
							</div>
							<ScrollArea className="min-h-0 flex-1 rounded-lg border">
								<div className="grid gap-2 p-3 sm:grid-cols-2">
									{projectTables.map((table) => (
										<div
											key={table.name}
											className="flex items-center gap-3 rounded-lg border p-3 transition-colors hover:bg-muted/50"
										>
											<Checkbox
												id={`ontology-source-${table.name}`}
												checked={selectedTables.has(table.name)}
												onCheckedChange={() => toggleTable(table.name)}
											/>
											<Database className="h-4 w-4 text-muted-foreground" />
											<Label
												htmlFor={`ontology-source-${table.name}`}
												className="min-w-0 flex-1 cursor-pointer truncate text-sm font-normal"
											>
												{table.name}
											</Label>
										</div>
									))}
									{projectTables.length === 0 && (
										<p className="col-span-full py-10 text-center text-sm text-muted-foreground">
											Create a project table before setting up an ontology.
										</p>
									)}
								</div>
							</ScrollArea>
						</div>
					)}

					{step === "objects" && (
						<ScrollArea className="h-full min-h-80 pr-3">
							<div className="space-y-3">
								<div>
									<p className="text-sm font-medium">Review inferred objects</p>
									<p className="text-xs text-muted-foreground">
										IDs and display fields are suggested from each selected
										schema.
									</p>
								</div>
								{objects.map((object, index) => {
									const dupLabel = hasDuplicateLabel(object);
									const dupApiName = hasDuplicateApiName(object);
									return (
										<div
											key={object.id}
											className="space-y-3 rounded-xl border p-4"
										>
											<div className="flex items-center gap-3">
												<span
													className="h-3 w-3 rounded-full"
													style={{ backgroundColor: object.style.color }}
												/>
												<div className="min-w-0">
													<p className="truncate text-sm font-medium">
														{object.table}
													</p>
													<p className="text-xs text-muted-foreground">
														{object.columns.length} properties
													</p>
												</div>
											</div>
											<div className="grid gap-3 sm:grid-cols-2">
												<div className="space-y-1.5">
													<Label>Object name</Label>
													<Input
														value={object.label}
														onChange={(event) => {
															const label = event.target.value;
															updateObject(
																index,
																object.api_name_touched
																	? { label }
																	: { label, api_name: apiName(label) },
															);
														}}
														aria-invalid={dupLabel}
														className={
															dupLabel ? "border-destructive" : undefined
														}
													/>
													{dupLabel && (
														<p className="text-xs text-destructive">
															Another object already uses this name.
														</p>
													)}
												</div>
												<div className="space-y-1.5">
													<Label>API name</Label>
													<Input
														value={object.api_name ?? ""}
														onChange={(event) =>
															updateObject(index, {
																api_name: event.target.value,
																api_name_touched: true,
															})
														}
														onBlur={() =>
															updateObject(index, {
																api_name: apiName(object.api_name ?? ""),
															})
														}
														aria-invalid={dupApiName}
														className={`font-mono text-xs${
															dupApiName ? " border-destructive" : ""
														}`}
													/>
													{dupApiName && (
														<p className="text-xs text-destructive">
															Another object already uses this API name.
														</p>
													)}
												</div>
												<div className="space-y-1.5">
													<Label>Unique ID</Label>
													<Select
														value={object.id_column}
														onValueChange={(value) =>
															updateObject(index, { id_column: value })
														}
													>
														<SelectTrigger>
															<SelectValue />
														</SelectTrigger>
														<SelectContent>
															{object.columns.map((column) => (
																<SelectItem
																	key={column.name}
																	value={column.name}
																>
																	{column.name}
																</SelectItem>
															))}
														</SelectContent>
													</Select>
												</div>
												<div className="space-y-1.5">
													<Label>Display property</Label>
													<Select
														value={object.display_column ?? "__none"}
														onValueChange={(value) =>
															updateObject(index, {
																display_column:
																	value === "__none" ? undefined : value,
															})
														}
													>
														<SelectTrigger>
															<SelectValue />
														</SelectTrigger>
														<SelectContent>
															<SelectItem value="__none">
																Use unique ID
															</SelectItem>
															{object.columns.map((column) => (
																<SelectItem
																	key={column.name}
																	value={column.name}
																>
																	{column.name}
																</SelectItem>
															))}
														</SelectContent>
													</Select>
												</div>
											</div>
										</div>
									);
								})}
							</div>
						</ScrollArea>
					)}

					{step === "relationships" && (
						<ScrollArea className="h-full min-h-80 pr-3">
							<div className="space-y-3">
								<div>
									<p className="text-sm font-medium">Review relationships</p>
									<p className="text-xs text-muted-foreground">
										Inferred from foreign-key style columns. Edit labels or
										remove any that do not belong.
									</p>
								</div>
								{edges.length === 0 ? (
									<div className="rounded-xl border border-dashed p-8 text-center">
										<p className="text-sm font-medium">
											No relationships inferred
										</p>
										<p className="mt-1 text-xs text-muted-foreground">
											We could not find foreign-key columns linking your
											objects. You can add relationships later from the ontology
											editor.
										</p>
									</div>
								) : (
									edges.map((edge) => (
										<EdgeReviewCard
											key={edge.id ?? edgeKey(edge)}
											edge={edge}
											onLabelChange={updateEdgeLabel}
											onRemove={removeEdge}
										/>
									))
								)}
							</div>
						</ScrollArea>
					)}

					{step === "publish" && (
						<div className="space-y-5 py-2">
							<div className="space-y-1.5">
								<Label htmlFor="ontology-description">Description</Label>
								<Input
									id="ontology-description"
									value={description}
									onChange={(event) => setDescription(event.target.value)}
									placeholder="What this model represents and who should use it"
								/>
							</div>
							<div className="rounded-xl border bg-muted/30 p-4">
								<div className="flex items-center justify-between">
									<div>
										<p className="font-medium">{name}</p>
										<p className="text-sm text-muted-foreground">
											{objects.length} objects · {edges.length} relationships
										</p>
									</div>
									<Badge className="gap-1">
										<Sparkles className="h-3 w-3" />
										Bindings on
									</Badge>
								</div>
								<div className="mt-4 flex flex-wrap gap-2">
									{objects.map((object) => (
										<Badge key={object.id} variant="secondary">
											{object.label}
										</Badge>
									))}
								</div>
							</div>
							{validation && !validation.ok && (
								<div className="space-y-2 rounded-lg border border-destructive/30 bg-destructive/10 p-3 text-sm text-destructive">
									<p className="font-medium">
										Fix these issues before creating
									</p>
									{validation.issues.length > 0 && (
										<ul className="list-disc space-y-1 pl-4 text-xs">
											{validation.issues.map((issue) => (
												<li key={issue}>{issue}</li>
											))}
										</ul>
									)}
									{failingMappings?.map((mapping) => (
										<div
											key={`${mapping.kind}-${mapping.label}`}
											className="text-xs"
										>
											<p className="font-medium">
												{mapping.kind === "edge" ? "Relationship" : "Object"}:{" "}
												{mapping.label}
											</p>
											<ul className="list-disc space-y-1 pl-4">
												{mapping.issues.map((issue) => (
													<li key={issue}>{issue}</li>
												))}
											</ul>
										</div>
									))}
								</div>
							)}
							{validationWarning && (
								<div className="rounded-lg border border-amber-500/30 bg-amber-500/10 p-3 text-xs text-amber-600 dark:text-amber-400">
									{validationWarning}
								</div>
							)}
							<p className="text-xs text-muted-foreground">
								The ontology stays private until you expose it from Sharing.
								Standard object views and board bindings are generated
								automatically.
							</p>
						</div>
					)}
				</div>

				<DialogFooter className="flex-row items-center justify-between border-t pt-4 sm:justify-between">
					<Button variant="ghost" onClick={handleBack}>
						{step === "sources" ? "Cancel" : "Back"}
					</Button>
					{step === "sources" && (
						<Button
							onClick={inferObjects}
							disabled={selectedTables.size === 0 || loadingSchemas}
						>
							{loadingSchemas ? (
								<Loader2 className="h-4 w-4 animate-spin" />
							) : (
								<Sparkles className="h-4 w-4" />
							)}
							Infer objects
						</Button>
					)}
					{step === "objects" && (
						<Button
							onClick={() => setStep("relationships")}
							disabled={!validObjects}
						>
							Review <ArrowRight className="h-4 w-4" />
						</Button>
					)}
					{step === "relationships" && (
						<Button onClick={() => setStep("publish")}>
							Continue <ArrowRight className="h-4 w-4" />
						</Button>
					)}
					{step === "publish" && (
						<Button
							onClick={handleCreate}
							disabled={!name.trim() || submitting}
						>
							{submitting ? (
								<Loader2 className="h-4 w-4 animate-spin" />
							) : (
								<Check className="h-4 w-4" />
							)}
							Create ontology
						</Button>
					)}
				</DialogFooter>
			</DialogContent>
		</Dialog>
	);
}

interface EdgeReviewCardProps {
	edge: EdgeLabelMapping;
	onLabelChange: (edge: EdgeLabelMapping, label: string) => void;
	onRemove: (edge: EdgeLabelMapping) => void;
}

function EdgeReviewCard({
	edge,
	onLabelChange,
	onRemove,
}: Readonly<EdgeReviewCardProps>) {
	return (
		<div className="space-y-3 rounded-xl border p-4">
			<div className="flex items-center justify-between gap-3">
				<div className="flex min-w-0 flex-wrap items-center gap-2 text-sm">
					<Badge variant="secondary">{edge.src_label}</Badge>
					<ArrowRight className="h-3.5 w-3.5 text-muted-foreground" />
					<Badge variant="secondary">{edge.dst_label}</Badge>
				</div>
				<Button
					variant="ghost"
					size="icon"
					onClick={() => onRemove(edge)}
					title="Remove relationship"
				>
					<Trash2 className="h-4 w-4" />
				</Button>
			</div>
			<div className="grid gap-3 sm:grid-cols-2">
				<div className="space-y-1.5">
					<Label>Relationship label</Label>
					<Input
						value={edge.label}
						onChange={(event) => onLabelChange(edge, event.target.value)}
						className="font-mono text-xs"
					/>
				</div>
				<div className="space-y-1.5">
					<Label>Join</Label>
					<p className="rounded-md border bg-muted/40 px-3 py-2 font-mono text-xs text-muted-foreground">
						{edge.src_column} → {edge.dst_column}
					</p>
				</div>
			</div>
		</div>
	);
}
