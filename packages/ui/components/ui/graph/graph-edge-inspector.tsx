"use client";

import { useTranslation } from "@flow-like/locales";
import { ArrowDown, ExternalLink, X } from "lucide-react";
import { useCallback, useMemo, useState } from "react";
import {
	type EditableRelationship,
	type ObjectEditField,
	lockedRelationshipColumns,
	resolveRelationshipIdentity,
} from "../../../lib/ontology-object-edit";
import type {
	GraphOverlay,
	SubgraphEdge,
	SubgraphNode,
} from "../../../state/backend-state/graph-state";
import { Button } from "../button";
import { ScrollArea } from "../scroll-area";
import { UserInlineTag } from "../user-identity";
import {
	CopyButton,
	FieldFilter,
	PropertyRow,
	type UpdateElementProperties,
	declaredTypes,
	usePropertyRowEdits,
} from "./graph-node-inspector";
import { getGraphIcon } from "./icons";

export interface GraphEdgeInspectorProps {
	edge: SubgraphEdge | null;
	overlay?: GraphOverlay;
	sourceCaption?: string;
	targetCaption?: string;
	sourceAccountId?: string | null;
	targetAccountId?: string | null;
	/** The loaded endpoints; they carry the typed ids a relationship edit keys on. */
	sourceNode?: SubgraphNode;
	targetNode?: SubgraphNode;
	onClose: () => void;
	/** Column types of the relationship's backing table; undefined while they load. */
	editFields?: ReadonlyMap<string, ObjectEditField>;
	/**
	 * Saves edited property values of a join-table relationship. Omitted, the
	 * inspector is read-only.
	 */
	onUpdateProperties?: UpdateElementProperties;
	/** Selects a loaded node, used to reach the object a foreign key lives on. */
	onOpenNode?: (nodeId: string) => void;
}

type ForeignKeyRelationship = Extract<
	EditableRelationship,
	{ reason: "foreignKey" }
>;

function ForeignKeyNotice({
	relationship,
	edge,
	ownerNode,
	ownerAccountId,
	onOpenNode,
}: {
	relationship: ForeignKeyRelationship;
	edge: SubgraphEdge;
	ownerNode?: SubgraphNode;
	ownerAccountId?: string | null;
	onOpenNode?: (nodeId: string) => void;
}) {
	const { t } = useTranslation("common");
	const name =
		(!ownerAccountId && ownerNode?.caption) || relationship.ownerLabel;
	const ownerId =
		relationship.ownerSide === "source" ? edge.source : edge.target;
	return (
		<div className="min-w-0 space-y-2 rounded-lg border border-dashed border-border/70 px-3 py-2.5">
			<p className="text-xs leading-relaxed text-muted-foreground [overflow-wrap:anywhere]">
				{t(
					"relationshipStoredOnObject",
					"These values are stored on {{object}}. Open it to edit them.",
					{ object: name },
				)}
			</p>
			{onOpenNode && ownerNode && (
				<Button
					variant="outline"
					size="sm"
					className="h-auto min-h-7 max-w-full justify-start gap-1.5 whitespace-normal px-2.5 py-1 text-left text-xs [overflow-wrap:anywhere]"
					onClick={() => onOpenNode(ownerId)}
				>
					<ExternalLink className="h-3.5 w-3.5 shrink-0" />
					{t("openObjectName", "Open {{name}}", { name })}
				</Button>
			)}
		</div>
	);
}

