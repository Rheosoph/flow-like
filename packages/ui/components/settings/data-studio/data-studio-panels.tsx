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
	Plus,
	RefreshCw,
	Search,
	Share2,
	ShieldCheck,
	Workflow,
	X,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import type { IBoard } from "../../../lib/schema/flow/board";
import type {
	GraphOverlay,
	NodeLabelMapping,
	OntologyActionDefinition,
} from "../../../state/backend-state/graph-state";
import type { IAppConnection } from "../../../state/backend-state/types";
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
}: Readonly<
	StudioPanelBaseProps & {
		onSample: (
			ontologyId: string,
			objectType: string,
			limit: number,
		) => Promise<unknown[]>;
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

	const loadObjects = useCallback(async () => {
		if (!ontology || !objectType) return;
		setLoading(true);
		setError(null);
		try {
			const result = await onSample(ontology.id, objectType.label, 100);
			setRows(
				result.filter(
					(row): row is Record<string, unknown> =>
						typeof row === "object" && row !== null && !Array.isArray(row),
				),
			);
		} catch (loadError) {
			setError(
				loadError instanceof Error
					? loadError.message
					: "Could not load objects.",
			);
			setRows([]);
		} finally {
			setLoading(false);
		}
	}, [objectType, onSample, ontology]);

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
						<SelectTrigger className="bg-background">
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
								className="pl-8"
							/>
							{query && (
								<Button
									variant="ghost"
									size="icon"
									className="absolute right-0 top-0 h-9 w-9"
									onClick={() => setQuery("")}
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
						<div className="m-4 rounded-lg border border-destructive/30 bg-destructive/10 p-4 text-sm text-destructive">
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
									<th className="w-10" />
								</tr>
							</thead>
							<tbody>
								{visibleRows.map((row, index) => (
									<tr
										key={String(row[objectType?.id_column ?? ""] ?? index)}
										className="cursor-pointer border-b transition-colors hover:bg-muted/50"
										onClick={() => setSelectedRow(row)}
										onKeyDown={(event) => {
											if (event.key === "Enter" || event.key === " ") {
												event.preventDefault();
												setSelectedRow(row);
											}
										}}
										tabIndex={0}
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
											<ChevronRight className="h-4 w-4 text-muted-foreground" />
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
			/>
		</div>
	);
}

function ObjectViewSheet({
	ontology,
	objectType,
	row,
	onClose,
}: Readonly<{
	ontology?: GraphOverlay;
	objectType?: NodeLabelMapping;
	row: Record<string, unknown> | null;
	onClose: () => void;
}>) {
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
										<Badge
											key={action.id}
											variant="outline"
											className="gap-1.5 py-1.5"
										>
											<Workflow className="h-3 w-3" />
											{action.name}
										</Badge>
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
			</SheetContent>
		</Sheet>
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
	onSaveActions,
}: Readonly<
	StudioPanelBaseProps & {
		boards: IBoard[];
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
	const [saving, setSaving] = useState(false);
	const ontology =
		ontologies.find((item) => item.id === ontologyId) ?? ontologies[0];
	const board = boards.find((item) => item.id === boardId);
	const startNodes = board
		? Object.values(board.nodes).filter((node) => node.start)
		: [];
	const allActions = ontologies.flatMap((item) =>
		(item.actions ?? []).map((action) => ({ action, ontology: item })),
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
		try {
			const version = board?.version;
			const boardVersion =
				Array.isArray(version) && version.length === 3
					? ([Number(version[0]), Number(version[1]), Number(version[2])] as [
							number,
							number,
							number,
						])
					: undefined;
			await onSaveActions(ontology.id, [
				...(ontology.actions ?? []),
				{
					id: createId(),
					name: name.trim(),
					description: description.trim() || undefined,
					object_type: objectType,
					board_id: boardId,
					board_version: boardVersion,
					start_node_id: startNodeId,
					enabled: true,
					allow_bulk: false,
				},
			]);
			setDialogOpen(false);
			setName("");
			setDescription("");
			setBoardId("");
			setStartNodeId("");
		} finally {
			setSaving(false);
		}
	}, [
		board?.version,
		boardId,
		description,
		name,
		objectType,
		onSaveActions,
		ontology,
		startNodeId,
	]);

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
				<Button onClick={() => setDialogOpen(true)}>
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
				<div className="grid gap-3 lg:grid-cols-2">
					{allActions.map(({ action, ontology: owner }) => (
						<Card key={action.id}>
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
							</CardContent>
						</Card>
					))}
				</div>
			)}
			<Dialog open={dialogOpen} onOpenChange={setDialogOpen}>
				<DialogContent className="max-w-xl">
					<DialogHeader>
						<DialogTitle>Define an ontology action</DialogTitle>
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
						</div>
					</div>
					<DialogFooter>
						<Button variant="ghost" onClick={() => setDialogOpen(false)}>
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
							{saving && <Loader2 className="h-4 w-4 animate-spin" />}Save
							action
						</Button>
					</DialogFooter>
				</DialogContent>
			</Dialog>
		</div>
	);
}

