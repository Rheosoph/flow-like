"use client";

import { useTranslation } from "@flow-like/locales";
import { createId } from "@paralleldrive/cuid2";
import {
	ArrowRight,
	Box,
	Braces,
	Check,
	CheckCircle2,
	ChevronDown,
	ChevronRight,
	CircleDot,
	Cloud,
	Copy,
	Database,
	ExternalLink,
	FileKey,
	GitBranch,
	Hash,
	Layers3,
	Loader2,
	MoreVertical,
	Network,
	Pencil,
	Play,
	Plus,
	RefreshCw,
	Search,
	Share2,
	ShieldCheck,
	Trash2,
	Workflow,
	X,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { toast } from "sonner";
import { useInvalidateInvoke, useInvoke } from "../../../hooks/use-invoke";
import type { IBoardSummary } from "../../../lib/schema/flow/board-summary";
import { IVersionType } from "../../../lib/schema/flow/version-type";
import { useBackend } from "../../../state/backend-state";
import type {
	EdgeLabelMapping,
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
import {
	DropdownMenu,
	DropdownMenuContent,
	DropdownMenuItem,
	DropdownMenuSeparator,
	DropdownMenuTrigger,
} from "../../ui/dropdown-menu";
import {
	CopyButton,
	PropertyValue,
	inferValueKind,
} from "../../ui/graph/graph-node-inspector";
import { getGraphIcon } from "../../ui/graph/icons";
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
import {
	Sheet,
	SheetContent,
	SheetDescription,
	SheetHeader,
	SheetTitle,
} from "../../ui/sheet";
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
	const { t } = useTranslation("settings");
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
				<Plus className="h-4 w-4" /> {t('setUpOntology', 'Set up ontology')}
			</Button>
		</div>
	);
}

