"use client";

import { useEffect, useRef, useState } from "react";
import {
	type DeviceMetricField,
	type MetricPoint,
	type MetricsHistoryInput,
	type PlacementMetricField,
	agentSupports,
	readMetricsHistory,
} from "../../../../lib/device-management/agent-reads";
import type { AgentFeatures } from "../../../../lib/device-management/model/types";
import type { ManagementCall } from "../../../../lib/device-management/telemetry";
import { deviceCall, useDeviceWorkspace } from "../workspace";
import { livePhase } from "./live-state";
import type { MetricSample } from "./observe-data";
import type { ObserveTarget } from "./use-observe-target";

const DEVICE_FIELDS: readonly DeviceMetricField[] = [
	"cpu_percent",
	"memory_used_bytes",
	"memory_total_bytes",
];
const SERVICE_FIELDS: readonly PlacementMetricField[] = [
	"cpu_percent",
	"memory_bytes",
];
const EVERY_MS = 30_000;
const MAX_PAGES = 5;
/** 30 minutes of 5 s samples. */
const KEEP = 360;

export interface MetricsHistory {
	/** The device's own samples of CPU and memory, oldest first; undefined until read or when unsupported. */
	samples: MetricSample[] | undefined;
	/** The agent doesn't serve its history (BG16): trends come from the encrypted status instead. */
	supported: boolean;
}

const inputOf = (
	serviceId: string | null,
	after: number,
): MetricsHistoryInput =>
	serviceId
		? { placementId: serviceId, fields: SERVICE_FIELDS, after }
		: { placementId: null, fields: DEVICE_FIELDS, after };

const sampleOf = (
	fields: readonly string[],
	[at, ...values]: MetricPoint,
): MetricSample => ({
	at,
	data: Object.fromEntries(
		fields.map((field, index) => [field, values[index]]),
	),
});

/**
 * Reads pages after `cursor` until the device has no newer sample (or a page
 * limit is hit). The cursor only moves while more remains, so the tail is read
 * again next time and deduplicated by the caller.
 */
async function readNewer(
	call: ManagementCall,
	features: AgentFeatures | undefined,
	serviceId: string | null,
	cursor: number,
): Promise<{ samples: MetricSample[]; cursor: number }> {
	const samples: MetricSample[] = [];
	let after = cursor;
	for (let page = 0; page < MAX_PAGES; page++) {
		const input = inputOf(serviceId, after);
		const result = await readMetricsHistory(call, features, input);
		if (result.kind !== "ok") break;
		for (const point of result.data.points)
			samples.push(sampleOf(input.fields, point));
		if (result.data.next === null) break;
		after = result.data.next;
	}
	return { samples, cursor: after };
}

const appendNewer =
	(fresh: readonly MetricSample[]) =>
	(known: MetricSample[] = []): MetricSample[] => {
		const newest = known.at(-1)?.at ?? 0;
		return [...known, ...fresh.filter((sample) => sample.at > newest)].slice(
			-KEEP,
		);
	};

/**
 * BG16: the device's metric history for denser trends. Only an agent that
 * announces `metrics_history` is asked; pages are read once, then only newer
 * points. Dropped when the session closes (decrypted data never outlives the keys).
 */
export function useMetricsHistory(target: ObserveTarget): MetricsHistory {
	const workspace = useDeviceWorkspace();
	const { deviceId, serviceId, features } = target;
	const supported = agentSupports(features, "metrics_history");
	const enabled = supported && livePhase(target) === "open";
	const [samples, setSamples] = useState<MetricSample[]>();
	const featuresRef = useRef(features);
	featuresRef.current = features;

	useEffect(() => {
		if (!enabled) {
			setSamples(undefined);
			return;
		}
		let stopped = false;
		let cursor = 0;
		let running = false;
		const call = deviceCall(workspace, deviceId, "poll");
		const read = async () => {
			if (running) return;
			running = true;
			try {
				const newer = await readNewer(
					call,
					featuresRef.current,
					serviceId,
					cursor,
				);
				cursor = newer.cursor;
				if (!stopped && newer.samples.length)
					setSamples(appendNewer(newer.samples));
			} catch {
				// A failed page keeps the trend read so far; the next cycle retries.
			} finally {
				running = false;
			}
		};
		void read();
		const timer = setInterval(() => void read(), EVERY_MS);
		return () => {
			stopped = true;
			clearInterval(timer);
		};
	}, [workspace, deviceId, serviceId, enabled]);

	return { samples: enabled ? samples : undefined, supported };
}
