"use client";

import { Badge, cn } from "@flow-like/flow-like-ui";
import type { WasmNodeDefinition } from "@flow-like/flow-like-ui/lib/schema/developer";
import { useTranslation } from "@flow-like/locales";
import { NodePermissionsSummary } from "./permissions";
import { isDataPin } from "./schema";

export function NodeCard({
	node,
	isSelected,
	onSelect,
}: {
	node: WasmNodeDefinition;
	isSelected: boolean;
	onSelect: () => void;
}) {
	const { t } = useTranslation("common");
	const inputCount = node.pins.filter((p) => isDataPin(p, "Input")).length;
	const outputCount = node.pins.filter((p) => isDataPin(p, "Output")).length;

	return (
		<button
			type="button"
			onClick={onSelect}
			className={cn(
				"w-full text-left rounded-lg border p-3 transition-colors",
				isSelected
					? "border-primary/40 bg-primary/5"
					: "border-border/20 hover:bg-muted/10",
			)}
		>
			<div className="flex items-start justify-between gap-2">
				<div className="min-w-0 flex-1">
					<div className="flex items-center gap-2">
						{node.icon && <span className="text-lg">{node.icon}</span>}
						<span className="font-medium text-sm truncate">
							{node.friendly_name}
						</span>
					</div>
					{node.description && (
						<p className="text-xs text-muted-foreground/60 mt-1 line-clamp-2">
							{node.description}
						</p>
					)}
				</div>
				<Badge variant="secondary" className="text-[10px] shrink-0">
					{node.category}
				</Badge>
			</div>
			<div className="flex items-center gap-2 mt-2">
				<span className="text-[10px] text-muted-foreground/60">
					{t(
						"inputcountInOutputcountOut",
						"{{inputCount}} in / {{outputCount}} out",
						{ inputCount, outputCount },
					)}
				</span>
				{node.long_running && (
					<Badge variant="outline" className="text-[10px]">
						{t("longRunning", "Long Running")}
					</Badge>
				)}
			</div>
			<div className="mt-3">
				<NodePermissionsSummary
					permissions={node.permissions}
					title="Requires"
					description={t(
						"sandboxCapabilitiesThisNodeAsksForDuringExecution",
						"Sandbox capabilities this node asks for during execution.",
					)}
					className={cn(
						"transition-colors",
						isSelected ? "border-primary/20 bg-primary/5" : "bg-background/50",
					)}
				/>
			</div>
		</button>
	);
}