export function GraphEdgeInspector({
	edge,
	overlay,
	sourceCaption,
	targetCaption,
	sourceAccountId,
	targetAccountId,
	sourceNode,
	targetNode,
	onClose,
	editFields,
	onUpdateProperties,
	onOpenNode,
}: GraphEdgeInspectorProps) {
	const { t } = useTranslation("common");
	const relationship = useMemo<EditableRelationship | null>(() => {
		if (!onUpdateProperties || !edge) return null;
		if (!overlay) return { ok: false, reason: "unknownType" };
		return resolveRelationshipIdentity(overlay, edge, sourceNode, targetNode);
	}, [onUpdateProperties, edge, overlay, sourceNode, targetNode]);
	const locked = useMemo(
		() =>
			overlay && relationship?.ok
				? lockedRelationshipColumns(overlay, relationship.mapping)
				: null,
		[overlay, relationship],
	);
	const { editingKey, rowEdit, guardToggle } = usePropertyRowEdits({
		elementId: edge?.id,
		props: edge?.props,
		fields: editFields,
		locked,
		onUpdate: onUpdateProperties,
	});
	const [hiddenFields, setHiddenFields] = useState<Set<string>>(new Set());
	const handleToggleField = useCallback((field: string) => {
		setHiddenFields((prev) => {
			const next = new Set(prev);
			if (next.has(field)) next.delete(field);
			else next.add(field);
			return next;
		});
	}, []);
	const typeNames = useMemo(
		() =>
			declaredTypes(
				overlay?.edges.find((candidate) => candidate.label === edge?.label)
					?.property_columns,
			),
		[overlay, edge?.label],
	);

	if (!edge) return null;

	const Icon = getGraphIcon(edge.style?.icon ?? "link");
	const propEntries = edge.props
		? Object.entries(edge.props).filter(
				([k, v]) => (v !== null && v !== undefined) || k === editingKey,
			)
		: [];
	const allFields = propEntries.map(([k]) => k);
	const visibleEntries = propEntries.filter(([k]) => !hiddenFields.has(k));
	const foreignKey =
		relationship && !relationship.ok && relationship.reason === "foreignKey"
			? relationship
			: null;

	return (
		<div className="flex h-full min-h-0 w-80 min-w-0 max-w-full shrink-0 flex-col overflow-hidden border-l bg-background animate-in slide-in-from-right-5 duration-200">
			<div className="flex shrink-0 items-start justify-between gap-3 border-b bg-muted/20 p-4">
				<div className="flex min-w-0 flex-1 items-start gap-3">
					<div
						className="flex h-9 w-9 shrink-0 items-center justify-center rounded-xl shadow-sm"
						style={{ backgroundColor: edge.style?.color ?? "#94a3b8" }}
					>
						<Icon className="h-4 w-4 text-white" />
					</div>
					<div className="min-w-0">
						<h3 className="text-sm font-semibold leading-snug [overflow-wrap:anywhere]">
							{edge.label}
						</h3>
						<p className="mt-1 text-xs text-muted-foreground">
							{t("edge", "Edge")}
						</p>
					</div>
				</div>
				<div className="flex items-center gap-1 shrink-0">
					{allFields.length > 0 && (
						<FieldFilter
							allFields={allFields}
							hiddenFields={hiddenFields}
							onToggle={guardToggle(handleToggleField)}
						/>
					)}
					<Button
						variant="ghost"
						size="icon"
						className="h-8 w-8"
						aria-label={t("close", "Close")}
						onClick={onClose}
					>
						<X className="h-4 w-4" />
					</Button>
				</div>
			</div>
			<ScrollArea
				className="min-h-0 min-w-0 flex-1"
				viewportClassName="[&>div]:!block [&>div]:w-full [&>div]:min-w-0"
			>
				<div className="w-full min-w-0 space-y-5 p-4">
					{/* Source → Target */}
					<div className="min-w-0 space-y-2 rounded-lg border border-border/60 bg-muted/20 px-3 py-3">
						<div className="min-w-0 space-y-1">
							<p className="text-[10px] font-medium uppercase tracking-wider text-muted-foreground">
								{t("source", "Source")}
							</p>
							<div className="text-sm font-medium leading-relaxed [overflow-wrap:anywhere]">
								{sourceAccountId ? (
									<UserInlineTag userId={sourceAccountId} className="text-sm" />
								) : (
									(sourceCaption ?? edge.source)
								)}
							</div>
							<p className="text-[10px] font-mono text-muted-foreground [overflow-wrap:anywhere]">
								{edge.source}
							</p>
						</div>
						<ArrowDown className="h-4 w-4 text-muted-foreground" />
						<div className="min-w-0 space-y-1">
							<p className="text-[10px] font-medium uppercase tracking-wider text-muted-foreground">
								{t("target", "Target")}
							</p>
							<div className="text-sm font-medium leading-relaxed [overflow-wrap:anywhere]">
								{targetAccountId ? (
									<UserInlineTag userId={targetAccountId} className="text-sm" />
								) : (
									(targetCaption ?? edge.target)
								)}
							</div>
							<p className="text-[10px] font-mono text-muted-foreground [overflow-wrap:anywhere]">
								{edge.target}
							</p>
						</div>
					</div>

					<div>
						<p className="text-[10px] font-medium uppercase tracking-wider text-muted-foreground mb-1">
							ID
						</p>
						<div className="group flex min-w-0 items-start justify-between gap-2">
							<p className="min-w-0 text-xs font-mono leading-relaxed text-muted-foreground [overflow-wrap:anywhere]">
								{edge.id}
							</p>
							<CopyButton text={edge.id} />
						</div>
					</div>

					{foreignKey && (
						<ForeignKeyNotice
							relationship={foreignKey}
							edge={edge}
							ownerNode={
								foreignKey.ownerSide === "source" ? sourceNode : targetNode
							}
							ownerAccountId={
								foreignKey.ownerSide === "source"
									? sourceAccountId
									: targetAccountId
							}
							onOpenNode={onOpenNode}
						/>
					)}

					{visibleEntries.length > 0 && (
						<div>
							<div className="flex items-center justify-between mb-2">
								<p className="text-[10px] font-medium uppercase tracking-wider text-muted-foreground">
									{t("properties", "Properties")}
								</p>
								{hiddenFields.size > 0 && (
									<span className="text-[10px] text-muted-foreground">
										{t("sizeHidden", "{{size}} hidden", {
											size: hiddenFields.size,
										})}
									</span>
								)}
							</div>
							<div className="space-y-2">
								{visibleEntries.map(([key, value]) => (
									<PropertyRow
										key={`${edge.id}:${key}`}
										value={value}
										propKey={key}
										metadata={edge.property_metadata?.[key]}
										typeName={typeNames.get(key)}
										edit={rowEdit(key, value)}
									/>
								))}
							</div>
						</div>
					)}
					{propEntries.length === 0 && (
						<p className="text-xs text-muted-foreground italic">
							{t("noPropertiesAvailable", "No properties available")}
						</p>
					)}
					{propEntries.length > 0 && visibleEntries.length === 0 && (
						<p className="text-xs text-muted-foreground italic">
							{t(
								"allFieldsHiddenUseTheFilterToShowThem",
								"All fields hidden. Use the filter to show them.",
							)}
						</p>
					)}
				</div>
			</ScrollArea>
		</div>
	);
}
