// This file configures the initialization of Sentry on the client.
// The added config here will be used whenever a users loads a page in their browser.
// https://docs.sentry.io/platforms/javascript/guides/nextjs/

import { setFlowPilotProductionMetricsSink } from "@flow-like/flow-like-ui/state/global-chat/agent-debug-report";
import * as Sentry from "@sentry/nextjs";
import posthog from "posthog-js";

const posthogKey = process.env.NEXT_PUBLIC_POSTHOG_KEY;
if (posthogKey) {
	posthog.init(posthogKey, {
		api_host: process.env.NEXT_PUBLIC_POSTHOG_HOST,
		defaults: "2025-11-30",
		ui_host: "https://eu.posthog.com",
	});
	setFlowPilotProductionMetricsSink((metrics) => {
		posthog.capture("flowpilot_generation_metrics", metrics);
	});
}

Sentry.init({
	dsn: "https://f262b1c585a179ce9888298ae9e5d4c7@o4507505692901376.ingest.de.sentry.io/4510815115477072",

	// Add optional integrations for additional features
	integrations: [Sentry.replayIntegration()],

	// Define how likely traces are sampled. Adjust this value in production, or use tracesSampler for greater control.
	tracesSampleRate: 0.3,
	// Enable logs to be sent to Sentry
	enableLogs: true,

	// Define how likely Replay events are sampled.
	// This sets the sample rate to be 10%. You may want this to be 100% while
	// in development and sample at a lower rate in production
	replaysSessionSampleRate: 0.1,

	// Define how likely Replay events are sampled when an error occurs.
	replaysOnErrorSampleRate: 1.0,

	// Enable sending user PII (Personally Identifiable Information)
	// https://docs.sentry.io/platforms/javascript/guides/nextjs/configuration/options/#sendDefaultPii
	sendDefaultPii: false,
});

export const onRouterTransitionStart = Sentry.captureRouterTransitionStart;
