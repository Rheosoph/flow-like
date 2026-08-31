"use client";

import { useTranslation } from "@flow-like/locales";
import { createId } from "@paralleldrive/cuid2";
import {
	ArrowLeftRight,
	ArrowRight,
	Check,
	Database,
	Loader2,
	Plus,
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
import { Switch } from "../../ui/switch";
import {
	AddRelationshipForm,
	type RelationshipEndpoint,
	type WizardEdge,
	apiName,
	buildEdge,
	displayName,
	endpointMatchesStem,
	foreignKeyStem,
	isValidGraphIdentifier,
	reversedEdge,
	toEdgeMapping,
	uniqueLabel,
} from "./relationship-form";

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

function objectToEndpoint(object: InferredObject): RelationshipEndpoint {
	return {
		id: object.id ?? "",
		label: object.label,
		api_name: object.api_name,
		table: object.table,
		id_column: object.id_column,
		columns: object.columns,
		color: object.style.color,
	};
}

function inferIdColumn(
	columns: PropertyColumn[],
	typeApiName: string,
	pointsAtAnotherObject: (stem: string) => boolean,
): string {
	const names = columns.map((column) => column.name);
	const exact = names.find((name) => apiName(name) === "id");
	if (exact) return exact;
	const own = names.find((name) => apiName(name) === `${typeApiName}_id`);
	if (own) return own;
	// A bare `*_id` that resolves to a *different* object is a foreign key, not
	// this object's identity — taking it here would also drop the relationship.
	const ownIdShaped = names.find((name) => {
		const stem = foreignKeyStem(name);
		return stem !== undefined && !pointsAtAnotherObject(stem);
	});
	return ownIdShaped ?? names[0] ?? "";
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

/**
 * Foreign-key inference. Deliberately conservative: a column only becomes an
 * edge when its stem resolves to another selected object, so widening the
 * accepted column shapes cannot invent relationships. Self-references and
 * role-named keys (`owner_id`) are left to the manual form.
 */
function inferEdges(objects: InferredObject[]): WizardEdge[] {
	const endpoints = objects.map(objectToEndpoint);
	const edges: WizardEdge[] = [];
	const taken = new Set(
		endpoints.map((endpoint) => endpoint.label.trim().toLowerCase()),
	);
	for (const source of endpoints) {
		for (const column of source.columns) {
			if (column.name === source.id_column) continue;
			const stem = foreignKeyStem(column.name);
			if (!stem) continue;
			const target = endpoints.find((endpoint) =>
				endpointMatchesStem(endpoint, stem),
			);
			if (!target || target.id === source.id) continue;
			const sourceApi = apiName(source.api_name ?? source.label);
			const targetApi = apiName(target.api_name ?? target.label);
			const role = stem === targetApi ? targetApi : stem;
			edges.push(
				buildEdge({
					originKey: `inferred:${source.id}::${target.id}::${column.name}`,
					manual: false,
					source,
					target,
					table: source.table,
					srcColumn: source.id_column,
					dstColumn: column.name,
					label: uniqueLabel(`${sourceApi}_has_${role}`, taken),
				}),
			);
		}
	}
	return edges;
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
	const { t } = useTranslation("settings");
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
	const [edges, setEdges] = useState<WizardEdge[]>([]);
	const [removedEdgeKeys, setRemovedEdgeKeys] = useState<Set<string>>(
		new Set(),
	);
	const [addingEdge, setAddingEdge] = useState(false);
	const [edgePrefill, setEdgePrefill] = useState<{
		sourceId: string;
		dstColumn: string;
	} | null>(null);
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
		setAddingEdge(false);
		setEdgePrefill(null);
		setLoadingSchemas(false);
		setSubmitting(false);
		setError(null);
		setValidation(null);
		setValidationWarning(null);
	}, [open]);

	const endpoints = useMemo(() => objects.map(objectToEndpoint), [objects]);
	const inferredEdges = useMemo(() => inferEdges(objects), [objects]);

	// Inferred edges are rebuilt from the current objects; manual ones are kept
	// verbatim and only have their endpoint labels re-resolved, so renaming an
	// object cannot orphan a hand-authored relationship.
	useEffect(() => {
		setEdges((current) => {
			const objectsById = new Map(
				objects.map((object) => [object.id ?? "", object]),
			);
			const previous = new Map(current.map((edge) => [edge.origin_key, edge]));
			const reconciled = inferredEdges
				.filter((candidate) => !removedEdgeKeys.has(candidate.origin_key))
				.map((candidate) => {
					const existing = previous.get(candidate.origin_key);
					return existing
						? {
								...candidate,
								id: existing.id,
								label: existing.label,
								containment: existing.containment,
								dst_ontology: existing.dst_ontology,
								dst_binding_id: existing.dst_binding_id,
							}
						: candidate;
				});
			const manual = current.flatMap((edge) => {
				if (!edge.manual) return [];
				const source = objectsById.get(edge.src_object_id ?? "");
				const target = objectsById.get(edge.dst_object_id ?? "");
				if (!source || !target) return [];
				return [{ ...edge, src_label: source.label, dst_label: target.label }];
			});
			return [...reconciled, ...manual];
		});
	}, [inferredEdges, objects, removedEdgeKeys]);

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
			// Schemas load first so identity inference can tell a foreign key apart
			// from an object's own id — that needs every selected table's API name.
			const loaded = await Promise.all(
				selected.map(async (table) => {
					const existing = existingByTable.get(table.name);
					return {
						table,
						existing,
						columns: existing ? [] : schemaColumns(await loadSchema(table)),
					};
				}),
			);
			const selectedApiNames = new Set(
				loaded.map(({ table, existing }) =>
					apiName(existing?.api_name ?? table.name),
				),
			);
			const inferred = loaded.map(
				({ table, existing, columns }, index): InferredObject => {
					if (existing) return existing;
					const typeApiName = apiName(table.name);
					const idColumn = inferIdColumn(
						columns,
						typeApiName,
						(stem) => stem !== typeApiName && selectedApiNames.has(stem),
					);
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
				},
			);
			setObjects(inferred);
			if (!name.trim()) {
				setName(
					inferred.length === 1
						? t("valOntology", "{{val}} Ontology", {
								val: inferred[0]?.label ?? "Data",
							})
						: "Operations Ontology",
				);
			}
			setStep("objects");
		} catch (schemaError) {
			setError(
				schemaError instanceof Error
					? schemaError.message
					: t(
							"couldNotInspectTheSelectedTables",
							"Could not inspect the selected tables.",
						),
			);
		} finally {
			setLoadingSchemas(false);
		}
	}, [loadSchema, name, objects, projectTables, selectedTables, t]);

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

	const updateEdge = useCallback(
		(originKey: string, patch: Partial<EdgeLabelMapping>) => {
			setEdges((current) =>
				current.map((edge) =>
					edge.origin_key === originKey ? { ...edge, ...patch } : edge,
				),
			);
		},
		[],
	);

	const removeEdge = useCallback((originKey: string) => {
		setEdges((current) =>
			current.filter((edge) => edge.origin_key !== originKey),
		);
		if (originKey.startsWith("inferred:")) {
			setRemovedEdgeKeys((keys) => new Set(keys).add(originKey));
		}
	}, []);

	// A reversed edge becomes manual so re-inference cannot flip it back, and
	// its inferred key is tombstoned so the original does not reappear beside it.
	const reverseEdge = useCallback((originKey: string) => {
		setEdges((current) =>
			current.map((edge) =>
				edge.origin_key === originKey
					? {
							...reversedEdge(edge),
							manual: true,
							src_object_id: edge.dst_object_id,
							dst_object_id: edge.src_object_id,
						}
					: edge,
			),
		);
		if (originKey.startsWith("inferred:")) {
			setRemovedEdgeKeys((keys) => new Set(keys).add(originKey));
		}
	}, []);

	const addEdge = useCallback((edge: WizardEdge) => {
		setEdges((current) => [...current, edge]);
		setAddingEdge(false);
		setEdgePrefill(null);
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

	// The server puts object and relationship labels in one case-insensitive
	// namespace, so collisions have to be counted across both.
	const labelCounts = useMemo(() => {
		const counts = new Map<string, number>();
		for (const label of [
			...objects.map((object) => object.label),
			...edges.map((edge) => edge.label),
		]) {
			const key = label.trim().toLowerCase();
			if (key) counts.set(key, (counts.get(key) ?? 0) + 1);
		}
		return counts;
	}, [edges, objects]);

	const takenLabels = useMemo(() => new Set(labelCounts.keys()), [labelCounts]);

	const edgeLabelIssue = useCallback(
		(edge: WizardEdge): "invalid" | "duplicate" | undefined => {
			const label = edge.label.trim();
			if (!isValidGraphIdentifier(label)) return "invalid";
			if ((labelCounts.get(label.toLowerCase()) ?? 0) > 1) return "duplicate";
			return undefined;
		},
		[labelCounts],
	);

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

	// Foreign-key-shaped columns inference could not resolve — offered as manual
	// starting points instead of guessed at.
	const unlinkedColumns = useMemo(() => {
		const linked = new Set(
			edges.map((edge) => `${edge.table}::${edge.dst_column}`),
		);
		return objects.flatMap((object) =>
			object.columns
				.filter(
					(column) =>
						column.name !== object.id_column &&
						foreignKeyStem(column.name) !== undefined &&
						!linked.has(`${object.table}::${column.name}`),
				)
				.map((column) => ({ object, column: column.name })),
		);
	}, [edges, objects]);

	const validObjects =
		objects.length > 0 &&
		objects.every(
			(object) =>
				isValidGraphIdentifier(object.label.trim()) &&
				apiName(object.api_name ?? "") &&
				object.id_column,
		) &&
		duplicateLabels.size === 0 &&
		duplicateApiNames.size === 0;

	const validEdges = useMemo(
		() => edges.every((edge) => !edgeLabelIssue(edge)),
		[edgeLabelIssue, edges],
	);

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
			const finalEdges = edges.map(toEdgeMapping);
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
					edges: finalEdges,
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
						t(
							"couldNotValidateTheOntologyBeforeCreatingProceedingWithoutValidation",
							"Could not validate the ontology before creating. Proceeding without validation.",
						),
					);
				}
			}

			await onCreate({
				name: name.trim(),
				description: description.trim() || undefined,
				nodes,
				edges: finalEdges,
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
					: t("couldNotCreateTheOntology", "Could not create the ontology."),
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
		t,
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
							<DialogTitle>
								{t("setUpAnOntology", "Set up an ontology")}
							</DialogTitle>
							<DialogDescription>
								{t(
									"turnExistingProjectTablesIntoObjectsAndRelationships",
									"Turn existing project tables into objects and relationships.",
								)}
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
								<Label htmlFor="ontology-name">
									{t("ontologyName", "Ontology name")}
								</Label>
								<Input
									id="ontology-name"
									value={name}
									onChange={(event) => setName(event.target.value)}
									placeholder={t("logisticsOperations", "Logistics operations")}
								/>
							</div>
							<div className="flex items-center justify-between">
								<div>
									<p className="text-sm font-medium">
										{t("chooseSourceTables", "Choose source tables")}
									</p>
									<p className="text-xs text-muted-foreground">
										{t(
											"schemasLoadOnlyAfterYouContinue",
											"Schemas load only after you continue.",
										)}
									</p>
								</div>
								<Badge variant="secondary">
									{t("sizeSelected", "{{size}} selected", {
										size: selectedTables.size,
									})}
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
											{t(
												"createAProjectTableBeforeSettingUpAnOntology",
												"Create a project table before setting up an ontology.",
											)}
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
									<p className="text-sm font-medium">
										{t("reviewInferredObjects", "Review inferred objects")}
									</p>
									<p className="text-xs text-muted-foreground">
										{t(
											"idsAndDisplayFieldsAreSuggestedFromEachSelectedSchema",
											"IDs and display fields are suggested from each selected schema.",
										)}
									</p>
								</div>
								{objects.map((object, index) => {
									const dupLabel = hasDuplicateLabel(object);
									const dupApiName = hasDuplicateApiName(object);
									const invalidLabel = !isValidGraphIdentifier(
										object.label.trim(),
									);
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
														{t("lengthProperties", "{{length}} properties", {
															length: object.columns.length,
														})}
													</p>
												</div>
											</div>
											<div className="grid gap-3 sm:grid-cols-2">
												<div className="space-y-1.5">
													<Label>{t("objectName", "Object name")}</Label>
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
														aria-invalid={dupLabel || invalidLabel}
														className={
															dupLabel || invalidLabel
																? "border-destructive"
																: undefined
														}
													/>
													{dupLabel && (
														<p className="text-xs text-destructive">
															{t(
																"anotherObjectAlreadyUsesThisName",
																"Another object already uses this name.",
															)}
														</p>
													)}
													{!dupLabel && invalidLabel && (
														<p className="text-xs text-destructive">
															{t(
																"useLettersDigitsAndUnderscoresStartingWithALetter",
																"Use letters, digits and underscores, starting with a letter.",
															)}
														</p>
													)}
												</div>
												<div className="space-y-1.5">
													<Label>{t("apiName", "API name")}</Label>
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
															{t(
																"anotherObjectAlreadyUsesThisApiName",
																"Another object already uses this API name.",
															)}
														</p>
													)}
												</div>
												<div className="space-y-1.5">
													<Label>{t("uniqueId", "Unique ID")}</Label>
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
													<Label>
														{t("displayProperty", "Display property")}
													</Label>
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
																{t("useUniqueId", "Use unique ID")}
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
								<div className="flex items-start justify-between gap-3">
									<div>
										<p className="text-sm font-medium">
											{t("reviewRelationships", "Review relationships")}
										</p>
										<p className="text-xs text-muted-foreground">
											{t(
												"inferredFromForeignkeyStyleColumnsAddYourOwnForAnythingInferenceCouldNotSee",
												"Inferred from foreign-key style columns. Add your own for anything inference could not see.",
											)}
										</p>
									</div>
									{!addingEdge && (
										<Button
											size="sm"
											variant="outline"
											onClick={() => {
												setEdgePrefill(null);
												setAddingEdge(true);
											}}
											disabled={objects.length === 0}
										>
											<Plus className="h-4 w-4" />
											{t("addRelationship", "Add relationship")}
										</Button>
									)}
								</div>

								{addingEdge && (
									<AddRelationshipForm
										endpoints={endpoints}
										takenLabels={takenLabels}
										prefill={edgePrefill}
										onAdd={addEdge}
										onCancel={() => {
											setAddingEdge(false);
											setEdgePrefill(null);
										}}
									/>
								)}

								{edges.length === 0 && !addingEdge && (
									<div className="rounded-xl border border-dashed p-8 text-center">
										<p className="text-sm font-medium">
											{t(
												"noRelationshipsInferred",
												"No relationships inferred",
											)}
										</p>
										<p className="mt-1 text-xs text-muted-foreground">
											{t(
												"weCouldNotFindForeignkeyColumnsLinkingYourObjectsAddOneByHandToConnectThem",
												"We could not find foreign-key columns linking your objects. Add one by hand to connect them.",
											)}
										</p>
									</div>
								)}

								{edges.map((edge) => (
									<EdgeReviewCard
										key={edge.origin_key}
										edge={edge}
										issue={edgeLabelIssue(edge)}
										onEdgeChange={updateEdge}
										onReverse={reverseEdge}
										onRemove={removeEdge}
									/>
								))}

								{unlinkedColumns.length > 0 && (
									<div className="space-y-2 rounded-xl border border-dashed p-4">
										<p className="text-xs font-medium">
											{t("unlinkedIdColumns", "Unlinked ID columns")}
										</p>
										<p className="text-xs text-muted-foreground">
											{t(
												"theseLookLikeForeignKeysButDoNotResolveToASelectedObjectPickOneToLinkItYourself",
												"These look like foreign keys but do not resolve to a selected object. Pick one to link it yourself.",
											)}
										</p>
										<div className="flex flex-wrap gap-2 pt-1">
											{unlinkedColumns.map(({ object, column }) => (
												<Button
													key={`${object.id}-${column}`}
													size="sm"
													variant="outline"
													className="h-7 font-mono text-[11px]"
													onClick={() => {
														setEdgePrefill({
															sourceId: object.id ?? "",
															dstColumn: column,
														});
														setAddingEdge(true);
													}}
												>
													<Plus className="h-3 w-3" />
													{`${object.table}.${column}`}
												</Button>
											))}
										</div>
									</div>
								)}
							</div>
						</ScrollArea>
					)}

					{step === "publish" && (
						<div className="space-y-5 py-2">
							<div className="space-y-1.5">
								<Label htmlFor="ontology-description">
									{t("description", "Description")}
								</Label>
								<Input
									id="ontology-description"
									value={description}
									onChange={(event) => setDescription(event.target.value)}
									placeholder={t(
										"whatThisModelRepresentsAndWhoShouldUseIt",
										"What this model represents and who should use it",
									)}
								/>
							</div>
							<div className="rounded-xl border bg-muted/30 p-4">
								<div className="flex items-center justify-between">
									<div>
										<p className="font-medium">{name}</p>
										<p className="text-sm text-muted-foreground">
											{t(
												"lengthObjectsLength2Relationships",
												"{{length}} objects · {{length2}} relationships",
												{ length: objects.length, length2: edges.length },
											)}
										</p>
									</div>
									<Badge className="gap-1">
										<Sparkles className="h-3 w-3" />
										{t("bindingsOn", "Bindings on")}
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
										{t(
											"fixTheseIssuesBeforeCreating",
											"Fix these issues before creating",
										)}
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
												{mapping.kind === "edge"
													? t("relationship", "Relationship")
													: t("object", "Object")}
												{`: ${mapping.label}`}
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
								{t(
									"theOntologyStaysPrivateUntilYouExposeItFromSharingObjectViewsAndBoardBindingsAreGeneratedAutomatically",
									"The ontology stays private until you expose it from Sharing. Object views and board bindings are generated automatically.",
								)}
							</p>
						</div>
					)}
				</div>

				<DialogFooter className="flex-row items-center justify-between border-t pt-4 sm:justify-between">
					<Button variant="ghost" onClick={handleBack}>
						{step === "sources" ? t("cancel", "Cancel") : t("back", "Back")}
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
							{t("inferObjects", "Infer objects")}
						</Button>
					)}
					{step === "objects" && (
						<Button
							onClick={() => setStep("relationships")}
							disabled={!validObjects}
						>
							{t("review", "Review")} <ArrowRight className="h-4 w-4" />
						</Button>
					)}
					{step === "relationships" && (
						<Button onClick={() => setStep("publish")} disabled={!validEdges}>
							{t("continue", "Continue")} <ArrowRight className="h-4 w-4" />
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
							{t("createOntology", "Create ontology")}
						</Button>
					)}
				</DialogFooter>
			</DialogContent>
		</Dialog>
	);
}

