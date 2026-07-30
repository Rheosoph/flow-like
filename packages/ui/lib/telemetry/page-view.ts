import { captureTelemetryEvent } from "./capture";
import { TELEMETRY_PAGE_VIEW_EVENT } from "./sampling";

const OPAQUE_ID_SEGMENT = /^[a-z0-9]{16,}$/i;
const NUMERIC_SEGMENT = /^\d+$/;
const UUID_SEGMENT =
	/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
const MAX_TELEMETRY_PATH_LENGTH = 256;

/** Strip query/hash and replace id-like path segments so paths stay anonymous. */
export function sanitizeTelemetryPath(path: string): string {
	const withoutQuery = path.split(/[?#]/, 1)[0] ?? "";
	const sanitized = withoutQuery
		.split("/")
		.map((segment) =>
			OPAQUE_ID_SEGMENT.test(segment) ||
			NUMERIC_SEGMENT.test(segment) ||
			UUID_SEGMENT.test(segment)
				? ":id"
				: segment,
		)
		.join("/")
		.slice(0, MAX_TELEMETRY_PATH_LENGTH);
	return sanitized.length === 0 ? "/" : sanitized;
}

/**
 * Route changes are the highest-volume capture and are sampled. The gate lives
 * in `captureTelemetryEvent` alone: `shouldSampleEvent` advances per-session
 * state, so asking it twice for one capture would consume two decisions.
 */
export function capturePageView(pathname: string) {
	captureTelemetryEvent(TELEMETRY_PAGE_VIEW_EVENT, {
		path: sanitizeTelemetryPath(pathname),
	});
}
