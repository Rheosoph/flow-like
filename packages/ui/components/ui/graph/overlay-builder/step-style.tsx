"use client";

import type {
	EdgeLabelMapping,
	NodeLabelMapping,
} from "../../../../state/backend-state/graph-state";
import { Separator } from "../../separator";
import { PresetPicker } from "./preset-picker";

export interface StepStyleProps {
	tables: string[];
	nodes: NodeLabelMapping[];
	edges: EdgeLabelMapping[];
	onApplyPreset: (nodes: NodeLabelMapping[], edges: EdgeLabelMapping[]) => void;
}

export function StepStyle({
	tables,
	nodes,
	edges,
	onApplyPreset,
}: StepStyleProps) {
	return (
		<div className="space-y-4">
			<div>
				<h3 className="text-sm font-medium mb-1">Style & Presets</h3>
				<p className="text-xs text-muted-foreground">
					Optionally apply a domain preset to auto-style your node and edge
					mappings. You can also customize individual styles in the Node and
					Edge steps.
				</p>
			</div>

			<Separator />

			<PresetPicker nodes={nodes} edges={edges} onApply={onApplyPreset} />

			{(nodes.length > 0 || edges.length > 0) && (
				<>
					<Separator />
					<div>
						<p className="text-xs text-muted-foreground">
							Current configuration: {nodes.length} node label
							{nodes.length !== 1 ? "s" : ""}, {edges.length} edge label
							{edges.length !== 1 ? "s" : ""}.
						</p>
					</div>
				</>
			)}
		</div>
	);
}