export function OntologySharingPanel({
	ontologies,
	connections,
	remoteConnections,
	onCreateOntology,
	onUpdateOntology,
	onLoadRemoteOntologies,
}: Readonly<
	StudioPanelBaseProps & {
		connections: IAppConnection[];
		remoteConnections: IAppConnection[];
		onUpdateOntology: (
			ontologyId: string,
			patch: Pick<GraphOverlay, "exposed" | "bindings_enabled">,
		) => Promise<void>;
		onLoadRemoteOntologies: (targetAppId: string) => Promise<GraphOverlay[]>;
	}
>) {
	const [savingId, setSavingId] = useState<string | null>(null);
	const [loadingConnectionId, setLoadingConnectionId] = useState<string | null>(
		null,
	);
	const [remoteOntologies, setRemoteOntologies] = useState<
		Record<string, GraphOverlay[]>
	>({});
	const [remoteErrors, setRemoteErrors] = useState<Record<string, string>>({});
	const update = useCallback(
		async (
			ontology: GraphOverlay,
			patch: Partial<Pick<GraphOverlay, "exposed" | "bindings_enabled">>,
		) => {
			setSavingId(ontology.id);
			try {
				await onUpdateOntology(ontology.id, {
					exposed: patch.exposed ?? ontology.exposed,
					bindings_enabled: patch.bindings_enabled ?? ontology.bindings_enabled,
				});
			} finally {
				setSavingId(null);
			}
		},
		[onUpdateOntology],
	);
	const discoverRemoteOntologies = useCallback(
		async (connection: IAppConnection) => {
			setLoadingConnectionId(connection.id);
			setRemoteErrors((current) => {
				const next = { ...current };
				delete next[connection.id];
				return next;
			});
			try {
				const contracts = await onLoadRemoteOntologies(
					connection.target_app_id,
				);
				setRemoteOntologies((current) => ({
					...current,
					[connection.id]: contracts,
				}));
			} catch (error) {
				setRemoteErrors((current) => ({
					...current,
					[connection.id]:
						error instanceof Error
							? error.message
							: "Could not discover remote ontologies.",
				}));
			} finally {
				setLoadingConnectionId(null);
			}
		},
		[onLoadRemoteOntologies],
	);
	if (ontologies.length === 0)
		return (
			<EmptyStudioState
				title="Nothing to expose yet"
				description="Set up an ontology, then publish its object and action contracts to connected projects."
				onCreate={onCreateOntology}
			/>
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
								{savingId === ontology.id && (
									<Loader2 className="h-4 w-4 animate-spin text-muted-foreground" />
								)}
							</div>
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
													disabled={loadingConnectionId === connection.id}
													onClick={() => discoverRemoteOntologies(connection)}
												>
													{loadingConnectionId === connection.id && (
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
														contracts.map((contract) => (
															<div
																key={contract.id}
																className="flex items-center justify-between gap-2 rounded-md bg-muted/40 px-2.5 py-2"
															>
																<div className="min-w-0">
																	<p className="truncate text-xs font-medium">
																		{contract.name}
																	</p>
																	<p className="text-[10px] text-muted-foreground">
																		{contract.nodes.length} objects ·{" "}
																		{contract.actions.length} actions
																	</p>
																</div>
																<Badge variant="outline">Remote</Badge>
															</div>
														))
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
