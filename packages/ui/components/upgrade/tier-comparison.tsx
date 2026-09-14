"use client";

import { ChevronDown } from "lucide-react";
import { formatQuota } from "../../lib/quota";
import type { ITierInfo } from "../../state/backend-state/user-state";
import { ENTERPRISE_TIER, TIER_ORDER, formatBytes } from "./tier-card";

const limits: { label: string; value: (tier: ITierInfo) => string }[] = [
	{
		label: "Cloud runtime / month",
		value: (tier) =>
			tier.max_runtime_ms === undefined
				? "Not specified"
				: formatQuota(tier.max_runtime_ms, "cloud_runtime_ms"),
	},
	{
		label: "Hosted AI allowance / month",
		value: (tier) =>
			formatQuota(
				tier.max_ai_cost_micros ?? tier.max_llm_cost * 10_000,
				"hosted_ai_cost_micros",
			),
	},
	{ label: "Cloud storage", value: (tier) => formatBytes(tier.max_total_size) },
	{
		label: "Private cloud projects",
		value: (tier) => formatQuota(tier.max_non_visible_projects, "projects"),
	},
	{
		label: "Cloud starts / month",
		value: (tier) => formatQuota(tier.max_remote_executions, "cloud_starts"),
	},
	{
		label: "Hosted AI operations / month",
		value: (tier) =>
			tier.max_llm_calls === undefined
				? "Not specified"
				: formatQuota(tier.max_llm_calls, "hosted_ai_calls"),
	},
	{
		label: "Concurrent cloud executions",
		value: (tier) =>
			tier.max_concurrent_executions === undefined
				? "Not specified"
				: formatQuota(
						tier.max_concurrent_executions,
						"concurrent_cloud_executions",
					),
	},
	{
		label: "Hosted model access",
		value: (tier) =>
			tier.llm_tiers
				.map((name) => name.charAt(0) + name.slice(1).toLowerCase())
				.join(", ") || "None",
	},
];

export function TierComparison({
	tiers,
}: { tiers: Record<string, ITierInfo> }) {
	const ordered = Object.entries(tiers).sort(([a], [b]) => {
		const rank = (name: string) =>
			TIER_ORDER.indexOf(name) < 0
				? TIER_ORDER.length
				: TIER_ORDER.indexOf(name);
		return rank(a) - rank(b) || a.localeCompare(b);
	});
	if (ordered.length === 0) return null;
	return (
		<details className="group rounded-xl border bg-card">
			<summary className="flex cursor-pointer list-none items-center justify-between gap-3 p-4 font-medium [&::-webkit-details-marker]:hidden">
				Compare all limits
				<ChevronDown className="h-4 w-4 shrink-0 transition-transform group-open:rotate-180" />
			</summary>
			<div className="overflow-x-auto border-t">
				<table className="w-full min-w-[640px] text-left text-sm">
					<caption className="sr-only">
						Plan allowances and included hosted model tiers
					</caption>
					<thead>
						<tr className="border-b bg-muted/30">
							<th scope="col" className="p-4 font-medium">
								Allowance
							</th>
							{ordered.map(([key, tier]) => (
								<th key={key} scope="col" className="p-4 font-semibold">
									{tier.display_name ?? tier.name ?? key}
								</th>
							))}
						</tr>
					</thead>
					<tbody>
						{limits.map((limit) => (
							<tr key={limit.label} className="border-b last:border-0">
								<th
									scope="row"
									className="p-4 font-normal text-muted-foreground"
								>
									{limit.label}
								</th>
								{ordered.map(([key, tier]) => (
									<td key={key} className="p-4 tabular-nums">
										{key === ENTERPRISE_TIER
											? "By agreement"
											: limit.value(tier)}
									</td>
								))}
							</tr>
						))}
					</tbody>
				</table>
			</div>
			<div className="space-y-2 border-t p-4 text-xs leading-relaxed text-muted-foreground">
				<p>
					Runtime, cloud starts and hosted AI allowances renew monthly,
					including on annual subscriptions. Both runtime and start limits
					apply; both AI money and operation limits apply.
				</p>
				<p>
					Storage and projects measure what you currently keep in the cloud.
					Concurrent execution slots become available when work finishes.
					Public-project forks, including purchases, use no project slots; their
					storage and cloud usage still count.
				</p>
				<p>
					Local execution and local embeddings are free. Your own models use no
					Flow-Like AI allowance or operations; your provider may charge
					separately. Cloud workflows still use cloud runtime.
				</p>
			</div>
		</details>
	);
}
