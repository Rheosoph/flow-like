"use client";

import { useTranslation } from "@flow-like/locales";
import { Plus, Trash2 } from "lucide-react";
import type {
	EdgeLabelMapping,
	LabelStyle,
	NodeLabelMapping,
	PropertyColumn,
} from "../../../../state/backend-state/graph-state";
import { Button } from "../../button";
import { Card } from "../../card";
import { Input } from "../../input";
import { Label } from "../../label";
import {
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "../../select";
import { Separator } from "../../separator";
import { ColumnPicker } from "./column-picker";
import { StyleEditor } from "./style-editor";

export interface StepEdgesProps {
	edges: EdgeLabelMapping[];
	nodes: NodeLabelMapping[];
	tables: string[];
	tableColumns: Record<string, PropertyColumn[]>;
	onChange: (edges: EdgeLabelMapping[]) => void;
}

const DEFAULT_EDGE_STYLE: LabelStyle = {
	color: "#94a3b8",
	icon: "database",
	size: { mode: "fixed", value: 2 },
	width: 2,
};

export function StepEdges({
	edges,
	nodes,
	tables,
	tableColumns,
	onChange,
}: StepEdgesProps) {
	const { t } = useTranslation("common");
	const nodeLabels = nodes.map((n) => n.label).filter(Boolean);

	const getNodeColumns = (label: string): string[] => {
		const node = nodes.find((n) => n.label === label);
		if (!node) return [];
		return (tableColumns[node.table] ?? []).map((c) => c.name);
	};

	const addEdge = () => {
		onChange([
			...edges,
			{
				label: "",
				table: tables[0] ?? "",
				src_column: "",
				dst_column: "",
				src_label: nodeLabels[0] ?? "",
				dst_label: nodeLabels[0] ?? "",
				property_columns: [],
				style: { ...DEFAULT_EDGE_STYLE },
			},
		]);
	};

	const updateEdge = (index: number, partial: Partial<EdgeLabelMapping>) => {
		const updated = edges.map((e, i) =>
			i === index ? { ...e, ...partial } : e,
		);
		onChange(updated);
	};

	const removeEdge = (index: number) => {
		onChange(edges.filter((_, i) => i !== index));
	};

	const togglePropertyColumn = (index: number, col: PropertyColumn) => {
		const edge = edges[index];
		if (!edge) return;
		const exists = edge.property_columns.some((c) => c.name === col.name);
		const cols = exists
			? edge.property_columns.filter((c) => c.name !== col.name)
			: [...edge.property_columns, col];
		updateEdge(index, { property_columns: cols });
	};

	return (
		<div className="space-y-4">
			<div className="flex items-center justify-between">
				<div>
					<h3 className="text-sm font-medium mb-1">
						{t("edgeMappings", "Edge Mappings")}
					</h3>
					<p className="text-xs text-muted-foreground">
						{t(
							"eachMappingTurnsATableIntoAGraphEdgeLabelConnectingTwoNodeLabels",
							"Each mapping turns a table into a graph edge label connecting two node labels.",
						)}
					</p>
				</div>
				<Button size="sm" variant="outline" onClick={addEdge}>
					<Plus className="h-3.5 w-3.5 mr-1" />
					{t("addEdge", "Add Edge")}
				</Button>
			</div>

			<div className="max-h-112.5 overflow-y-auto">
				<div className="space-y-4 pr-2">
					{edges.map((edge, i) => {
						const cols = tableColumns[edge.table] ?? [];
						const colNames = cols.map((c) => c.name);
						return (
							<Card key={i} className="p-4 space-y-3">
								<div className="flex items-center justify-between">
									<span className="text-xs font-medium text-muted-foreground">
										{t("edge2", "Edge #")}
										{i + 1}
									</span>
									<Button
										variant="ghost"
										size="icon"
										className="h-7 w-7 text-destructive"
										onClick={() => removeEdge(i)}
									>
										<Trash2 className="h-3.5 w-3.5" />
									</Button>
								</div>

								<div className="grid grid-cols-2 gap-3">
									<div className="space-y-1.5">
										<Label className="text-xs">{t("label", "Label")}</Label>
										<Input
											value={edge.label}
											onChange={(e) => updateEdge(i, { label: e.target.value })}
											className="h-8 text-xs"
											placeholder="KNOWS"
										/>
									</div>
									<div className="space-y-1.5">
										<Label className="text-xs">{t("table", "Table")}</Label>
										<Select
											value={edge.table}
											onValueChange={(v) =>
												updateEdge(i, {
													table: v,
													src_column: "",
													dst_column: "",
													property_columns: [],
												})
											}
										>
											<SelectTrigger className="h-8 text-xs">
												<SelectValue
													placeholder={t("selectTable", "Select table")}
												/>
											</SelectTrigger>
											<SelectContent>
												{tables.map((t) => (
													<SelectItem key={t} value={t} className="text-xs">
														{t}
													</SelectItem>
												))}
											</SelectContent>
										</Select>
									</div>
									<div className="space-y-1.5">
										<Label className="text-xs">
											{t("sourceColumn", "Source Column")}
										</Label>
										<ColumnPicker
											columns={colNames}
											value={edge.src_column}
											onChange={(v) => updateEdge(i, { src_column: v })}
											placeholder={t("sourceFk", "Source FK")}
										/>
									</div>
									<div className="space-y-1.5">
										<Label className="text-xs">
											{t("targetColumn", "Target Column")}
										</Label>
										<ColumnPicker
											columns={colNames}
											value={edge.dst_column}
											onChange={(v) => updateEdge(i, { dst_column: v })}
											placeholder={t("targetFk", "Target FK")}
										/>
									</div>
									<div className="space-y-1.5">
										<Label className="text-xs">
											{t("sourceLabel", "Source Label")}
										</Label>
										<Select
											value={edge.src_label}
											onValueChange={(v) =>
												updateEdge(i, {
													src_label: v,
													src_node_column: undefined,
												})
											}
										>
											<SelectTrigger className="h-8 text-xs">
												<SelectValue
													placeholder={t(
														"sourceNodeLabel",
														"Source node label",
													)}
												/>
											</SelectTrigger>
											<SelectContent>
												{nodeLabels.map((l) => (
													<SelectItem key={l} value={l} className="text-xs">
														{l}
													</SelectItem>
												))}
											</SelectContent>
										</Select>
									</div>
									<div className="space-y-1.5">
										<Label className="text-xs">
											{t("targetLabel", "Target Label")}
										</Label>
										<Select
											value={edge.dst_label}
											onValueChange={(v) =>
												updateEdge(i, {
													dst_label: v,
													dst_node_column: undefined,
												})
											}
										>
											<SelectTrigger className="h-8 text-xs">
												<SelectValue
													placeholder={t(
														"targetNodeLabel",
														"Target node label",
													)}
												/>
											</SelectTrigger>
											<SelectContent>
												{nodeLabels.map((l) => (
													<SelectItem key={l} value={l} className="text-xs">
														{l}
													</SelectItem>
												))}
											</SelectContent>
										</Select>
									</div>
									{edge.src_label &&
										getNodeColumns(edge.src_label).length > 0 && (
											<div className="space-y-1.5">
												<Label className="text-xs">
													{t("sourceJoinColumn", "Source Join Column")}
												</Label>
												<ColumnPicker
													columns={getNodeColumns(edge.src_label)}
													value={edge.src_node_column ?? ""}
													onChange={(v) =>
														updateEdge(i, { src_node_column: v || undefined })
													}
													placeholder={
														nodes.find((n) => n.label === edge.src_label)
															?.id_column || t("nodeIdColumn", "Node ID column")
													}
												/>
											</div>
										)}
									{edge.dst_label &&
										getNodeColumns(edge.dst_label).length > 0 && (
											<div className="space-y-1.5">
												<Label className="text-xs">
													{t("targetJoinColumn", "Target Join Column")}
												</Label>
												<ColumnPicker
													columns={getNodeColumns(edge.dst_label)}
													value={edge.dst_node_column ?? ""}
													onChange={(v) =>
														updateEdge(i, { dst_node_column: v || undefined })
													}
													placeholder={
														nodes.find((n) => n.label === edge.dst_label)
															?.id_column || t("nodeIdColumn", "Node ID column")
													}
												/>
											</div>
										)}
								</div>

								{cols.length > 0 && (
									<div className="space-y-1.5">
										<Label className="text-xs">
											{t("propertyColumns", "Property Columns")}
										</Label>
										<p className="text-[10px] text-muted-foreground">
											{t(
												"leaveEmptyToIncludeAllNonvectorColumns",
												"Leave empty to include all non-vector columns.",
											)}
										</p>
										<div className="flex flex-wrap gap-1.5">
											{cols.map((col) => (
												<button
													key={col.name}
													type="button"
													onClick={() => togglePropertyColumn(i, col)}
													className={`text-[10px] px-2 py-0.5 rounded-full border transition-colors ${
														edge.property_columns.some(
															(c) => c.name === col.name,
														)
															? "bg-primary text-primary-foreground border-primary"
															: "bg-muted hover:bg-accent"
													}`}
													title={`${col.data_type}${col.nullable ? " (nullable)" : ""}`}
												>
													{col.name}
												</button>
											))}
										</div>
									</div>
								)}

								<Separator />
								<StyleEditor
									style={edge.style}
									onChange={(s) => updateEdge(i, { style: s })}
								/>
							</Card>
						);
					})}
					{edges.length === 0 && (
						<p className="text-sm text-muted-foreground text-center py-8">
							{t(
								"noEdgeMappingsYetClickQuotaddEdgequotToConnectNodeLabels",
								'No edge mappings yet. Click "Add Edge" to connect node labels.',
							)}
						</p>
					)}
				</div>
			</div>
		</div>
	);
}
