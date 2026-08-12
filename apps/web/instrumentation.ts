import { captureServerRequestError } from "@/lib/server-telemetry";
import * as Sentry from "@sentry/nextjs";
import type { Instrumentation } from "next";

export async function register() {
	if (process.env.NEXT_RUNTIME === "nodejs") {
		await import("./sentry.server.config");
	}

	if (process.env.NEXT_RUNTIME === "edge") {
		await import("./sentry.edge.config");
	}
}

/**
 * Server and edge request errors go to both sinks: Sentry keeps the existing
 * identity-linked reporting, the internal ingest gets an anonymous, sanitized
 * copy. The internal capture runs first so a Sentry failure cannot swallow it.
 */
export const onRequestError: Instrumentation.onRequestError = (
	error,
	request,
	context,
) => {
	captureServerRequestError(error, request, context);
	return Sentry.captureRequestError(error, request, context);
};
