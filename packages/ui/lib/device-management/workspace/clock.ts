import type { ClockModel, WorkspaceDeps } from "./types";

/** The hub sets a signaling admission's `expires_at = min(authorized_until, now + 300)`. */
export const ADMISSION_TTL_S = 300;
const WINDOW_MS = 10 * 60_000;
const MAX_SAMPLES = 8;
/** A sample this far above the estimate means this computer's clock was changed. */
const CLOCK_JUMP_S = 30;

export type ClockSource = Parameters<ClockModel["observe"]>[0];

/**
 * Hub time implied by a signaling admission. Exact within the request latency
 * only for owners and for grants that end more than 300 s later; callers pass
 * it to `observe("admission", …)` only then.
 */
export function admissionHubTimeS(expiresAt: number): number {
	return expiresAt - ADMISSION_TTL_S;
}

export interface DeviceClockModel extends ClockModel {
	/**
	 * `localMs` is this computer's raw clock at arrival. For `snapshot`, `hubTimeS`
	 * is the device's `observed_at` and `deviceId` names the device; a snapshot
	 * sample without one is ignored.
	 */
	observe(
		source: ClockSource,
		hubTimeS: number,
		localMs: number,
		deviceId?: string,
	): void;
}

interface Sample {
	value: number;
	at: number;
}

function recent(samples: readonly Sample[], next: Sample): Sample[] {
	return [...samples, next]
		.filter((sample) => sample.at >= next.at - WINDOW_MS)
		.slice(-MAX_SAMPLES);
}

const lowest = (samples: readonly Sample[]) =>
	samples.length
		? Math.min(...samples.map((sample) => sample.value))
		: undefined;
const highest = (samples: readonly Sample[]) =>
	samples.length
		? Math.max(...samples.map((sample) => sample.value))
		: undefined;

/**
 * M-DATA §3.11. Every offset sample (`local − hub`) overstates the offset by the
 * request latency, so the lowest recent one is the estimate; every snapshot
 * sample (`device − local`) understates the device clock by the snapshot's age,
 * so the highest recent one is. `server_time` wins over the admission heuristic.
 */
export function createClockModel(
	deps: Pick<WorkspaceDeps, "now">,
): DeviceClockModel {
	const localNow = deps.now ?? Date.now;
	const offsets: Record<"server_time" | "admission", Sample[]> = {
		server_time: [],
		admission: [],
	};
	const devices = new Map<string, Sample[]>();

	const hubOffset = () =>
		lowest(offsets.server_time) ?? lowest(offsets.admission);

	function observeOffset(source: "server_time" | "admission", sample: Sample) {
		const current = lowest(offsets[source]);
		offsets[source] =
			current !== undefined && sample.value - current > CLOCK_JUMP_S
				? [sample]
				: recent(offsets[source], sample);
	}

	return {
		now: () => localNow() - (hubOffset() ?? 0) * 1_000,
		get hubOffsetS() {
			return hubOffset();
		},
		deviceSkewS(deviceId) {
			const deviceMinusLocal = highest(devices.get(deviceId) ?? []);
			return deviceMinusLocal === undefined
				? undefined
				: deviceMinusLocal + (hubOffset() ?? 0);
		},
		observe(source, hubTimeS, localMs, deviceId) {
			if (!Number.isFinite(hubTimeS) || !Number.isFinite(localMs)) return;
			const localS = localMs / 1_000;
			if (source === "snapshot") {
				if (!deviceId) return;
				devices.set(
					deviceId,
					recent(devices.get(deviceId) ?? [], {
						value: hubTimeS - localS,
						at: localMs,
					}),
				);
				return;
			}
			observeOffset(source, { value: localS - hubTimeS, at: localMs });
		},
	};
}
