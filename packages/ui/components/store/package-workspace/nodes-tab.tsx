"use client";

import { useTranslation } from "@flow-like/locales";
import { FileCode } from "lucide-react";
import type { PackageNodeEntry } from "../../../lib/schema/wasm";
import {
	CountBadge,
	WorkspaceNotice,
	WorkspaceSection,
} from "./workspace-parts";

/** The nodes the registry extracted from the live build. */
export function NodesTab({
	nodes,
	version,
}: Readonly<{ nodes: readonly PackageNodeEntry[]; version?: string }>) {
	const { t } = useTranslation("common");

	if (nodes.length === 0) {
		return (
			<WorkspaceNotice
				icon={FileCode}
				title={t("workspaceNoNodesTitle", "No nodes on the registry")}
				description={t(
					"workspaceNoNodesDescription",
					"The registry extracts nodes when it compiles a published version.",
				)}
			/>
		);
	}

	return (
		<WorkspaceSection
			icon={FileCode}
			title={
				<span className="flex items-center gap-2">
					{t("nodes", "Nodes")}
					<CountBadge value={nodes.length} />
				</span>
			}
			action={
				version && (
					<span className="shrink-0 text-xs text-muted-foreground">
						{t(
							"workspaceNodesFromVersion",
							"From the registry · v{{version}}",
							{
								version,
							},
						)}
					</span>
				)
			}
			bodyClassName="-mx-4 -mb-3.5"
		>
			<ul>
				{nodes.map((node) => (
					<li
						key={node.id}
						className="grid grid-cols-1 gap-x-4 gap-y-1 border-t border-border/60 px-4 py-2.5 text-sm sm:grid-cols-[minmax(8rem,14rem)_minmax(0,1fr)_auto] sm:items-center"
					>
						<span className="truncate font-medium">
							{node.friendlyName || node.name}
						</span>
						<span className="min-w-0 text-muted-foreground sm:truncate">
							{node.description}
						</span>
						{node.category && (
							<span className="w-fit rounded-md border border-border/60 bg-muted/40 px-1.5 py-0.5 font-mono text-[11px] text-muted-foreground">
								{node.category}
							</span>
						)}
					</li>
				))}
			</ul>
		</WorkspaceSection>
	);
}
