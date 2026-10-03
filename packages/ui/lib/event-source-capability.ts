const CONFIRMED_TTL_MS = 60_000;
const UNCONFIRMED_TTL_MS = 10_000;

export const DEVICE_EVENT_CREATION_UNSUPPORTED =
	"Update the hub to create events directly on devices.";

interface Entry {
	result: Promise<void>;
	expiresAt: number;
}

const entries = new Map<string, Entry>();

function explainProbeFailure(error: unknown): unknown {
	const status = (error as { status?: unknown } | null)?.status;
	if (status === 403) {
		return new Error(
			"This account or token cannot manage devices on the hub, so it cannot create events directly on devices.",
		);
	}
	if (status === 503) {
		return new Error(
			"This hub does not manage devices, so it cannot create events directly on devices.",
		);
	}
	return error;
}

/**
 * Resolves when the hub can create device-only events for `key`, otherwise rejects. Concurrent
 * callers share one probe. A confirmed answer is reused for a minute; a failed or negative one
 * for only a few seconds, so a hub upgrade or a granted permission is noticed quickly.
 */
export function ensureDeviceEventCreation(
	key: string,
	probe: () => Promise<{ device_event_creation?: boolean } | undefined>,
	now: () => number = Date.now,
): Promise<void> {
	const cached = entries.get(key);
	if (cached && cached.expiresAt > now()) return cached.result;

	const entry: Entry = {
		result: Promise.resolve(),
		expiresAt: Number.POSITIVE_INFINITY,
	};
	entry.result = (async () => {
		try {
			const capabilities = await probe();
			if (capabilities?.device_event_creation !== true) {
				throw new Error(DEVICE_EVENT_CREATION_UNSUPPORTED);
			}
			entry.expiresAt = now() + CONFIRMED_TTL_MS;
		} catch (error) {
			entry.expiresAt = now() + UNCONFIRMED_TTL_MS;
			throw explainProbeFailure(error);
		}
	})();
	entries.set(key, entry);
	return entry.result;
}

export function resetDeviceEventCreationCache(): void {
	entries.clear();
}
