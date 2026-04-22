"use client";

import { Plus, Trash2 } from "lucide-react";
import type {
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

export interface StepNodesProps {
	nodes: NodeLabelMapping[];
	tables: string[];
	tableColumns: Record<string, PropertyColumn[]>;
	onChange: (nodes: NodeLabelMapping[]) => void;
}

const DEFAULT_STYLE: LabelStyle = {
	color: "#3b82f6",
	icon: "database",
	size: { mode: "fixed", value: 10 },
};

export function StepNodes({
	nodes,
	tables,
	tableColumns,
	onChange,
}: StepNodesProps) {
	const addNode = () => {
		onChange([
			...nodes,
			{
				label: "",
				table: tables[0] ?? "",
				id_column: "",
				property_columns: [],
				style: { ...DEFAULT_STYLE },
			},
		]);
	};

	const updateNode = (index: number, partial: Partial<NodeLabelMapping>) => {
		const updated = nodes.map((n, i) =>
			i === index ? { ...n, ...partial } : n,
		);
		onChange(updated);
	};

	const removeNode = (index: number) => {
		onChange(nodes.filter((_, i) => i !== index));
	};

	const togglePropertyColumn = (index: number, col: PropertyColumn) => {
		const node = nodes[index];
		if (!node) return;
		const exists = node.property_columns.some((c) => c.name === col.name);
		const cols = exists
			? node.property_columns.filter((c) => c.name !== col.name)
			: [...node.property_columns, col];
		updateNode(index, { property_columns: cols });
	};

	return (
		<div className="space-y-4">
			<div className="flex items-center justify-between">
				<div>
					<h3 className="text-sm font-medium mb-1">Node Mappings</h3>
					<p className="text-xs text-muted-foreground">
						Each mapping turns a table into a graph node label.
					</p>
				</div>
				<Button size="sm" variant="outline" onClick={addNode}>
					<Plus className="h-3.5 w-3.5 mr-1" />
					Add Node
				</Button>
			</div>

			<div className="max-h-[450px] overflow-y-auto">
				<div className="space-y-4 pr-2">
					{nodes.map((node, i) => {
						const cols = tableColumns[node.table] ?? [];
						const colNames = cols.map((c) => c.name);
						return (
							<Card key={i} className="p-4 space-y-3">
								<div className="flex items-center justify-between">
									<span className="text-xs font-medium text-muted-foreground">
										Node #{i + 1}
									</span>
									<Button
										variant="ghost"
										size="icon"
										className="h-7 w-7 text-destructive"
										onClick={() => removeNode(i)}
									>
										<Trash2 className="h-3.5 w-3.5" />
									</Button>
								</div>

								<div className="grid grid-cols-2 gap-3">
									<div className="space-y-1.5">
										<Label className="text-xs">Label</Label>
										<Input
											value={node.label}
											onChange={(e) => updateNode(i, { label: e.target.value })}
											className="h-8 text-xs"
											placeholder="Person"
										/>
									</div>
									<div className="space-y-1.5">
										<Label className="text-xs">Table</Label>
										<Select
											value={node.table}
											onValueChange={(v) =>
												updateNode(i, {
													table: v,
													id_column: "",
													display_column: undefined,
													property_columns: [],
												})
											}
										>
											<SelectTrigger className="h-8 text-xs">
												<SelectValue placeholder="Select table" />
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
										<Label className="text-xs">ID Column</Label>
										<ColumnPicker
											columns={colNames}
											value={node.id_column}
											onChange={(v) => updateNode(i, { id_column: v })}
											placeholder="Pick ID column"
										/>
									</div>
									<div className="space-y-1.5">
										<Label className="text-xs">Display Column</Label>
										<ColumnPicker
											columns={colNames}
											value={node.display_column ?? ""}
											onChange={(v) =>
												updateNode(i, { display_column: v || undefined })
											}
											placeholder="Optional"
										/>
									</div>
								</div>

								{cols.length > 0 && (
									<div className="space-y-1.5">
										<Label className="text-xs">Property Columns</Label>
										<p className="text-[10px] text-muted-foreground">
											Leave empty to include all non-vector columns.
										</p>
										<div className="flex flex-wrap gap-1.5">
											{cols.map((col) => (
												<button
													key={col.name}
													type="button"
													onClick={() => togglePropertyColumn(i, col)}
													className={`text-[10px] px-2 py-0.5 rounded-full border transition-colors ${
														node.property_columns.some(
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
									style={node.style}
									onChange={(s) => updateNode(i, { style: s })}
								/>
							</Card>
						);
					})}
					{nodes.length === 0 && (
						<p className="text-sm text-muted-foreground text-center py-8">
							No node mappings yet. Click &quot;Add Node&quot; to get started.
						</p>
					)}
				</div>
			</div>
		</div>
	);
}
