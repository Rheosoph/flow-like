"use client";

import { useEffect } from "react";
import { toast } from "sonner";
import {
	formatQuota,
	formatQuotaDate,
	quotaLabels,
	quotaWarningKey,
} from "../../lib/quota";
import type { QuotaOverview, QuotaResource } from "../../lib/quota";

const warned = new Set<string>();

function showQuotaNotice(
	overview: QuotaOverview,
	resourceName: string,
	threshold: number,
	resource?: QuotaResource,
) {
	const label =
		resourceName === "hosted_ai_cost_micros"
			? "Hosted AI allowance"
			: (quotaLabels[resourceName] ?? "Plan allowance");
	const occupancy =
		resourceName === "storage_bytes" || resourceName.includes("project");
	const plan = overview.plan.charAt(0) + overview.plan.slice(1).toLowerCase();
	const description = [
		resource
			? `${formatQuota(resource.used, resourceName)} of ${formatQuota(resource.limit, resourceName)} used.`
			: "View your usage for details.",
		occupancy ? undefined : `Renews ${formatQuotaDate(overview.periodEnd)}.`,
	]
		.filter(Boolean)
		.join(" ");
	const notify = threshold >= 90 ? toast.warning : toast.info;
	notify(
		`${plan} plan · ${label} ${threshold >= 100 ? "reached" : `${threshold}% used`}`,
		{
			description,
			action: {
				label: "View usage",
				onClick: () => {
					window.location.href = "/subscription?tab=usage";
				},
			},
		},
	);
}

export function QuotaWarnings({ overview }: { overview?: QuotaOverview }) {
	useEffect(() => {
		// Mounted app-wide: a restored or partial usage answer must not take the shell down.
		if (!overview || !Array.isArray(overview.resources)) return;
		if (Array.isArray(overview.warnings)) {
			for (const notice of overview.warnings) {
				if (warned.has(notice.id)) continue;
				warned.add(notice.id);
				showQuotaNotice(
					overview,
					notice.resource,
					notice.threshold,
					overview.resources.find(
						(resource) => resource.resource === notice.resource,
					),
				);
			}
			return;
		}
		for (const resource of overview.resources) {
			if (
				resource.limit < 0 ||
				resource.resource === "concurrent_cloud_executions"
			)
				continue;
			const threshold = resource.threshold;
			const occupancy =
				resource.resource === "storage_bytes" ||
				resource.resource.includes("project");
			if (!threshold) {
				if (occupancy)
					for (const level of [75, 90, 100]) {
						const key = quotaWarningKey(overview, resource, level);
						warned.delete(key);
						try {
							localStorage.removeItem(key);
						} catch {
							/* In-memory state still rearms. */
						}
					}
				continue;
			}
			const key = quotaWarningKey(overview, resource, threshold);
			let seen = warned.has(key);
			try {
				seen ||= localStorage.getItem(key) === "shown";
			} catch {
				/* Session deduplication remains available. */
			}
			if (seen) continue;
			// Mark lower thresholds too so late responses cannot display an older warning.
			for (const level of [75, 90, 100].filter((level) => level <= threshold)) {
				const levelKey = quotaWarningKey(overview, resource, level);
				warned.add(levelKey);
				try {
					localStorage.setItem(levelKey, "shown");
				} catch {
					/* Storage can be unavailable in private mode. */
				}
			}
			showQuotaNotice(overview, resource.resource, threshold, resource);
		}
	}, [overview]);
	return null;
}