export function DataStudioOverview({
	ontologies,
	tableCount,
	remoteCount,
	onCreateOntology,
	onOpenOntology,
	onNavigate,
}: Readonly<
	StudioPanelBaseProps & {
		tableCount: number;
		remoteCount: number;
		onOpenOntology: (ontologyId: string) => void;
		onNavigate: (view: string) => void;
	}
>) {
	const { t } = useTranslation("settings");
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
			<div className="grid gap-3 sm:grid-cols-2 xl:grid-cols-5">
				{[
					{ label: t('ontologies', 'Ontologies'), value: ontologies.length, icon: Layers3 },
					{ label: t('objectTypes', 'Object types'), value: objectCount, icon: Box },
					{ label: t('actions', 'Actions'), value: actionCount, icon: Workflow },
					{ label: t('shared', 'Shared'), value: exposedCount, icon: Share2 },
					{ label: t('remote', 'Remote'), value: remoteCount, icon: Cloud },
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
							<CardTitle className="text-base">{t('yourSemanticLayer', 'Your semantic layer')}</CardTitle>
							<p className="mt-1 text-sm text-muted-foreground">{t('objectsRelationshipsViewsAndOperationsOverTablecount', 'Objects, relationships, views, and operations over {{tableCount}}', { tableCount })}{" "}
								tables.
							</p>
						</div>
						<Button size="sm" onClick={onCreateOntology}>
							<Plus className="h-4 w-4" /> {t('newOntology', 'New ontology')}
						</Button>
					</CardHeader>
					<CardContent>
						{ontologies.length === 0 ? (
							<div className="rounded-xl border border-dashed p-8 text-center">
								<p className="text-sm font-medium">
									{t('modelYourFirstBusinessObject', 'Model your first business object')}
								</p>
								<p className="mt-1 text-xs text-muted-foreground">
									{t('selectTablesAndDataStudioWillInferObjectIdsDisplayFieldsAndRelationships', "Select tables and Data Studio will infer object IDs, display fields, and relationships.")}
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
											<p className="text-xs text-muted-foreground">{t('lengthObjects', '{{length}} objects ·', { length: ontology.nodes.length })}{" "}{t('lengthRelationships', '{{length}} relationships', { length: ontology.edges.length })}</p>
										</div>
										{ontology.bindings_enabled && (
											<Badge variant="secondary">{t('bindings', 'Bindings')}</Badge>
										)}
										<ChevronRight className="h-4 w-4 text-muted-foreground" />
									</button>
								))}
								{ontologies.length > 5 && (
									<Button
										variant="ghost"
										size="sm"
										className="w-full justify-center text-muted-foreground"
										onClick={() => onNavigate("model")}
									>{t('viewAllLength', 'View all ({{length}})', { length: ontologies.length })}</Button>
								)}
							</div>
						)}
					</CardContent>
				</Card>

				<Card>
					<CardHeader className="pb-3">
						<CardTitle className="text-base">{t('startWithATask', 'Start with a task')}</CardTitle>
					</CardHeader>
					<CardContent className="space-y-2">
						{[
							{
								view: "objects",
								title: t('exploreBusinessObjects', 'Explore business objects'),
								description: t('searchAndInspectGeneratedObjectViews', 'Search and inspect generated object views'),
								icon: Search,
							},
							{
								view: "model",
								title: t('shapeTheModel', 'Shape the model'),
								description: t('reviewTypesLinksMappingsAndHealth', 'Review types, links, mappings, and health'),
								icon: GitBranch,
							},
							{
								view: "actions",
								title: t('connectAnAction', 'Connect an action'),
								description: t('bindAnOperationToATypedBoardEntry', 'Bind an operation to a typed board entry'),
								icon: Workflow,
							},
							{
								view: "sharing",
								title: t('exposeAContract', 'Expose a contract'),
								description: t('shareWithProjectsThroughAppConnections', 'Share with projects through app connections'),
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

interface ExplorerSource {
	/** Unique select value: overlay id for local, import id for remote. */
	value: string;
	name: string;
	overlay: GraphOverlay;
	/** Present when this source is an installed remote ontology. */
	remoteImportId?: string;
	/** Source app label, shown as the remote provenance. */
	sourceLabel?: string;
}

export function ObjectExplorerPanel({
	ontologies,
	remoteImports,
	initialSourceValue,
	onCreateOntology,
	onSample,
	onSampleRemote,
	onInvokeAction,
	resolveSourceName,
}: Readonly<
	StudioPanelBaseProps & {
		remoteImports?: RemoteOntologyImport[];
		initialSourceValue?: string;
		onSample: (
			ontologyId: string,
			objectType: string,
			limit: number,
		) => Promise<unknown[]>;
		onSampleRemote?: (
			importId: string,
			objectType: string,
			limit: number,
		) => Promise<unknown[]>;
		onInvokeAction: (
			ontologyId: string,
			actionId: string,
			payload: InvokeOntologyActionPayload,
			onStatus?: (run: OntologyActionRun) => void,
		) => Promise<OntologyActionRun>;
		resolveSourceName?: (targetAppId: string) => string;
	}
>) {
	const { t } = useTranslation("settings");
	const sources = useMemo<ExplorerSource[]>(() => {
		const local: ExplorerSource[] = ontologies.map((overlay) => ({
			value: overlay.id,
			name: overlay.name,
			overlay,
		}));
		const remote: ExplorerSource[] = (remoteImports ?? []).map((imported) => ({
			value: imported.id,
			name: imported.contract.name,
			// Remote actions run through their own governed invoke path, not the
			// local action endpoint — strip them so the object sheet stays read-only.
			overlay: { ...imported.contract, actions: [] },
			remoteImportId: imported.id,
			sourceLabel:
				resolveSourceName?.(imported.target_app_id) ?? imported.target_app_id,
		}));
		return [...local, ...remote];
	}, [ontologies, remoteImports, resolveSourceName]);

	const [selectedSourceValue, setSelectedSourceValue] = useState(
		initialSourceValue ?? sources[0]?.value ?? "",
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

	const source = useMemo(
		() =>
			sources.find((item) => item.value === selectedSourceValue) ?? sources[0],
		[sources, selectedSourceValue],
	);
	const ontology = source?.overlay;
	const objectType = useMemo(
		() =>
			ontology?.nodes.find(
				(object) => objectKey(object) === selectedObjectKey,
			) ?? ontology?.nodes[0],
		[ontology, selectedObjectKey],
	);

	useEffect(() => {
		if (!source) return;
		setSelectedSourceValue(source.value);
		if (!objectType && ontology?.nodes[0])
			setSelectedObjectKey(objectKey(ontology.nodes[0]));
		else if (objectType) setSelectedObjectKey(objectKey(objectType));
	}, [objectType, source, ontology]);

	const activeObjectKey = objectType ? objectKey(objectType) : "";
	const activeSelectionKey = `${source?.value ?? ""}:${activeObjectKey}`;
	const activeSelectionRef = useRef(activeSelectionKey);
	useEffect(() => {
		activeSelectionRef.current = activeSelectionKey;
		loadGeneration.current += 1;
		setSelectedRow(null);
		setRows([]);
		setError(null);
	}, [activeSelectionKey]);

	const loadObjects = useCallback(async () => {
		if (!source || !objectType) return;
		const generation = ++loadGeneration.current;
		const selectionKey = activeSelectionKey;
		setLoading(true);
		setError(null);
		try {
			const result = source.remoteImportId
				? ((await onSampleRemote?.(
						source.remoteImportId,
						objectType.label,
						100,
					)) ?? [])
				: await onSample(source.overlay.id, objectType.label, 100);
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
					: t('couldNotLoadObjects', 'Could not load objects.'),
			);
			setRows([]);
		} finally {
			if (generation === loadGeneration.current) setLoading(false);
		}
	}, [activeSelectionKey, objectType, onSample, onSampleRemote, source]);

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

	if (sources.length === 0) {
		return (
			<EmptyStudioState
				title={t('noObjectsToExplore', 'No objects to explore')}
				description={t('setUpAnOntologyToTurnNativeTablesIntoSearchableBusinessObjectsAndStandardObjectViews', 'Set up an ontology to turn native tables into searchable business objects and standard object views.')}
				onCreate={onCreateOntology}
			/>
		);
	}

	return (
		<div className="grid h-full min-h-0 grid-cols-1 overflow-hidden rounded-xl border lg:grid-cols-[260px_minmax(0,1fr)]">
			<aside className="min-h-0 border-b bg-muted/20 lg:border-r lg:border-b-0">
				<div className="border-b p-3">
					<Select
						value={source?.value}
						onValueChange={(value) => {
							setSelectedSourceValue(value);
							setSelectedObjectKey("");
						}}
					>
						<SelectTrigger
							className="bg-background"
							aria-label={t('selectOntology', 'Select ontology')}
						>
							<SelectValue />
						</SelectTrigger>
						<SelectContent>
							{sources.map((item) => (
								<SelectItem key={item.value} value={item.value}>
									<span className="flex items-center gap-2">
										<span className="truncate">{item.name}</span>
										{item.remoteImportId && (
											<Badge variant="outline" className="gap-1 text-[10px]">
												<Cloud className="h-3 w-3" /> {t('remote', 'Remote')}
											</Badge>
										)}
									</span>
								</SelectItem>
							))}
						</SelectContent>
					</Select>
				</div>
				<ScrollArea className="h-[180px] lg:h-[calc(100%-61px)]">
					<div className="space-y-1 p-2">
						<p className="px-2 py-1 text-[10px] font-semibold uppercase tracking-wider text-muted-foreground">
							{t('objectTypes', 'Object types')}
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
							<Badge variant="outline">{t('lengthPreview', '{{length}} preview', { length: visibleRows.length })}</Badge>
							{source?.remoteImportId && (
								<Badge variant="outline" className="gap-1">
									<Cloud className="h-3 w-3" />{t('remoteSourcelabel', 'Remote · {{sourceLabel}}', { sourceLabel: source.sourceLabel })}</Badge>
							)}
						</div>
						<p className="text-xs text-muted-foreground">
							{source?.remoteImportId
								? t('remoteObjectReadonly', 'Remote object · read-only')
								: t('standardObjectView', 'Standard object view')}{" "}
							{t('source2', '· source')} {objectType?.table}
						</p>
					</div>
					<div className="flex items-center gap-2">
						<div className="relative min-w-56">
							<Search className="absolute left-2.5 top-2.5 h-4 w-4 text-muted-foreground" />
							<Input
								value={query}
								onChange={(event) => setQuery(event.target.value)}
								placeholder={t('filterLoadedObjects', 'Filter loaded objects')}
								aria-label={t('filterLoadedObjects', 'Filter loaded objects')}
								className="pl-8"
							/>
							{query && (
								<Button
									variant="ghost"
									size="icon"
									className="absolute right-0 top-0 h-9 w-9"
									onClick={() => setQuery("")}
									aria-label={t('clearObjectFilter', 'Clear object filter')}
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
							aria-label={t('refreshObjects', 'Refresh objects')}
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
							{t('noObjectsInThisPreview', 'No objects in this preview.')}
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
										<span className="sr-only">{t('openObject', 'Open object')}</span>
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
												aria-label={t('openValVal2', 'Open {{val}} {{val2}}', { val: objectType?.label ?? "object", val2: String(
													row[objectType?.display_column ?? ""] ??
														row[objectType?.id_column ?? ""] ??
														index + 1,
												) })}
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

function CopyChip({ text, label }: Readonly<{ text: string; label: string }>) {
	const [copied, setCopied] = useState(false);
	const handleCopy = useCallback(() => {
		navigator.clipboard.writeText(text);
		setCopied(true);
		setTimeout(() => setCopied(false), 1500);
	}, [text]);
	return (
		<Button
			variant="outline"
			size="sm"
			className="h-7 shrink-0 gap-1.5 px-2.5 text-xs"
			onClick={handleCopy}
		>
			{copied ? (
				<Check className="h-3.5 w-3.5 text-green-500" />
			) : (
				<Copy className="h-3.5 w-3.5" />
			)}
			{label}
		</Button>
	);
}

function ObjectFieldCard({
	label,
	value,
}: Readonly<{ label: string; value: unknown }>) {
	return (
		<div className="min-w-0 rounded-xl border bg-muted/30 p-3">
			<div className="mb-1 flex items-center justify-between gap-2">
				<p className="truncate text-[10px] font-medium uppercase tracking-wider text-muted-foreground">
					{label}
				</p>
				<span className="shrink-0 text-[9px] uppercase tracking-wide text-muted-foreground/50">
					{inferValueKind(value).kind}
				</span>
			</div>
			<PropertyValue value={value} propKey={label} />
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
	const { t } = useTranslation("settings");
	const [selectedAction, setSelectedAction] =
		useState<OntologyActionDefinition | null>(null);
	const [showAllProperties, setShowAllProperties] = useState(false);
	const [lastRow, setLastRow] = useState(row);
	if (row !== lastRow) {
		setLastRow(row);
		setShowAllProperties(false);
	}
	const emptyType: NodeLabelMapping = {
		label: "",
		table: "",
		id_column: "",
		property_columns: [],
		style: { color: "", icon: "", size: { mode: "fixed" } },
	};
	const activeKey = objectKey(objectType ?? emptyType);
	const view = ontology?.object_views?.find(
		(item) => item.object_type === activeKey,
	);
	const titleProperty =
		view?.title_property ?? objectType?.display_column ?? objectType?.id_column;
	const prominent =
		view?.prominent_properties ??
		objectType?.property_columns.slice(0, 4).map((property) => property.name) ??
		[];
	const actions =
		ontology?.actions?.filter(
			(action) => action.enabled && action.object_type === activeKey,
		) ?? [];
	const prominentKeys = row ? prominent.filter((key) => key in row) : [];
	const prominentSet = new Set(prominentKeys);
	const prominentEntries = prominentKeys.map(
		(key) => [key, row?.[key]] as [string, unknown],
	);
	const restEntries = row
		? Object.entries(row).filter(([key]) => !prominentSet.has(key))
		: [];
	const hasProminent = prominentEntries.length > 0;
	const restCollapsed = hasProminent && !showAllProperties;
	const totalFields = prominentEntries.length + restEntries.length;

	const accentColor = objectType?.style?.color || "hsl(var(--primary))";
	const TypeIcon = getGraphIcon(objectType?.style?.icon ?? "database");
	const titleValue = String(
		row?.[titleProperty ?? ""] ??
			row?.[objectType?.id_column ?? ""] ??
			"Object",
	);
	const idRaw = row?.[objectType?.id_column ?? ""];
	const idValue = idRaw === undefined || idRaw === null ? "" : String(idRaw);
	const showIdChip = idValue !== "" && idValue !== titleValue;

	return (
		<Sheet open={Boolean(row)} onOpenChange={(open) => !open && onClose()}>
			<SheetContent className="w-full p-0 sm:max-w-xl">
				<div className="flex h-full min-h-0 flex-col">
					<SheetHeader className="gap-0 space-y-0 border-b p-5 pr-12 text-left">
						<div className="flex items-start gap-3.5">
							<div
								className="flex h-11 w-11 shrink-0 items-center justify-center rounded-xl text-white shadow-sm"
								style={{
									backgroundColor: accentColor,
									boxShadow: `0 0 0 4px color-mix(in srgb, ${accentColor} 14%, transparent)`,
								}}
							>
								<TypeIcon className="h-5 w-5" />
							</div>
							<div className="min-w-0 flex-1">
								<p className="truncate text-[11px] font-semibold uppercase tracking-wider text-muted-foreground">
									{objectType?.label || "Object"}
								</p>
								<SheetTitle className="mt-0.5 truncate text-xl leading-tight">
									{titleValue}
								</SheetTitle>
								{showIdChip && (
									<div className="group mt-1.5 flex items-center gap-1 text-muted-foreground">
										<Hash className="h-3 w-3 shrink-0 opacity-70" />
										<span className="truncate font-mono text-xs">
											{idValue}
										</span>
										<CopyButton text={idValue} />
									</div>
								)}
							</div>
						</div>
						<SheetDescription className="sr-only">
							{objectType?.label || "Object"} {`details from ontology`}{" "}
							{ontology?.name ?? ""}
						</SheetDescription>
					</SheetHeader>

					{row && (
						<div className="relative min-h-0 flex-1 overflow-y-auto">
							<div className="space-y-6 p-5">
								{actions.length > 0 && (
									<section>
										<p className="mb-2.5 flex items-center gap-1.5 text-[11px] font-semibold uppercase tracking-wider text-muted-foreground">
											<Workflow className="h-3.5 w-3.5" />
											{t('actions', 'Actions')}
										</p>
										<div className="space-y-1.5">
											{actions.map((action) => (
												<button
													key={action.id}
													type="button"
													onClick={() => setSelectedAction(action)}
													className="group flex w-full items-center gap-3 rounded-xl border bg-card px-3.5 py-2.5 text-left transition-colors hover:border-primary/40 hover:bg-accent"
												>
													<span className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg bg-primary/10 text-primary">
														<Workflow className="h-4 w-4" />
													</span>
													<span className="min-w-0 flex-1">
														<span className="block truncate text-sm font-medium">
															{action.name}
														</span>
														{action.description && (
															<span className="block truncate text-xs text-muted-foreground">
																{action.description}
															</span>
														)}
													</span>
													<ArrowRight className="h-4 w-4 shrink-0 text-muted-foreground transition-transform group-hover:translate-x-0.5 group-hover:text-foreground" />
												</button>
											))}
										</div>
									</section>
								)}

								{hasProminent && (
									<section>
										<p className="mb-2.5 text-[11px] font-semibold uppercase tracking-wider text-muted-foreground">
											{t('highlights', 'Highlights')}
										</p>
										<div className="grid grid-cols-2 gap-2.5">
											{prominentEntries.map(([key, value]) => (
												<ObjectFieldCard
													key={key}
													label={humanizeIdentifier(key)}
													value={value}
												/>
											))}
										</div>
									</section>
								)}

								{restEntries.length > 0 && (
									<section>
										<div className="mb-2.5 flex items-center justify-between">
											<p className="text-[11px] font-semibold uppercase tracking-wider text-muted-foreground">
												{hasProminent ? "More properties" : "Properties"}
											</p>
											<span className="text-[10px] text-muted-foreground">{t('totalfieldsFields', '{{totalFields}} fields', { totalFields })}</span>
										</div>
										{!restCollapsed && (
											<div className="space-y-1.5">
												{restEntries.map(([key, value]) => (
													<ObjectFieldCard
														key={key}
														label={humanizeIdentifier(key)}
														value={value}
													/>
												))}
											</div>
										)}
										{hasProminent && (
											<Button
												variant="ghost"
												size="sm"
												className="mt-1.5 w-full justify-center gap-1 text-muted-foreground"
												onClick={() =>
													setShowAllProperties((current) => !current)
												}
											>
												<ChevronDown
													className={`h-3.5 w-3.5 transition-transform ${showAllProperties ? "rotate-180" : ""}`}
												/>
												{showAllProperties
													? "Show fewer"
													: t('showAllLengthFields', 'Show all {{length}} fields', { length: restEntries.length })}
											</Button>
										)}
									</section>
								)}

								{totalFields === 0 && (
									<p className="rounded-xl border border-dashed p-6 text-center text-sm text-muted-foreground">
										{t('thisObjectHasNoPropertiesToDisplay', 'This object has no properties to display.')}
									</p>
								)}
							</div>

							<div className="sticky bottom-0 flex items-center justify-between gap-3 border-t bg-background/80 p-4 backdrop-blur">
								<div className="flex min-w-0 items-center gap-2 text-xs text-muted-foreground">
									<Database className="h-3.5 w-3.5 shrink-0" />
									<span className="truncate">
										<span className="text-foreground">
											{objectType?.table || "—"}
										</span>
										{ontology?.name ? ` · ${ontology.name}` : ""}
									</span>
								</div>
								<CopyChip
									text={JSON.stringify(row, null, 2)}
									label={t('copyJson', 'Copy JSON')}
								/>
							</div>
						</div>
					)}
				</div>

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

export function initialActionParameters(
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

export function OntologyActionParameterForm({
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
	const { t } = useTranslation("settings");
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
				<Label>{t('parameters', 'Parameters')}</Label>
				<p className="text-xs text-muted-foreground">
					{t('valuesAreValidatedAgainstTheSavedActionContract', 'Values are validated against the saved action contract.')}
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
											placeholder={t('chooseVal', 'Choose {{val}}', { val: label.toLowerCase() })}
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
										{t('enterValidJson', 'Enter valid JSON.')}
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
					{t('completeAllRequiredParameters', 'Complete all required parameters.')}
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
	const { t } = useTranslation("settings");
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
						t('theActionEndedWithStatusVal', 'The action ended with status {{val}}.', { val: result.status.toLowerCase() }),
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
					: t('theActionCouldNotBeStarted', 'The action could not be started.'),
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
						{action?.name ?? t('applyAction', 'Apply action')}
					</DialogTitle>
					<DialogDescription>
						{action?.description ??
							t('runThisGovernedOperationThroughItsSavedWorkflowBinding', 'Run this governed operation through its saved workflow binding.')}
					</DialogDescription>
				</DialogHeader>
				<div className="space-y-4 py-1" aria-busy={submitting}>
					<div className="rounded-lg border bg-muted/30 p-3">
						<p className="text-[10px] font-medium uppercase tracking-wider text-muted-foreground">
							{t('target2', 'Target')} {objectType?.label ?? "object"}
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
							{t('confirmBeforeApplying', 'Confirm before applying')}
						</p>
						<p className="mt-1">
							{t('theServerReloadsThisObjectValidatesTheSavedContractAndRunsOnlyThePinnedActionImplementation', "The server reloads this object, validates the saved contract, and runs only the pinned action implementation.")}
						</p>
					</div>
					<div aria-live="polite">
						{submitting && !succeeded && !failed && (
							<div className="flex items-center gap-2 rounded-lg bg-primary/5 p-3 text-sm">
								<Loader2 className="h-4 w-4 animate-spin text-primary" />
								<div>
									<p>
										{run?.status === "Running"
											? t('actionRunning', 'Action running…')
											: t('submittingAction', 'Submitting action…')}
									</p>
									{run?.run_id && (
										<p className="font-mono text-[10px] text-muted-foreground">{t('runRun_id', 'Run {{run_id}}', { run_id: run.run_id })}</p>
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
											? t('actionApplied', 'Action applied')
											: humanizeIdentifier(run.status)}
									</p>
									{run.run_id && (
										<p className="font-mono text-[10px] text-muted-foreground">{t('runRun_id', 'Run {{run_id}}', { run_id: run.run_id })}</p>
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
									<p className="mt-1 font-mono text-[10px]">{t('runRun_id', 'Run {{run_id}}', { run_id: run.run_id })}</p>
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
							{t('confirm', 'Confirm')} {action?.name ?? "action"}
						</Button>
					)}
				</DialogFooter>
			</DialogContent>
		</Dialog>
	);
}

function OntologyLifecycleMenu({
	appId,
	ontology,
}: Readonly<{ appId: string; ontology: GraphOverlay }>) {
	const { t } = useTranslation("settings");
	const backend = useBackend();
	const invalidate = useInvalidateInvoke();
	const [renameOpen, setRenameOpen] = useState(false);
	const [deleteOpen, setDeleteOpen] = useState(false);
	const [nameDraft, setNameDraft] = useState(ontology.name);
	const [busy, setBusy] = useState(false);
	const [error, setError] = useState<string | null>(null);

	const refreshOverlays = useCallback(async () => {
		await invalidate(backend.graphState.listOverlays, [appId]);
		await invalidate(backend.boardState.getCatalog, [appId]);
	}, [appId, backend.boardState, backend.graphState, invalidate]);

	const rename = useCallback(async () => {
		const nextName = nameDraft.trim();
		if (!nextName) return;
		if (nextName === ontology.name) {
			setRenameOpen(false);
			return;
		}
		setBusy(true);
		setError(null);
		try {
			await backend.graphState.updateOverlay(appId, ontology.id, {
				name: nextName,
				expected_updated_at: ontology.updated_at,
			});
			await refreshOverlays();
			setRenameOpen(false);
		} catch (renameError) {
			setError(
				renameError instanceof Error
					? renameError.message
					: t('couldNotRenameTheOntology', 'Could not rename the ontology.'),
			);
		} finally {
			setBusy(false);
		}
	}, [appId, backend.graphState, nameDraft, ontology, refreshOverlays]);

	const remove = useCallback(async () => {
		setBusy(true);
		setError(null);
		try {
			await backend.graphState.deleteOverlay(appId, ontology.id);
			await refreshOverlays();
			setDeleteOpen(false);
		} catch (deleteError) {
			setError(
				deleteError instanceof Error
					? deleteError.message
					: t('couldNotDeleteTheOntology', 'Could not delete the ontology.'),
			);
		} finally {
			setBusy(false);
		}
	}, [appId, backend.graphState, ontology.id, refreshOverlays]);

	return (
		<>
			<DropdownMenu>
				<DropdownMenuTrigger asChild>
					<Button
						variant="ghost"
						size="icon"
						className="h-7 w-7"
						aria-label={t('manageName', 'Manage {{name}}', { name: ontology.name })}
					>
						<MoreVertical className="h-4 w-4" />
					</Button>
				</DropdownMenuTrigger>
				<DropdownMenuContent align="end">
					<DropdownMenuItem
						onSelect={(event) => {
							event.preventDefault();
							setNameDraft(ontology.name);
							setError(null);
							setRenameOpen(true);
						}}
					>
						<Pencil className="h-4 w-4" /> {t('rename', 'Rename')}
					</DropdownMenuItem>
					<DropdownMenuSeparator />
					<DropdownMenuItem
						className="text-destructive focus:text-destructive"
						onSelect={(event) => {
							event.preventDefault();
							setError(null);
							setDeleteOpen(true);
						}}
					>
						<Trash2 className="h-4 w-4" /> {t('delete', 'Delete')}
					</DropdownMenuItem>
				</DropdownMenuContent>
			</DropdownMenu>

			<Dialog
				open={renameOpen}
				onOpenChange={(open) => {
					if (busy) return;
					if (!open) setError(null);
					setRenameOpen(open);
				}}
			>
				<DialogContent className="sm:max-w-md">
					<DialogHeader>
						<DialogTitle>{t('renameOntology', 'Rename ontology')}</DialogTitle>
						<DialogDescription>
							{t('updateTheDisplayNameOfThisSemanticLayer', 'Update the display name of this semantic layer.')}
						</DialogDescription>
					</DialogHeader>
					<div className="grid gap-1.5 py-1">
						<Label htmlFor={`rename-ontology-${ontology.id}`}>Name</Label>
						<Input
							id={`rename-ontology-${ontology.id}`}
							value={nameDraft}
							disabled={busy}
							onChange={(event) => setNameDraft(event.target.value)}
							onKeyDown={(event) => {
								if (event.key === "Enter") {
									event.preventDefault();
									void rename();
								}
							}}
						/>
						{error && (
							<p role="alert" className="text-sm text-destructive">
								{error}
							</p>
						)}
					</div>
					<DialogFooter>
						<Button
							variant="ghost"
							disabled={busy}
							onClick={() => setRenameOpen(false)}
						>
							{t('cancel', 'Cancel')}
						</Button>
						<Button
							onClick={() => void rename()}
							disabled={busy || !nameDraft.trim()}
						>
							{busy && <Loader2 className="h-4 w-4 animate-spin" />}
							{t('saveName', 'Save name')}
						</Button>
					</DialogFooter>
				</DialogContent>
			</Dialog>

			<AlertDialog
				open={deleteOpen}
				onOpenChange={(open) => {
					if (busy) return;
					if (!open) setError(null);
					setDeleteOpen(open);
				}}
			>
				<AlertDialogContent>
					<AlertDialogHeader>
						<AlertDialogTitle>{`Delete ${ontology.name}?`}</AlertDialogTitle>
						<AlertDialogDescription>
							{t('thisRemovesTheSemanticLayerItsObjectViewsAndActionBindingsYourUnderlyingDataTablesAreNotDeletedOnlyThisOntologyDefinition', "This removes the semantic layer, its object views, and action bindings. Your underlying data tables are not deleted — only this ontology definition.")}
						</AlertDialogDescription>
					</AlertDialogHeader>
					{error && (
						<p role="alert" className="text-sm text-destructive">
							{error}
						</p>
					)}
					<AlertDialogFooter>
						<AlertDialogCancel disabled={busy}>{t('keepOntology', 'Keep ontology')}</AlertDialogCancel>
						<AlertDialogAction
							className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
							disabled={busy}
							onClick={(event) => {
								event.preventDefault();
								void remove();
							}}
						>
							{busy && <Loader2 className="h-3.5 w-3.5 animate-spin" />}
							{t('deleteOntology', 'Delete ontology')}
						</AlertDialogAction>
					</AlertDialogFooter>
				</AlertDialogContent>
			</AlertDialog>
		</>
	);
}

function encodeEdgeTarget(edge: EdgeLabelMapping): string {
	if (edge.dst_ontology) return `local:${edge.dst_ontology}`;
	if (edge.dst_binding_id) return `remote:${edge.dst_binding_id}`;
	return "self";
}

function decodeEdgeTarget(value: string): Partial<EdgeLabelMapping> {
	if (value.startsWith("local:")) {
		return { dst_ontology: value.slice(6), dst_binding_id: undefined };
	}
	if (value.startsWith("remote:")) {
		return { dst_binding_id: value.slice(7), dst_ontology: undefined };
	}
	return { dst_ontology: undefined, dst_binding_id: undefined };
}

function RelationshipRow({
	edge,
	index,
	otherOntologies,
	installedOntologies,
	onChange,
}: Readonly<{
	edge: EdgeLabelMapping;
	index: number;
	otherOntologies: GraphOverlay[];
	installedOntologies: RemoteOntologyImport[];
	onChange?: (index: number, patch: Partial<EdgeLabelMapping>) => void;
}>) {
	const { t } = useTranslation("settings");
	const rowId = edge.id ?? edge.api_name ?? `edge-${index}`;
	const summary = (
		<div className="flex items-center gap-2 text-sm">
			<Badge variant="outline">{edge.src_label}</Badge>
			<ArrowRight className="h-3.5 w-3.5 text-muted-foreground" />
			<span className="font-medium">{humanizeIdentifier(edge.label)}</span>
			<ArrowRight className="h-3.5 w-3.5 text-muted-foreground" />
			<Badge variant="outline">{edge.dst_label}</Badge>
			<code className="ml-auto hidden text-[10px] text-muted-foreground sm:block">{`${edge.table}.${edge.dst_column}`}</code>
		</div>
	);

	if (!onChange) {
		return <div className="rounded-lg border px-3 py-2">{summary}</div>;
	}

	return (
		<div className="space-y-2.5 rounded-lg border px-3 py-2.5">
			{summary}
			<div className="flex flex-col gap-2 sm:flex-row sm:items-center sm:justify-between">
				<div className="flex items-center gap-2">
					<Switch
						id={`edge-containment-${rowId}`}
						checked={Boolean(edge.containment)}
						onCheckedChange={(checked) =>
							onChange(
								index,
								checked
									? { containment: true }
									: {
											containment: false,
											dst_ontology: undefined,
											dst_binding_id: undefined,
										},
							)
						}
					/>
					<div>
						<Label
							htmlFor={`edge-containment-${rowId}`}
							className="text-xs font-medium"
						>
							{t('hierarchy', 'Hierarchy')}
						</Label>
						<p className="text-[10px] text-muted-foreground">
							{t('drilldownParentChild', 'Drill-down parent → child')}
						</p>
					</div>
				</div>
				{edge.containment && (
					<Select
						value={encodeEdgeTarget(edge)}
						onValueChange={(value) => onChange(index, decodeEdgeTarget(value))}
					>
						<SelectTrigger
							className="h-8 w-full text-xs sm:w-64"
							aria-label={t('childObjectLocation', 'Child object location')}
						>
							<SelectValue placeholder={t('childLocation', 'Child location')} />
						</SelectTrigger>
						<SelectContent>
							<SelectItem value="self">{t('thisOntology', 'This ontology')}</SelectItem>
							{otherOntologies.map((ontology) => (
								<SelectItem key={ontology.id} value={`local:${ontology.id}`}>
									{ontology.name}
								</SelectItem>
							))}
							{installedOntologies.map((imported) => (
								<SelectItem key={imported.id} value={`remote:${imported.id}`}>{t('remoteName', 'Remote: {{name}}', { name: imported.contract.name })}</SelectItem>
							))}
						</SelectContent>
					</Select>
				)}
			</div>
		</div>
	);
}

export function OntologyModelPanel({
	ontologies,
	appId,
	installedOntologies,
	onCreateOntology,
	onOpenOntology,
	onSaveEdges,
}: Readonly<
	StudioPanelBaseProps & {
		appId?: string;
		installedOntologies?: RemoteOntologyImport[];
		onOpenOntology: (ontologyId: string) => void;
		onSaveEdges?: (
			ontologyId: string,
			edges: EdgeLabelMapping[],
		) => Promise<void> | void;
	}
>) {
	const { t } = useTranslation("settings");
	const [selectedId, setSelectedId] = useState(ontologies[0]?.id ?? "");
	const selected =
		ontologies.find((ontology) => ontology.id === selectedId) ?? ontologies[0];
	useEffect(() => {
		if (selected) setSelectedId(selected.id);
	}, [selected]);
	const [edgesDraft, setEdgesDraft] = useState<EdgeLabelMapping[]>(
		() => ontologies[0]?.edges ?? [],
	);
	const edgesDraftRef = useRef(edgesDraft);
	const confirmedEdgesRef = useRef(
		new Map<string, EdgeLabelMapping[]>(
			ontologies[0] ? [[ontologies[0].id, ontologies[0].edges]] : [],
		),
	);
	const edgesDraftsRef = useRef(
		new Map<string, EdgeLabelMapping[]>(
			ontologies[0] ? [[ontologies[0].id, ontologies[0].edges]] : [],
		),
	);
	const selectedIdRef = useRef(selected?.id);
	const saveQueueRef = useRef<Promise<void>>(Promise.resolve());
	const latestSaveVersionRef = useRef(new Map<string, number>());
	const pendingSavesRef = useRef(new Map<string, number>());

	useEffect(() => {
		if (!selected) return;
		const selectionChanged = selectedIdRef.current !== selected.id;
		selectedIdRef.current = selected.id;
		const hasPendingSave = (pendingSavesRef.current.get(selected.id) ?? 0) > 0;
		if (!hasPendingSave) {
			confirmedEdgesRef.current.set(selected.id, selected.edges);
			edgesDraftsRef.current.set(selected.id, selected.edges);
			edgesDraftRef.current = selected.edges;
			setEdgesDraft(selected.edges);
		} else if (selectionChanged) {
			// Preserve this ontology's optimistic draft while its serialized save is
			// still pending, even if the user briefly viewed another ontology.
			const draft = edgesDraftsRef.current.get(selected.id) ?? selected.edges;
			if (!confirmedEdgesRef.current.has(selected.id)) {
				confirmedEdgesRef.current.set(selected.id, selected.edges);
			}
			edgesDraftRef.current = draft;
			setEdgesDraft(draft);
		} else {
			// A successful earlier item in the queue may have refreshed the parent
			// while a later draft is still pending. Track it as the rollback point.
			confirmedEdgesRef.current.set(selected.id, selected.edges);
		}
	}, [selected]);
	const otherOntologies = useMemo(
		() => ontologies.filter((ontology) => ontology.id !== selected?.id),
		[ontologies, selected?.id],
	);
	const handleEdgeChange = useCallback(
		(index: number, patch: Partial<EdgeLabelMapping>) => {
			if (!selected || !onSaveEdges) return;
			const next = edgesDraftRef.current.map((edge, edgeIndex) =>
				edgeIndex === index ? { ...edge, ...patch } : edge,
			);
			edgesDraftRef.current = next;
			edgesDraftsRef.current.set(selected.id, next);
			setEdgesDraft(next);

			const ontologyId = selected.id;
			const saveEdges = onSaveEdges;
			const saveVersion =
				(latestSaveVersionRef.current.get(ontologyId) ?? 0) + 1;
			latestSaveVersionRef.current.set(ontologyId, saveVersion);
			pendingSavesRef.current.set(
				ontologyId,
				(pendingSavesRef.current.get(ontologyId) ?? 0) + 1,
			);
			saveQueueRef.current = saveQueueRef.current
				.then(async () => {
					const isLatestSave = () =>
						latestSaveVersionRef.current.get(ontologyId) === saveVersion;
					try {
						await saveEdges(ontologyId, next);
						confirmedEdgesRef.current.set(ontologyId, next);
						if (selectedIdRef.current === ontologyId && isLatestSave()) {
							edgesDraftsRef.current.set(ontologyId, next);
							edgesDraftRef.current = next;
							setEdgesDraft(next);
						}
					} catch (error) {
						if (isLatestSave()) {
							const confirmed = confirmedEdgesRef.current.get(ontologyId) ?? [];
							edgesDraftsRef.current.set(ontologyId, confirmed);
							if (selectedIdRef.current === ontologyId) {
								edgesDraftRef.current = confirmed;
								setEdgesDraft(confirmed);
							}
							toast.error(
								error instanceof Error
									? t('failedToSaveRelationshipMessage', 'Failed to save relationship: {{message}}', { message: error.message })
									: t('failedToSaveRelationship', 'Failed to save relationship'),
							);
						}
					}
				})
				.finally(() => {
					const pending = (pendingSavesRef.current.get(ontologyId) ?? 1) - 1;
					if (pending > 0) pendingSavesRef.current.set(ontologyId, pending);
					else pendingSavesRef.current.delete(ontologyId);
				});
		},
		[onSaveEdges, selected],
	);
	if (ontologies.length === 0)
		return (
			<EmptyStudioState
				title={t('buildTheSharedModel', 'Build the shared model')}
				description={t('chooseNativeTablesAndDataStudioWillInferStableObjectIdentitiesDisplayFieldsAndForeignkeyRelationships', 'Choose native tables and Data Studio will infer stable object identities, display fields, and foreign-key relationships.')}
				onCreate={onCreateOntology}
			/>
		);
	return (
		<div className="grid gap-5 xl:grid-cols-[320px_minmax(0,1fr)]">
			<div className="space-y-3">
				<div className="flex items-center justify-between">
					<div>
						<h3 className="font-semibold">{t('ontologies', 'Ontologies')}</h3>
						<p className="text-xs text-muted-foreground">
							{t('savedSemanticContracts', 'Saved semantic contracts')}
						</p>
					</div>
					<Button size="sm" onClick={onCreateOntology}>
						<Plus className="h-4 w-4" /> {t('new', 'New')}
					</Button>
				</div>
				{ontologies.map((ontology) => (
					<div
						key={ontology.id}
						className={`relative rounded-xl border transition-colors ${ontology.id === selected?.id ? "border-primary bg-primary/5" : "hover:bg-muted/40"}`}
					>
						<button
							type="button"
							onClick={() => setSelectedId(ontology.id)}
							className="w-full p-4 pr-24 text-left"
						>
							<p className="truncate font-medium">{ontology.name}</p>
							<p className="mt-1 line-clamp-2 text-xs text-muted-foreground">
								{ontology.description ?? t('noDescriptionYet', 'No description yet')}
							</p>
							<div className="mt-3 flex gap-2 text-xs text-muted-foreground">
								<span>{t('lengthObjects2', '{{length}} objects', { length: ontology.nodes.length })}</span>
								<span>·</span>
								<span>{t('lengthLinks', '{{length}} links', { length: ontology.edges.length })}</span>
							</div>
						</button>
						<div className="absolute right-3 top-3 flex items-center gap-1">
							{ontology.exposed ? (
								<Badge variant="secondary">{t('shared', 'Shared')}</Badge>
							) : (
								<Badge variant="outline">{t('private', 'Private')}</Badge>
							)}
							{appId && (
								<OntologyLifecycleMenu appId={appId} ontology={ontology} />
							)}
						</div>
					</div>
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
										{t('bindingsGenerated', 'Bindings generated')}
									</Badge>
								)}
							</div>
							<p className="mt-1 text-sm text-muted-foreground">
								{selected.description ?? t('aSemanticModelOverProjectData', 'A semantic model over project data.')}
							</p>
						</div>
						<Button
							variant="outline"
							onClick={() => onOpenOntology(selected.id)}
						>
							<Network className="h-4 w-4" /> {t('exploreDataGraph', 'Explore data graph')}{" "}
							<ExternalLink className="h-3.5 w-3.5" />
						</Button>
					</div>
					<Separator />
					<div>
						<div className="mb-3 flex items-center justify-between">
							<div>
								<h3 className="text-sm font-medium">{t('objectTypes', 'Object types')}</h3>
								<p className="text-xs text-muted-foreground">
									{`Business objects compiled from native tables`}
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
											<p className="mt-1 text-xs text-muted-foreground">{`${object.table} · ID ${object.id_column}`}</p>
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
								<h3 className="text-sm font-medium">{t('relationships', 'Relationships')}</h3>
								<p className="text-xs text-muted-foreground">
									{t('inferredLinksCanBeRefinedInTheGraphEditor', 'Inferred links can be refined in the graph editor')}
								</p>
							</div>
							<Badge variant="secondary">{selected.edges.length}</Badge>
						</div>
						{selected.edges.length === 0 ? (
							<div className="rounded-lg border border-dashed p-5 text-center text-sm text-muted-foreground">
								{t('noRelationshipsInferredYet', 'No relationships inferred yet.')}
							</div>
						) : (
							<div className="space-y-2">
								{edgesDraft.map((edge, index) => (
									<RelationshipRow
										key={
											edge.id ??
											edge.api_name ??
											`${edge.src_label}-${edge.dst_label}-${index}`
										}
										edge={edge}
										index={index}
										otherOntologies={otherOntologies}
										installedOntologies={installedOntologies ?? []}
										onChange={onSaveEdges ? handleEdgeChange : undefined}
									/>
								))}
							</div>
						)}
					</div>
				</div>
			)}
		</div>
	);
}

const CURRENT_DRAFT_VERSION = "current";

function versionKey(version: readonly number[]): string {
	return `${version[0] ?? 0}.${version[1] ?? 0}.${version[2] ?? 0}`;
}

export function OntologyActionsPanel({
	ontologies,
	boards,
	appId,
	onCreateOntology,
	onNeedBoards,
	onSaveActions,
}: Readonly<
	StudioPanelBaseProps & {
		/** Listing only; the selected board's graph is fetched on demand. */
		boards: IBoardSummary[];
		appId?: string;
		onNeedBoards: () => void;
		onSaveActions: (
			ontologyId: string,
			actions: OntologyActionDefinition[],
		) => Promise<void>;
	}
>) {
	const { t } = useTranslation("settings");
	const backend = useBackend();
	const [dialogOpen, setDialogOpen] = useState(false);
	const [ontologyId, setOntologyId] = useState(ontologies[0]?.id ?? "");
	const [name, setName] = useState("");
	const [description, setDescription] = useState("");
	const [objectType, setObjectType] = useState("");
	const [boardId, setBoardId] = useState("");
	const [startNodeId, setStartNodeId] = useState("");
	// null selects the board's working draft (published on save); a tuple pins
	// an existing immutable board version.
	const [selectedVersion, setSelectedVersion] = useState<
		[number, number, number] | null
	>(null);
	const [publishedVersions, setPublishedVersions] = useState<
		[number, number, number][]
	>([]);
	const [publishingVersion, setPublishingVersion] = useState(false);
	const [editingActionId, setEditingActionId] = useState<string | null>(null);
	const [actionEnabled, setActionEnabled] = useState(true);
	const [allowBulk, setAllowBulk] = useState(false);
	const [actionExposed, setActionExposed] = useState(true);
	const [saving, setSaving] = useState(false);
	const [saveError, setSaveError] = useState<string | null>(null);
	const [repairingOntologyId, setRepairingOntologyId] = useState<string | null>(
		null,
	);
	const [repairError, setRepairError] = useState<string | null>(null);
	const ontology =
		ontologies.find((item) => item.id === ontologyId) ?? ontologies[0];
	const boardSummary = boards.find((item) => item.id === boardId);
	// Only the selected board's graph is needed (its start nodes and their pin schemas), so it is
	// fetched on demand instead of shipping every board of the app.
	const selectedBoard = useInvoke(
		backend.boardState.getBoard,
		backend.boardState,
		[appId ?? "", boardId],
		Boolean(appId) && boardId !== "",
	);
	const board = selectedBoard.data;
	const startNodes = useMemo(
		() =>
			board ? Object.values(board.nodes).filter((node) => node.start) : [],
		[board],
	);
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
	// Load the board's published (immutable) versions so the user can pin one.
	useEffect(() => {
		if (!appId || !boardId) {
			setPublishedVersions([]);
			return;
		}
		let cancelled = false;
		backend.boardState
			.getBoardVersions(appId, boardId)
			.then((versions) => {
				if (cancelled) return;
				const sorted = [...versions].sort((a, b) => {
					for (let index = 0; index < 3; index += 1) {
						if ((b[index] ?? 0) !== (a[index] ?? 0))
							return (b[index] ?? 0) - (a[index] ?? 0);
					}
					return 0;
				});
				setPublishedVersions(sorted);
			})
			.catch(() => {
				if (!cancelled) setPublishedVersions([]);
			});
		return () => {
			cancelled = true;
		};
	}, [appId, boardId, backend.boardState]);

	const resetActionEditor = useCallback(() => {
		setEditingActionId(null);
		setName("");
		setDescription("");
		setBoardId("");
		setStartNodeId("");
		setSelectedVersion(null);
		setActionEnabled(true);
		setAllowBulk(false);
		setActionExposed(true);
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
				setSelectedVersion(action.board_version ?? null);
				setActionEnabled(action.enabled);
				setAllowBulk(action.allow_bulk);
				setActionExposed(action.exposed ?? true);
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

	const publishDraftVersion = useCallback(async () => {
		if (!appId || !boardId) return;
		setPublishingVersion(true);
		setSaveError(null);
		try {
			await backend.boardState.createBoardVersion(
				appId,
				boardId,
				IVersionType.Patch,
			);
			const versions = await backend.boardState.getBoardVersions(
				appId,
				boardId,
			);
			const sorted = [...versions].sort((a, b) => {
				for (let index = 0; index < 3; index += 1) {
					if ((b[index] ?? 0) !== (a[index] ?? 0))
						return (b[index] ?? 0) - (a[index] ?? 0);
				}
				return 0;
			});
			setPublishedVersions(sorted);
			if (sorted[0]) setSelectedVersion(sorted[0]);
		} catch (error) {
			setSaveError(
				error instanceof Error
					? error.message
					: t('theBoardVersionCouldNotBePublished', 'The board version could not be published.'),
			);
		} finally {
			setPublishingVersion(false);
		}
	}, [appId, boardId, backend.boardState]);

	const boardsRequestedRef = useRef(false);
	useEffect(() => {
		if (boardsRequestedRef.current) return;
		boardsRequestedRef.current = true;
		onNeedBoards();
	}, [onNeedBoards]);

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
			// A pinned published version is used verbatim; otherwise the board's
			// working draft is pinned and published server-side on save.
			const draft = board?.version ?? boardSummary?.version;
			const draftVersion =
				Array.isArray(draft) && draft.length === 3
					? ([Number(draft[0]), Number(draft[1]), Number(draft[2])] as [
							number,
							number,
							number,
						])
					: previous?.board_id === boardId
						? previous.board_version
						: undefined;
			const boardVersion = selectedVersion ?? draftVersion;
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
				exposed: actionExposed,
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
					: t('theOntologyActionCouldNotBeSaved', 'The ontology action could not be saved.'),
			);
		} finally {
			setSaving(false);
		}
	}, [
		board?.version,
		boardSummary?.version,
		boardId,
		selectedVersion,
		actionEnabled,
		allowBulk,
		actionExposed,
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
						: t('theActionBindingCouldNotBeRefreshed', 'The action binding could not be refreshed.'),
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
						: t('theActionCouldNotBeRemoved', 'The action could not be removed.'),
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
				title={t('actionsStartWithObjects', 'Actions start with objects')}
				description={t('createAnOntologyFirstThenBindObjectlevelOperationsToTypedBoardEntryNodes', 'Create an ontology first, then bind object-level operations to typed board entry nodes.')}
				onCreate={onCreateOntology}
			/>
		);
	return (
		<div className="space-y-5">
			<div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
				<div>
					<h3 className="font-semibold">{t('ontologyActions', 'Ontology actions')}</h3>
					<p className="text-sm text-muted-foreground">
						{t('governedObjectOperationsBackedByAPinnedBoardAndStartNode', 'Governed object operations backed by a pinned board and start node.')}
					</p>
				</div>
				<Button onClick={() => openActionEditor()}>
					<Plus className="h-4 w-4" /> {t('defineAction', 'Define action')}
				</Button>
			</div>
			{allActions.length === 0 ? (
				<div className="rounded-xl border border-dashed p-10 text-center">
					<div className="mx-auto mb-3 w-fit rounded-xl bg-primary/10 p-3 text-primary">
						<Workflow className="h-5 w-5" />
					</div>
					<p className="font-medium">{t('noActionsDefined', 'No actions defined')}</p>
					<p className="mt-1 text-sm text-muted-foreground">
						{t('bindAnObjectOperationToAnExistingBoardEntryNode', 'Bind an object operation to an existing board entry node.')}
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
												<p className="text-xs text-muted-foreground">{`${owner.name} ·`}{" "}
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
											<span className="text-muted-foreground">{t('board2', 'Board')}</span>
											<p className="mt-0.5 truncate font-medium">
												{boards.find((item) => item.id === action.board_id)
													?.name ?? action.board_id}
											</p>
										</div>
										<div className="rounded-lg bg-muted/40 p-2">
											<span className="text-muted-foreground">{t('binding', 'Binding')}</span>
											<p className="mt-0.5 truncate font-mono text-[10px]">
												{action.start_node_id ?? t('notSet', 'Not set')}
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
											{t('edit', 'Edit')}
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
													{t('remove', 'Remove')}
												</Button>
											</AlertDialogTrigger>
											<AlertDialogContent>
												<AlertDialogHeader>
													<AlertDialogTitle>{t('removeName', 'Remove {{name}}?', { name: action.name })}</AlertDialogTitle>
													<AlertDialogDescription>
														{t('theGeneratedProjectBindingAndItsManagedEventWillBeRemovedBoardsAlreadyUsingTheBindingWillNeedToBeUpdated', "The generated project binding and its managed event will be removed. Boards already using the binding will need to be updated.")}
													</AlertDialogDescription>
												</AlertDialogHeader>
												<AlertDialogFooter>
													<AlertDialogCancel>{t('keepAction', 'Keep action')}</AlertDialogCancel>
													<AlertDialogAction
														className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
														onClick={() => void removeAction(owner, action.id)}
													>
														{t('removeAction', 'Remove action')}
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
				<DialogContent className="max-h-[85vh] overflow-y-auto max-w-xl">
					<DialogHeader>
						<DialogTitle>
							{editingActionId
								? t('editOntologyAction', 'Edit ontology action')
								: t('defineAnOntologyAction', 'Define an ontology action')}
						</DialogTitle>
						<DialogDescription>
							{t('chooseTheObjectAndTheExactBoardEntryThatImplementsThisOperation', "Choose the object and the exact board entry that implements this operation.")}
						</DialogDescription>
					</DialogHeader>
					<div className="grid gap-4 py-2">
						<div className="grid gap-1.5">
							<Label>{t('ontology', 'Ontology')}</Label>
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
							<Label>{t('objectType', 'Object type')}</Label>
							<Select value={objectType} onValueChange={setObjectType}>
								<SelectTrigger>
									<SelectValue placeholder={t('selectObject', 'Select object')} />
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
							<Label>{t('actionName', 'Action name')}</Label>
							<Input
								value={name}
								onChange={(event) => setName(event.target.value)}
								placeholder={t('approveOrder', 'Approve order')}
							/>
						</div>
						<div className="grid gap-1.5">
							<Label>{t('description', 'Description')}</Label>
							<Textarea
								value={description}
								onChange={(event) => setDescription(event.target.value)}
								placeholder={t('whatChangesWhenThisActionSucceeds', 'What changes when this action succeeds?')}
							/>
						</div>
						<div className="grid gap-3 sm:grid-cols-2">
							<div className="grid gap-1.5">
								<Label>{t('board2', 'Board')}</Label>
								<Select
									value={boardId}
									onValueChange={(value) => {
										setBoardId(value);
										setStartNodeId("");
										setSelectedVersion(null);
									}}
								>
									<SelectTrigger>
										<SelectValue placeholder={t('selectBoard', 'Select board')} />
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
								<Label>{t('startNode', 'Start node')}</Label>
								<Select
									value={startNodeId}
									onValueChange={setStartNodeId}
									disabled={!boardId}
								>
									<SelectTrigger>
										<SelectValue placeholder={t('selectEntry', 'Select entry')} />
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
						<div className="grid gap-1.5">
							<div className="flex items-center justify-between">
								<Label>{t('boardVersion', 'Board version')}</Label>
								{boardId && appId && (
									<button
										type="button"
										className="text-xs text-muted-foreground hover:text-foreground disabled:opacity-50"
										disabled={publishingVersion}
										onClick={() => void publishDraftVersion()}
									>
										{publishingVersion ? (
											<span className="flex items-center gap-1">
												<Loader2 className="h-3 w-3 animate-spin" />
												{t('publishing', 'Publishing…')}
											</span>
										) : (
											`Publish current as new version`
										)}
									</button>
								)}
							</div>
							<Select
								value={
									selectedVersion
										? versionKey(selectedVersion)
										: CURRENT_DRAFT_VERSION
								}
								onValueChange={(value) => {
									if (value === CURRENT_DRAFT_VERSION) {
										setSelectedVersion(null);
										return;
									}
									const parts = value.split(".").map(Number);
									setSelectedVersion([
										parts[0] ?? 0,
										parts[1] ?? 0,
										parts[2] ?? 0,
									]);
								}}
								disabled={!boardId}
							>
								<SelectTrigger>
									<SelectValue placeholder={t('selectVersion', 'Select version')} />
								</SelectTrigger>
								<SelectContent>
									<SelectItem value={CURRENT_DRAFT_VERSION}>
										{board?.version
											? t('currentDraftVval', 'Current draft (v{{val}})', { val: versionKey(board.version) })
											: "Current draft"}
									</SelectItem>
									{publishedVersions.map((version) => (
										<SelectItem
											key={versionKey(version)}
											value={versionKey(version)}
										>
											v{versionKey(version)}
										</SelectItem>
									))}
								</SelectContent>
							</Select>
							<p className="text-[11px] text-muted-foreground">
								{t('pinAPublishedVersionForAReproducibleActionOrKeepTheCurrentDraftItIsPublishedAutomaticallyWhenYouSave', "Pin a published version for a reproducible action, or keep the current draft — it is published automatically when you save.")}
							</p>
						</div>
						<div className="rounded-lg border bg-muted/30 p-3 text-xs text-muted-foreground">
							<div className="flex items-center gap-1.5 font-medium text-foreground">
								<ShieldCheck className="h-3.5 w-3.5" />
								{t('pinnedImplementation', 'Pinned implementation')}
							</div>
							<p className="mt-1">
								{t('theActionResolvesThisSavedBindingServersideObjectViewsAndGeneratedProjectNodesNeverTrustAnArbitraryBoardTarget', "The action resolves this saved binding server-side; object views and generated project nodes never trust an arbitrary board target.")}
							</p>
							{inferredParameterSchema && (
								<p className="mt-2 flex items-center gap-1.5 font-medium text-foreground">
									<CheckCircle2 className="h-3.5 w-3.5 text-emerald-500" />
									{`Typed parameters detected from this entry node.`}
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
								<Label htmlFor="ontology-action-enabled">{t('enabled', 'Enabled')}</Label>
								<p className="text-xs text-muted-foreground">
									{t('visibleInObjectViews', 'Visible in object views')}
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
								<Label htmlFor="ontology-action-bulk">{t('allowBulk', 'Allow bulk')}</Label>
								<p className="text-xs text-muted-foreground">
									{t('upTo100ObjectsPerRun', 'Up to 100 objects per run')}
								</p>
							</div>
							<Switch
								id="ontology-action-bulk"
								checked={allowBulk}
								onCheckedChange={setAllowBulk}
							/>
						</div>
						<div className="flex items-center justify-between gap-3 rounded-lg border p-3">
							<div>
								<Label htmlFor="ontology-action-exposed">
									{t('exposeToConnectedProjects', 'Expose to connected projects')}
								</Label>
								<p className="text-xs text-muted-foreground">
									{`Off hides this action from connected projects; it still runs locally`}
								</p>
							</div>
							<Switch
								id="ontology-action-exposed"
								checked={actionExposed}
								onCheckedChange={setActionExposed}
							/>
						</div>
					</div>
					<DialogFooter>
						<Button
							variant="ghost"
							onClick={() => setDialogOpen(false)}
							disabled={saving}
						>
							{t('cancel', 'Cancel')}
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
							{editingActionId ? t('saveChanges2', 'Save changes') : t('saveAction', 'Save action')}
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
	const { t } = useTranslation("settings");
	return (
		<AlertDialog>
			<AlertDialogTrigger asChild>
				<Button variant="ghost" size="sm" disabled={disabled}>
					{loading && <Loader2 className="h-3.5 w-3.5 animate-spin" />}
					{t('uninstall', 'Uninstall')}
				</Button>
			</AlertDialogTrigger>
			<AlertDialogContent>
				<AlertDialogHeader>
					<AlertDialogTitle>{t('uninstallRemoteOntology', 'Uninstall remote ontology?')}</AlertDialogTitle>
					<AlertDialogDescription>{t('thisRemovesTheInstalledOntologynameContractFromSourcenameExistingBoardNodesThatUseItsGeneratedBindingsWillStopResolvingUntilTheOntologyIsInstalledAgain', "This removes the installed {{ontologyName}} contract from {{sourceName}}. Existing board nodes that use its generated bindings will stop resolving until the ontology is installed again.", { ontologyName, sourceName })}</AlertDialogDescription>
				</AlertDialogHeader>
				<AlertDialogFooter>
					<AlertDialogCancel>{t('keepInstalled', 'Keep installed')}</AlertDialogCancel>
					<AlertDialogAction
						className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
						onClick={() => void onConfirm()}
					>
						{t('uninstallBindings', 'Uninstall bindings')}
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
	const { t } = useTranslation("settings");
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
							: t('couldNotUpdateOntologySharing', 'Could not update ontology sharing.'),
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
								: t('couldNotDiscoverRemoteOntologies', 'Could not discover remote ontologies.'),
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
						: t('couldNotUpdateTheRemoteOntologyBinding', 'Could not update the remote ontology binding.'),
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
					<h3 className="font-semibold">{t('ontologyContracts', 'Ontology contracts')}</h3>
					<p className="text-sm text-muted-foreground">
						{t('exposureControlsDiscoveryExistingConnectionRolesStillGovernEveryDataReadAndAction', "Exposure controls discovery; existing connection roles still govern every data read and action.")}
					</p>
				</div>
				{ontologies.length === 0 && (
					<EmptyStudioState
						title={t('nothingToExposeYet', 'Nothing to expose yet')}
						description={`Set up a local ontology, or install a contract from a connected project.`}
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
										<p className="text-xs text-muted-foreground">{t('lengthObjectContracts', '{{length}} object contracts ·', { length: ontology.nodes.length })}{" "}
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
										{t('exposeToConnectedProjects', 'Expose to connected projects')}
									</Label>
									<p className="text-xs text-muted-foreground">
										{t('allowsPermittedProjectsToDiscoverThisContract', 'Allows permitted projects to discover this contract.')}
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
										{t('generateBoardBindings', 'Generate board bindings')}
									</Label>
									<p className="text-xs text-muted-foreground">
										{t('addsObjectAndActionBindingsToThisProjectapossNodeCatalog', "Adds object and action bindings to this project's node catalog.")}
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
							{t('connectedProjects', 'Connected projects')}
						</CardTitle>
					</CardHeader>
					<CardContent className="space-y-3">
						{connections.filter((connection) => connection.status === "ACTIVE")
							.length === 0 ? (
							<div className="rounded-lg border border-dashed p-5 text-center text-sm text-muted-foreground">
								{`No active app connections. Create one from Team → Connections.`}
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
							<p className="font-medium text-foreground">{t('defenseInDepth', 'Defense in depth')}</p>
							<p className="mt-1">
								{t('readdatabaseControlsObjectAccessAnyEventExecutionRemainsSeparatelyPermissionedExposureNeverWidensTheAssignedRole', "ReadDatabase controls object access. Any event execution remains separately permissioned; exposure never widens the assigned role.")}
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
							{t('loadingInstalledOntologyBindings', 'Loading installed ontology bindings…')}
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
								{t('couldNotLoadInstalledOntologyBindings', 'Could not load installed ontology bindings:')}{" "}
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
								{t('installedBindings', 'Installed bindings')}
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
											<p className="truncate text-xs text-muted-foreground">{t('remoteSourcename', 'Remote · {{sourceName}} ·', { sourceName })}{" "}{t('lengthObjects2', '{{length}} objects', { length: installed.contract.nodes.length })}</p>
										</div>
										<Badge variant="secondary">{t('installed', 'Installed')}</Badge>
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
							{t('availableRemoteOntologies', 'Available remote ontologies')}
						</CardTitle>
					</CardHeader>
					<CardContent className="space-y-3">
						{remoteConnections.filter(
							(connection) => connection.status === "ACTIVE",
						).length === 0 ? (
							<p className="text-sm text-muted-foreground">
								{t('noOutgoingProjectConnectionsCanExposeContractsYet', 'No outgoing project connections can expose contracts yet.')}
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
														{t('onlyExplicitlyExposedContractsAreReturned', 'Only explicitly exposed contracts are returned.')}
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
															{t('noContractsAreExposedByThisProject', 'No contracts are exposed by this project.')}
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
																			<p className="text-[10px] text-muted-foreground">{t('lengthObjectTypesBindingsOnly', "{{length}} object types · bindings only", { length: contract.nodes.length })}</p>
																		</div>
																		<Badge
																			variant={
																				installed ? "secondary" : "outline"
																			}
																		>
																			{installedOntologiesLoading
																				? "Checking"
																				: installedOntologiesError
																					? "Unavailable"
																					: updateAvailable
																						? t('updateAvailable', 'Update available')
																						: installed
																							? "Installed"
																							: "Remote"}
																		</Badge>
																	</div>
																	<div className="flex flex-wrap items-center justify-end gap-2">
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
																			{installed ? "Refresh" : "Install"}
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