interface EdgeReviewCardProps {
	edge: WizardEdge;
	issue?: "invalid" | "duplicate";
	onEdgeChange: (originKey: string, patch: Partial<EdgeLabelMapping>) => void;
	onReverse: (originKey: string) => void;
	onRemove: (originKey: string) => void;
}

function EdgeReviewCard({
	edge,
	issue,
	onEdgeChange,
	onReverse,
	onRemove,
}: Readonly<EdgeReviewCardProps>) {
	const { t } = useTranslation("settings");
	const containmentId = `edge-containment-${edge.origin_key}`;
	return (
		<div className="space-y-3 rounded-xl border p-4">
			<div className="flex items-center justify-between gap-3">
				<div className="flex min-w-0 flex-wrap items-center gap-2 text-sm">
					<Badge variant="secondary">{edge.src_label}</Badge>
					<ArrowRight className="h-3.5 w-3.5 text-muted-foreground" />
					<Badge variant="secondary">{edge.dst_label}</Badge>
					{edge.manual && (
						<Badge variant="outline" className="text-[10px]">
							{t("manual", "Manual")}
						</Badge>
					)}
				</div>
				<div className="flex items-center">
					<Button
						variant="ghost"
						size="icon"
						onClick={() => onReverse(edge.origin_key)}
						title={t("reverseDirection", "Reverse direction")}
					>
						<ArrowLeftRight className="h-4 w-4" />
					</Button>
					<Button
						variant="ghost"
						size="icon"
						onClick={() => onRemove(edge.origin_key)}
						title={t("removeRelationship", "Remove relationship")}
					>
						<Trash2 className="h-4 w-4" />
					</Button>
				</div>
			</div>
			<div className="grid gap-3 sm:grid-cols-2">
				<div className="space-y-1.5">
					<Label>{t("relationshipLabel", "Relationship label")}</Label>
					<Input
						value={edge.label}
						onChange={(event) =>
							onEdgeChange(edge.origin_key, { label: event.target.value })
						}
						aria-invalid={Boolean(issue)}
						className={`font-mono text-xs${issue ? " border-destructive" : ""}`}
					/>
					{issue === "duplicate" && (
						<p className="text-xs text-destructive">
							{t(
								"thisLabelIsAlreadyUsedByAnotherObjectOrRelationship",
								"This label is already used by another object or relationship.",
							)}
						</p>
					)}
					{issue === "invalid" && (
						<p className="text-xs text-destructive">
							{t(
								"useLettersDigitsAndUnderscoresStartingWithALetter",
								"Use letters, digits and underscores, starting with a letter.",
							)}
						</p>
					)}
				</div>
				<div className="space-y-1.5">
					<Label>{t("join", "Join")}</Label>
					<p className="rounded-md border bg-muted/40 px-3 py-2 font-mono text-xs text-muted-foreground">{`${edge.table}.${edge.src_column} → ${edge.table}.${edge.dst_column}`}</p>
				</div>
			</div>
			<div className="flex items-center gap-2">
				<Switch
					id={containmentId}
					checked={Boolean(edge.containment)}
					onCheckedChange={(checked) =>
						onEdgeChange(edge.origin_key, { containment: checked })
					}
				/>
				<Label htmlFor={containmentId} className="text-xs font-medium">
					{t("hierarchyParentChild", "Hierarchy (parent → child)")}
				</Label>
			</div>
		</div>
	);
}
