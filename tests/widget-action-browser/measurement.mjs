import { createHash } from "node:crypto";

/** Groups method/path pairs without retaining paths, credentials, or query values. */
export function requestRouteKey(rawUrl, method) {
	try {
		const url = new URL(rawUrl);
		return createHash("sha256")
			.update(JSON.stringify([method, url.pathname]))
			.digest("hex");
	} catch {
		return null;
	}
}

export function percentile(values, fraction) {
	if (!values.length) return null;
	const sorted = [...values].sort((a, b) => a - b);
	const position = (sorted.length - 1) * fraction;
	const lower = Math.floor(position);
	return (
		sorted[lower] +
		(sorted[Math.ceil(position)] - sorted[lower]) * (position - lower)
	);
}

export function summarize(
	samples,
	metrics = [
		"clickToDispatchMs",
		"clickToPatchMs",
		"clickToHistoryDomMs",
		"clickToHistoryFrameOpportunityMs",
		"clickToSettledMs",
		"handlerToDispatchMs",
		"handlerToPatchMs",
		"handlerToSettledMs",
		"scheduledToClickMs",
	],
) {
	const successful = samples.filter((sample) => sample.status === "ok");
	return {
		attempts: samples.length,
		successful: successful.length,
		failed: samples.length - successful.length,
		metrics: Object.fromEntries(
			metrics.map((metric) => {
				const values = successful
					.map((sample) => sample[metric])
					.filter(Number.isFinite);
				return [
					metric,
					{
						count: values.length,
						p50: percentile(values, 0.5),
						p95: percentile(values, 0.95),
						p99: percentile(values, 0.99),
						min: values.length ? Math.min(...values) : null,
						max: values.length ? Math.max(...values) : null,
					},
				];
			}),
		),
	};
}
