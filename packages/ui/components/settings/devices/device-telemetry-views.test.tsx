import { expect, test } from "bun:test";
import { renderToStaticMarkup } from "react-dom/server";
import { DeviceMessagesView } from "./device-messages-view";
import { DeviceMetricsView } from "./device-metrics-view";

test("retained usage labels checkpoint coverage separately from current process gauges", () => {
	const html = renderToStaticMarkup(
		<DeviceMetricsView
			connected
			placement
			sample={{
				records: [
					{
						timestamp: Date.now() / 1000,
						data: {
							cpu_percent: 175.5,
							memory_bytes: 1024,
							ready_replicas: 2,
							desired_replicas: 2,
							processes_observed: 2,
							running_replicas: 2,
							metric_coverage: {
								sampled_placements: 1,
								missing_or_stale_placements: 1,
							},
							usage_retained: {
								counters: {
									invocations_started: 20,
									runtime_messages: 30,
									request_payload_bytes: Number.MAX_SAFE_INTEGER + 1,
								},
								coverage: {
									registered_runs: 4,
									reported_runs: 3,
									finalized_runs: 1,
									incomplete_runs: 2,
									unreported_runs: 1,
									stale_active_runs: 1,
									awaiting_first_report_runs: 0,
								},
								tail_loss_possible: true,
							},
						},
					},
				],
			}}
		/>,
	);
	expect(html).toContain("175.5%");
	expect(html).toContain("1.0 KiB");
	expect(html).toContain("separate from billing");
	expect(html).toContain("ended before their first report");
	expect(html).toContain("Some activity may be missing");
	expect(html).toContain(
		'Request payload</dt><dd class="mt-1 font-medium tabular-nums">Unavailable',
	);
});

test("missing samples remain unavailable and a disconnected sample is labeled stale", () => {
	const html = renderToStaticMarkup(
		<DeviceMetricsView
			connected={false}
			placement
			sample={{
				records: [
					{
						timestamp: Date.now() / 1000,
						data: {
							cpu_percent: null,
							memory_bytes: null,
							usage_retained: {
								counters: null,
								coverage: {
									registered_runs: 1,
									reported_runs: 0,
									awaiting_first_report_runs: 1,
								},
								tail_loss_possible: true,
							},
						},
					},
				],
			}}
		/>,
	);
	expect(html).toContain("Last known sample");
	expect(html).toContain("Unavailable");
	expect(html).not.toContain("0.0%");
});

test("operational messages render typed lifecycle metadata without unrecognized payloads", () => {
	const html = renderToStaticMarkup(
		<DeviceMessagesView
			sample={{
				records: [
					{
						sequence: 1,
						timestamp: 1234,
						data: {
							kind: "operation",
							state: "completed",
							source_id: "safe-operation",
							placement_id: "api",
							secret: "do-not-render-this",
						},
					},
					{
						sequence: 2,
						timestamp: 1234,
						data: {
							kind: "arbitrary",
							state: "completed",
							source_id: "unknown-kind",
						},
					},
					{
						sequence: 3,
						timestamp: 1234,
						data: {
							kind: "replica",
							state: "<script>unknown-state</script>",
							source_id: "unknown-state",
						},
					},
				],
			}}
		/>,
	);
	expect(html).toContain("safe-operation");
	expect(html).toContain("Command: completed");
	expect(html).not.toContain("do-not-render-this");
	expect(html).not.toContain("unknown-kind");
	expect(html).not.toContain("unknown-state");
});

test("resource metrics distinguish host traffic from placement disk accounting", () => {
	const html = renderToStaticMarkup(
		<DeviceMetricsView
			connected
			placement={false}
			sample={{
				records: [
					{
						timestamp: Date.now() / 1000,
						data: {
							network: { received_bytes: 1024, transmitted_bytes: null },
							storage_volume: { available_bytes: 2048, total_bytes: 4096 },
							io: {
								basis: "cgroup_block_io",
								read_bytes_per_second: 512.5,
								written_bytes_per_second: null,
							},
							disk_quota: { used_bytes: 1024, limit_bytes: 8192 },
						},
					},
				],
			}}
		/>,
	);
	expect(html).toContain("Host interfaces received / sample");
	expect(html).toContain("not project traffic or billing");
	expect(html).toContain("isolated process group");
	expect(html).toContain("513 B/s");
	expect(html).toContain("1.0 KiB / 8.0 KiB");
	expect(html).toContain("Unavailable");
});
