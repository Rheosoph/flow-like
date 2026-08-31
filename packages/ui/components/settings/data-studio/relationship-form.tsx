"use client";

import { useTranslation } from "@flow-like/locales";
import { createId } from "@paralleldrive/cuid2";
import { Plus, X } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import type {
	EdgeLabelMapping,
	NodeLabelMapping,
	PropertyColumn,
} from "../../../state/backend-state/graph-state";
import { Button } from "../../ui/button";
import { Input } from "../../ui/input";
import { Label } from "../../ui/label";
import {
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "../../ui/select";
import { Switch } from "../../ui/switch";

/**
 * The object shape a relationship needs at both ends. The setup wizard holds
 * freshly inspected schemas; the ontology editor reads them back off saved
 * nodes — this is the common denominator.
 */
export interface RelationshipEndpoint {
	id: string;
	label: string;
	api_name?: string;
	table: string;
	id_column: string;
	columns: PropertyColumn[];
	color: string;
}

/**
 * An edge plus surface-local bookkeeping. `origin_key` gives a relationship a
 * stable identity while the objects around it are edited, and the endpoint ids
 * let a hand-authored edge follow an object that gets renamed. All three are
 * stripped by `toEdgeMapping` before the payload is sent.
 */
export interface WizardEdge extends EdgeLabelMapping {
	origin_key: string;
	manual?: boolean;
	src_object_id?: string;
	dst_object_id?: string;
}

/** Mirrors `is_valid_graph_identifier` in the storage crate: labels become Cypher identifiers. */
const GRAPH_IDENTIFIER = /^[A-Za-z_][A-Za-z0-9_]*$/;

export function isValidGraphIdentifier(value: string): boolean {
	return GRAPH_IDENTIFIER.test(value);
}

const UNCOUNTABLE = new Set([
	"series",
	"species",
	"news",
	"data",
	"media",
	"metadata",
	"info",
]);

function singularizeWord(word: string): string {
	if (UNCOUNTABLE.has(word)) return word;
	if (word.endsWith("ies") && word.length > 4) return `${word.slice(0, -3)}y`;
	if (word.endsWith("yses")) return `${word.slice(0, -3)}sis`;
	if (
		word.endsWith("sses") ||
		word.endsWith("xes") ||
		word.endsWith("zes") ||
		word.endsWith("ches") ||
		word.endsWith("shes") ||
		word.endsWith("uses")
	) {
		return word.slice(0, -2);
	}
	if (
		word.endsWith("s") &&
		!word.endsWith("ss") &&
		!word.endsWith("us") &&
		!word.endsWith("is")
	) {
		return word.slice(0, -1);
	}
	return word;
}

function identifierParts(value: string): string[] {
	return value
		.trim()
		.replace(/([a-z0-9])([A-Z])/g, "$1_$2")
		.replace(/[^a-zA-Z0-9]+/g, "_")
		.toLowerCase()
		.split("_")
		.filter(Boolean);
}

/** snake_case identifier with only the trailing word singularized. */
export function apiName(value: string): string {
	const parts = identifierParts(value);
	if (parts.length === 0) return "";
	parts[parts.length - 1] = singularizeWord(parts[parts.length - 1]);
	return parts.join("_");
}

/** PascalCase display label — labels are query identifiers, so spaces are not allowed. */
export function displayName(value: string): string {
	const pascal = apiName(value)
		.split("_")
		.filter(Boolean)
		.map((part) => `${part.charAt(0).toUpperCase()}${part.slice(1)}`)
		.join("");
	if (!pascal) return "";
	return isValidGraphIdentifier(pascal) ? pascal : `_${pascal}`;
}

const FK_SUFFIXES = ["_id", "_uuid", "_key", "_fk", "_ref"] as const;

/** The object a foreign-key-shaped column points at, normalized like object API names. */
export function foreignKeyStem(columnName: string): string | undefined {
	const normalized = apiName(columnName);
	for (const suffix of FK_SUFFIXES) {
		if (normalized.endsWith(suffix) && normalized.length > suffix.length) {
			return apiName(normalized.slice(0, -suffix.length));
		}
	}
	if (normalized.startsWith("fk_") && normalized.length > 3) {
		return apiName(normalized.slice(3));
	}
	return undefined;
}

export function endpointMatchesStem(
	endpoint: RelationshipEndpoint,
	stem: string,
): boolean {
	if (!stem) return false;
	if (apiName(endpoint.api_name ?? "") === stem) return true;
	if (apiName(endpoint.table) === stem) return true;
	if (apiName(endpoint.label) === stem) return true;
	const idStem = /_id$/i.test(endpoint.id_column)
		? apiName(endpoint.id_column.replace(/_id$/i, ""))
		: "";
	return Boolean(idStem) && idStem === stem;
}

export function uniqueLabel(base: string, taken: Set<string>): string {
	const seed = isValidGraphIdentifier(base) ? base : `link_${base}`;
	let candidate = seed;
	let suffix = 2;
	while (taken.has(candidate.toLowerCase())) {
		candidate = `${seed}_${suffix}`;
		suffix += 1;
	}
	taken.add(candidate.toLowerCase());
	return candidate;
}

export function buildEdge(params: {
	originKey: string;
	manual: boolean;
	source: RelationshipEndpoint;
	target: RelationshipEndpoint;
	table: string;
	srcColumn: string;
	dstColumn: string;
	label: string;
	containment?: boolean;
}): WizardEdge {
	const sourceApi = apiName(params.source.api_name ?? params.source.label);
	const targetApi = apiName(params.target.api_name ?? params.target.label);
	return {
		id: createId(),
		origin_key: params.originKey,
		manual: params.manual,
		src_object_id: params.source.id,
		dst_object_id: params.target.id,
		api_name: `${sourceApi}_to_${targetApi}`,
		label: params.label,
		table: params.table,
		src_column: params.srcColumn,
		dst_column: params.dstColumn,
		src_label: params.source.label,
		dst_label: params.target.label,
		containment: params.containment ?? false,
		property_columns: [],
		style: {
			color: params.source.color,
			icon: "arrow-right",
			size: { mode: "fixed", value: 2 },
		},
	};
}

export function toEdgeMapping(edge: WizardEdge): EdgeLabelMapping {
	const {
		origin_key: _originKey,
		manual: _manual,
		src_object_id: _srcObjectId,
		dst_object_id: _dstObjectId,
		...mapping
	} = edge;
	return mapping;
}

/** Reverses an edge — the join columns have to swap with the endpoints. */
export function reversedEdge<T extends EdgeLabelMapping>(edge: T): T {
	return {
		...edge,
		src_label: edge.dst_label,
		dst_label: edge.src_label,
		src_column: edge.dst_column,
		dst_column: edge.src_column,
	};
}

/** Saved ontology nodes carry every column in `property_columns`. */
export function nodeToEndpoint(node: NodeLabelMapping): RelationshipEndpoint {
	return {
		id: node.id ?? node.api_name ?? node.label,
		label: node.label,
		api_name: node.api_name,
		table: node.table,
		id_column: node.id_column,
		columns: node.property_columns,
		color: node.style.color,
	};
}

function suggestForeignKeyColumn(
	columns: PropertyColumn[],
	target: RelationshipEndpoint | undefined,
	exclude: string,
): string {
	const candidates = columns.filter((column) => column.name !== exclude);
	if (target) {
		const match = candidates.find((column) => {
			const stem = foreignKeyStem(column.name);
			return stem !== undefined && endpointMatchesStem(target, stem);
		});
		if (match) return match.name;
	}
	return (
		candidates.find((column) => foreignKeyStem(column.name) !== undefined)
			?.name ??
		candidates[0]?.name ??
		""
	);
}

export interface AddRelationshipFormProps {
	endpoints: RelationshipEndpoint[];
	takenLabels: Set<string>;
	prefill?: { sourceId: string; dstColumn: string } | null;
	onAdd: (edge: WizardEdge) => void;
	onCancel: () => void;
}

export function AddRelationshipForm({
	endpoints,
	takenLabels,
	prefill,
	onAdd,
	onCancel,
}: Readonly<AddRelationshipFormProps>) {
	const { t } = useTranslation("settings");
	const prefilledSource = prefill
		? endpoints.find((endpoint) => endpoint.id === prefill.sourceId)
		: undefined;
	const initialSourceId = prefilledSource?.id ?? endpoints[0]?.id ?? "";
	const [sourceId, setSourceId] = useState(initialSourceId);
	const [targetId, setTargetId] = useState(
		endpoints.find((endpoint) => endpoint.id !== initialSourceId)?.id ??
			initialSourceId,
	);
	const [table, setTable] = useState(
		prefilledSource?.table ?? endpoints[0]?.table ?? "",
	);
	const [srcColumn, setSrcColumn] = useState(
		prefilledSource?.id_column ?? endpoints[0]?.id_column ?? "",
	);
	const [dstColumn, setDstColumn] = useState(prefill?.dstColumn ?? "");
	const [label, setLabel] = useState("");
	const [labelTouched, setLabelTouched] = useState(false);
	const [containment, setContainment] = useState(false);

	const source = endpoints.find((endpoint) => endpoint.id === sourceId);
	const target = endpoints.find((endpoint) => endpoint.id === targetId);

	// A link table has to be one of the mapped objects — that keeps its column
	// list available without a second schema fetch.
	const tableOptions = useMemo(() => {
		const seen = new Set<string>();
		return endpoints.filter((endpoint) => {
			if (seen.has(endpoint.table)) return false;
			seen.add(endpoint.table);
			return true;
		});
	}, [endpoints]);

	const joinColumns = useMemo(
		() => endpoints.find((endpoint) => endpoint.table === table)?.columns ?? [],
		[endpoints, table],
	);

	useEffect(() => {
		if (!source) return;
		setTable(source.table);
	}, [source]);

	useEffect(() => {
		const columns =
			endpoints.find((endpoint) => endpoint.table === table)?.columns ?? [];
		if (columns.length === 0) return;
		const nextSrc =
			source && table === source.table
				? source.id_column
				: suggestForeignKeyColumn(columns, source, "");
		setSrcColumn(nextSrc);
		setDstColumn((current) =>
			current && columns.some((column) => column.name === current)
				? current
				: suggestForeignKeyColumn(columns, target, nextSrc),
		);
	}, [endpoints, source, table, target]);

	useEffect(() => {
		if (labelTouched || !source || !target) return;
		const base = `${apiName(source.api_name ?? source.label)}_has_${apiName(
			target.api_name ?? target.label,
		)}`;
		setLabel(uniqueLabel(base, new Set(takenLabels)));
	}, [labelTouched, source, takenLabels, target]);

	const trimmedLabel = label.trim();
	const labelInvalid = !isValidGraphIdentifier(trimmedLabel);
	const labelTaken = takenLabels.has(trimmedLabel.toLowerCase());
	const columnsValid =
		joinColumns.some((column) => column.name === srcColumn) &&
		joinColumns.some((column) => column.name === dstColumn);
	const canAdd =
		Boolean(source) &&
		Boolean(target) &&
		columnsValid &&
		!labelInvalid &&
		!labelTaken;

	const handleAdd = useCallback(() => {
		if (!source || !target) return;
		onAdd(
			buildEdge({
				originKey: `manual:${createId()}`,
				manual: true,
				source,
				target,
				table,
				srcColumn,
				dstColumn,
				label: trimmedLabel,
				containment,
			}),
		);
	}, [
		containment,
		dstColumn,
		onAdd,
		source,
		srcColumn,
		table,
		target,
		trimmedLabel,
	]);

	return (
		<div className="space-y-3 rounded-xl border border-primary/40 bg-primary/5 p-4">
			<div className="flex items-center justify-between">
				<p className="text-sm font-medium">
					{t("newRelationship", "New relationship")}
				</p>
				<Button
					variant="ghost"
					size="icon"
					onClick={onCancel}
					title={t("cancel", "Cancel")}
				>
					<X className="h-4 w-4" />
				</Button>
			</div>
			<div className="grid gap-3 sm:grid-cols-2">
				<div className="space-y-1.5">
					<Label>{t("fromObject", "From object")}</Label>
					<Select value={sourceId} onValueChange={setSourceId}>
						<SelectTrigger>
							<SelectValue />
						</SelectTrigger>
						<SelectContent>
							{endpoints.map((endpoint) => (
								<SelectItem key={endpoint.id} value={endpoint.id}>
									{endpoint.label}
								</SelectItem>
							))}
						</SelectContent>
					</Select>
				</div>
				<div className="space-y-1.5">
					<Label>{t("toObject", "To object")}</Label>
					<Select value={targetId} onValueChange={setTargetId}>
						<SelectTrigger>
							<SelectValue />
						</SelectTrigger>
						<SelectContent>
							{endpoints.map((endpoint) => (
								<SelectItem key={endpoint.id} value={endpoint.id}>
									{endpoint.label}
								</SelectItem>
							))}
						</SelectContent>
					</Select>
				</div>
				<div className="space-y-1.5">
					<Label>{t("joinTable", "Join table")}</Label>
					<Select value={table} onValueChange={setTable}>
						<SelectTrigger>
							<SelectValue />
						</SelectTrigger>
						<SelectContent>
							{tableOptions.map((endpoint) => (
								<SelectItem key={endpoint.table} value={endpoint.table}>
									{endpoint.table}
								</SelectItem>
							))}
						</SelectContent>
					</Select>
					<p className="text-[10px] text-muted-foreground">
						{t(
							"bothJoinColumnsLiveInThisTablePickALinkTableForManytomany",
							"Both join columns live in this table. Pick a link table for many-to-many.",
						)}
					</p>
				</div>
				<div className="space-y-1.5">
					<Label>{t("relationshipLabel", "Relationship label")}</Label>
					<Input
						value={label}
						onChange={(event) => {
							setLabelTouched(true);
							setLabel(event.target.value);
						}}
						aria-invalid={labelInvalid || labelTaken}
						className={`font-mono text-xs${
							labelInvalid || labelTaken ? " border-destructive" : ""
						}`}
					/>
					{labelTaken && (
						<p className="text-xs text-destructive">
							{t(
								"thisLabelIsAlreadyUsedByAnotherObjectOrRelationship",
								"This label is already used by another object or relationship.",
							)}
						</p>
					)}
					{!labelTaken && labelInvalid && (
						<p className="text-xs text-destructive">
							{t(
								"useLettersDigitsAndUnderscoresStartingWithALetter",
								"Use letters, digits and underscores, starting with a letter.",
							)}
						</p>
					)}
				</div>
				<div className="space-y-1.5">
					<Label>{t("sourceColumn", "Source column")}</Label>
					<Select value={srcColumn} onValueChange={setSrcColumn}>
						<SelectTrigger>
							<SelectValue />
						</SelectTrigger>
						<SelectContent>
							{joinColumns.map((column) => (
								<SelectItem key={column.name} value={column.name}>
									{column.name}
								</SelectItem>
							))}
						</SelectContent>
					</Select>
				</div>
				<div className="space-y-1.5">
					<Label>{t("targetColumn", "Target column")}</Label>
					<Select value={dstColumn} onValueChange={setDstColumn}>
						<SelectTrigger>
							<SelectValue />
						</SelectTrigger>
						<SelectContent>
							{joinColumns.map((column) => (
								<SelectItem key={column.name} value={column.name}>
									{column.name}
								</SelectItem>
							))}
						</SelectContent>
					</Select>
				</div>
			</div>
			<div className="flex flex-wrap items-center justify-between gap-3">
				<div className="flex items-center gap-2">
					<Switch
						id="new-edge-containment"
						checked={containment}
						onCheckedChange={setContainment}
					/>
					<Label htmlFor="new-edge-containment" className="text-xs font-medium">
						{t("hierarchyParentChild", "Hierarchy (parent → child)")}
					</Label>
				</div>
				<div className="flex items-center gap-2">
					<Button variant="ghost" size="sm" onClick={onCancel}>
						{t("cancel", "Cancel")}
					</Button>
					<Button size="sm" onClick={handleAdd} disabled={!canAdd}>
						<Plus className="h-4 w-4" />
						{t("addRelationship", "Add relationship")}
					</Button>
				</div>
			</div>
		</div>
	);
}
