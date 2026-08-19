"use client";

import { useTranslation } from "@flow-like/locales";
import { Check } from "lucide-react";
import { useState } from "react";
import { cn } from "../../../../lib/utils";
import type {
	EdgeLabelMapping,
	NodeLabelMapping,
} from "../../../../state/backend-state/graph-state";
import { Card } from "../../card";
import { getGraphIcon } from "../icons";
import { type DomainPreset, applyPreset, getPresets } from "../presets";

export interface PresetPickerProps {
	nodes: NodeLabelMapping[];
	edges: EdgeLabelMapping[];
	onApply: (nodes: NodeLabelMapping[], edges: EdgeLabelMapping[]) => void;
}

function PresetPreview({ preset }: { preset: DomainPreset }) {
	const icons = preset.nodeRules.slice(0, 4).map((rule) => {
		const Icon = getGraphIcon(rule.icon);
		return { key: rule.icon, color: rule.color, Icon };
	});

	return (
		<div className="flex items-center gap-1.5 mt-1">
			{icons.map(({ key, color, Icon }) => (
				<span
					key={key}
					className="inline-flex items-center justify-center h-5 w-5 rounded-full"
					style={{ backgroundColor: `${color}20`, color }}
				>
					<Icon className="h-3 w-3" />
				</span>
			))}
		</div>
	);
}

export function PresetPicker({ nodes, edges, onApply }: PresetPickerProps) {
	const { t } = useTranslation("common");
	const presets = getPresets();
	const [activePreset, setActivePreset] = useState<string | null>(null);

	const handleApply = (preset: DomainPreset) => {
		const result = applyPreset(preset, nodes, edges);
		onApply(result.nodes, result.edges);
		setActivePreset(preset.name);
	};

	return (
		<div className="space-y-3">
			<p className="text-sm text-muted-foreground">
				{t(
					"pickADomainPresetToAutostyleYourNodeAndEdgeMappings",
					"Pick a domain preset to auto-style your node and edge mappings.",
				)}
			</p>
			<div className="grid grid-cols-1 sm:grid-cols-2 gap-3">
				{presets.map((preset) => {
					const isActive = activePreset === preset.name;
					return (
						<Card
							key={preset.name}
							className={cn(
								"p-4 space-y-1 cursor-pointer transition-colors hover:bg-accent/50",
								isActive && "ring-2 ring-primary bg-primary/5",
							)}
							onClick={() => handleApply(preset)}
						>
							<div className="flex items-start justify-between">
								<h4 className="text-sm font-medium">{preset.name}</h4>
								{isActive && (
									<Check className="h-4 w-4 text-primary shrink-0" />
								)}
							</div>
							<p className="text-xs text-muted-foreground">
								{preset.description}
							</p>
							<PresetPreview preset={preset} />
						</Card>
					);
				})}
			</div>
		</div>
	);
}
