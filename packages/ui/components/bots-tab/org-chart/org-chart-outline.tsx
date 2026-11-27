"use client";

import { flattenForest, orgForest } from "../../../lib/bot-teams/org-model";
import { useOrgChart } from "./org-chart-context";
import { Bench, TeamBox } from "./org-chart-tree";

const INDENT_PX = 14;

export function OrgOutline() {
	const { org } = useOrgChart();
	const nodes = flattenForest(orgForest(org));
	return (
		<div className="flex flex-col gap-2 p-3">
			{nodes.map((node) => (
				<div
					key={node.team.id}
					style={{ marginLeft: node.depth * INDENT_PX }}
					className={node.depth > 0 ? "border-l pl-2" : undefined}
				>
					<TeamBox team={node.team} className="w-full" layout="inline" />
				</div>
			))}
			<Bench />
		</div>
	);
}
