import { formatDurationMs } from "./traces-shared";

export type ITelemetryAlertMetric =
	| "error_rate"
	| "latency_p95"
	| "crash_free_rate"
	| "event_count"
	| "span_error_rate"
	| "llm_error_rate";

export type ITelemetryAlertComparator = "gt" | "lt";

export type ITelemetryAlertMode = "threshold" | "anomaly";

export type ITelemetryAlertStatus = "triggered" | "resolved";

export type ITelemetryAlertMetricUnit = "ratio" | "ms" | "count";

export interface ITelemetryAlertRule {
	id: string;
	name: string;
	metric: string;
	source?: string | null;
	comparator: string;
	threshold?: number | null;
	mode: string;
	windowMinutes: number;
	sensitivity?: number | null;
	minSamples: number;
	enabled: boolean;
	notifyEmail: boolean;
	notifyPush: boolean;
	lastEvaluatedAt?: string | null;
	lastTriggeredAt?: string | null;
	lastValue?: number | null;
	createdAt: string;
	updatedAt: string;
}

export interface ITelemetryAlertRulesResponse {
	rules: ITelemetryAlertRule[];
}

export interface ITelemetryAlertEvent {
	id: string;
	ruleId: string;
	ruleName: string;
	status: string;
	value: number;
	threshold?: number | null;
	message: string;
	acknowledgedAt?: string | null;
	createdAt: string;
}

export interface ITelemetryAlertEventsResponse {
	events: ITelemetryAlertEvent[];
	total: number;
	page: number;
	pageSize: number;
	/** Triggered alerts in the window that were never acknowledged. */
	unacknowledged: number;
}

export interface ITelemetryAlertEvaluationResponse {
	evaluated: number;
	triggered: number;
	resolved: number;
}

/** Deleting a rule also deletes its inbox entries — the server reports how many. */
export interface ITelemetryAlertRuleDeleteResponse {
	id: string;
	eventsDeleted: number;
}

/** Request bodies stay snake_case, mirroring the other telemetry admin mutations. */
export interface ITelemetryAlertRulePayload {
	name: string;
	metric: string;
	source: string | null;
	comparator: string;
	threshold: number | null;
	mode: string;
	window_minutes: number;
	sensitivity: number | null;
	min_samples: number;
	enabled: boolean;
	notify_email: boolean;
	notify_push: boolean;
}

export const ALERTS_QUERY_KEY = ["admin", "telemetry", "alerts"];

export const ALERT_RULES_PATH = "admin/telemetry/alerts";

export const ALERT_EVENTS_PATH = "admin/telemetry/alerts/events";

export const ALERT_EVALUATE_PATH = "admin/telemetry/alerts/evaluate";

export function alertRulePath(id: string) {
	return `${ALERT_RULES_PATH}/${encodeURIComponent(id)}`;
}

/** Acknowledgement hangs off the rules collection, not off the inbox listing. */
export function alertEventAckPath(id: string) {
	return `${ALERT_RULES_PATH}/${encodeURIComponent(id)}/ack`;
}

export interface AlertMetricMeta {
	value: ITelemetryAlertMetric;
	label: string;
	unit: ITelemetryAlertMetricUnit;
	hint: string;
}

/** Rate metrics are fractions in 0..1 — the UI renders them as percentages. */
export const ALERT_METRIC_OPTIONS: AlertMetricMeta[] = [
	{
		value: "error_rate",
		label: "Error rate",
		unit: "ratio",
		hint: "Share of error events, 0.05 = 5%",
	},
	{
		value: "latency_p95",
		label: "Latency p95",
		unit: "ms",
		hint: "95th percentile span duration in milliseconds",
	},
	{
		value: "crash_free_rate",
		label: "Crash-free sessions",
		unit: "ratio",
		hint: "Share of sessions without a crash, 0.995 = 99.5%",
	},
	{
		value: "event_count",
		label: "Event count",
		unit: "count",
		hint: "Number of telemetry events in the window",
	},
	{
		value: "span_error_rate",
		label: "Span error rate",
		unit: "ratio",
		hint: "Share of spans with status error, 0.02 = 2%",
	},
	{
		value: "llm_error_rate",
		label: "LLM error rate",
		unit: "ratio",
		hint: "Share of failed LLM calls, 0.02 = 2%",
	},
];

const NEUTRAL_METRIC: AlertMetricMeta = {
	value: "event_count",
	label: "Unknown metric",
	unit: "count",
	hint: "",
};

export function alertMetricMeta(metric: string): AlertMetricMeta {
	return (
		ALERT_METRIC_OPTIONS.find((option) => option.value === metric) ??
		NEUTRAL_METRIC
	);
}

export function alertMetricLabel(metric: string) {
	const meta = ALERT_METRIC_OPTIONS.find((option) => option.value === metric);
	return meta?.label ?? metric;
}

export const ALERT_MODE_OPTIONS: {
	value: ITelemetryAlertMode;
	label: string;
	hint: string;
}[] = [
	{
		value: "threshold",
		label: "Threshold",
		hint: "Fires when the metric crosses a fixed value.",
	},
	{
		value: "anomaly",
		label: "Anomaly",
		hint: "Fires when the metric deviates from the baseline of previous windows.",
	},
];

export const ALERT_COMPARATOR_OPTIONS: {
	value: ITelemetryAlertComparator;
	label: string;
}[] = [
	{ value: "gt", label: "Above (>)" },
	{ value: "lt", label: "Below (<)" },
];

export type ITelemetryAlertChannel = "email" | "push";

export interface AlertChannelMeta {
	value: ITelemetryAlertChannel;
	/** Badge text in the rule list. */
	label: string;
	/** Switch label in the rule dialog. */
	title: string;
	/** Who actually receives it. */
	description: string;
}

/**
 * Out-of-band delivery on top of the inbox. Both channels fire on the firing
 * and on the recovery transition, and neither has per-rule recipients.
 */
const ALERT_CHANNEL_META: Record<ITelemetryAlertChannel, AlertChannelMeta> = {
	email: {
		value: "email",
		label: "Email",
		title: "Email the platform alerting mailbox",
		description:
			"Goes to the single operator mailbox configured for this platform — rules cannot add their own recipients.",
	},
	push: {
		value: "push",
		label: "Push",
		title: "Push to platform admins",
		description:
			"Notifies every user holding the Admin permission, in-app and on their registered devices.",
	},
};

export const ALERT_CHANNEL_OPTIONS: AlertChannelMeta[] = [
	ALERT_CHANNEL_META.email,
	ALERT_CHANNEL_META.push,
];

export function alertChannelMeta(
	channel: ITelemetryAlertChannel,
): AlertChannelMeta {
	return ALERT_CHANNEL_META[channel];
}

/** The channels a rule delivers on, in a stable order. */
export function alertRuleChannels(
	rule: ITelemetryAlertRule,
): AlertChannelMeta[] {
	const enabled: Record<ITelemetryAlertChannel, boolean> = {
		email: rule.notifyEmail,
		push: rule.notifyPush,
	};
	return ALERT_CHANNEL_OPTIONS.filter((channel) => enabled[channel.value]);
}

export type ITelemetryAlertSource =
	| "desktop"
	| "desktop_core"
	| "desktop_native"
	| "web"
	| "web_server"
	| "backend";

/**
 * Closed source vocabulary of the ingest. Event batches only accept
 * `desktop | desktop_core | web | backend`, every other stream also accepts
 * `desktop_native` and `web_server`, so a rule on an event-backed metric never
 * fires when it is scoped to a source that stream cannot emit.
 */
export const ALERT_SOURCE_OPTIONS: readonly ITelemetryAlertSource[] = [
	"desktop",
	"desktop_core",
	"desktop_native",
	"web",
	"web_server",
	"backend",
];

/** Metrics whose numerator or denominator reads the telemetry event stream. */
export const EVENT_BACKED_ALERT_METRICS: readonly ITelemetryAlertMetric[] = [
	"event_count",
	"error_rate",
];

/** Sources that only ever reach the crash ingest, never the event ingest. */
export const ERROR_ONLY_ALERT_SOURCES: readonly ITelemetryAlertSource[] = [
	"desktop_native",
	"web_server",
];

const ALERT_SOURCE_META: Record<
	ITelemetryAlertSource,
	{ label: string; hint: string }
> = {
	desktop: {
		label: "Desktop (app)",
		hint: "Desktop frontend — reports every metric.",
	},
	desktop_core: {
		label: "Desktop (core)",
		hint: "Desktop Rust core — reports every metric.",
	},
	desktop_native: {
		label: "Desktop (native)",
		hint: "Native crash reporter — no product events, so event count and error rate never fire on it.",
	},
	web: { label: "Web", hint: "Web app — reports every metric." },
	web_server: {
		label: "Web (server)",
		hint: "Next.js server and edge runtime — crash reports only, so event count and error rate never fire on it.",
	},
	backend: {
		label: "Backend",
		hint: "API and workers — reports every metric.",
	},
};

export function alertSourceLabel(source: string): string {
	return ALERT_SOURCE_META[source as ITelemetryAlertSource]?.label ?? source;
}

export function alertSourceHint(source: string): string | undefined {
	return ALERT_SOURCE_META[source as ITelemetryAlertSource]?.hint;
}

/**
 * A rule scoped to a source that cannot emit the metric's underlying stream can
 * never fire. Returns the reason so the UI can say so before the rule is saved.
 */
export function alertSourceMismatch(
	metric: string,
	source: string | null | undefined,
): string | null {
	if (!source) return null;
	if (!ERROR_ONLY_ALERT_SOURCES.includes(source as ITelemetryAlertSource))
		return null;
	if (!EVENT_BACKED_ALERT_METRICS.includes(metric as ITelemetryAlertMetric)) {
		return null;
	}
	return `${alertMetricLabel(metric)} is computed from product events, which this source never sends — a rule on "${alertSourceLabel(source)}" can never fire.`;
}

export const ALERT_STATUS_OPTIONS: ITelemetryAlertStatus[] = [
	"triggered",
	"resolved",
];

export const ALERT_HOUR_OPTIONS: { value: number; label: string }[] = [
	{ value: 24, label: "Last 24 hours" },
	{ value: 72, label: "Last 3 days" },
	{ value: 168, label: "Last 7 days" },
	{ value: 720, label: "Last 30 days" },
];

export const ALERT_WINDOW_OPTIONS: { value: number; label: string }[] = [
	{ value: 5, label: "5 minutes" },
	{ value: 15, label: "15 minutes" },
	{ value: 30, label: "30 minutes" },
	{ value: 60, label: "1 hour" },
	{ value: 180, label: "3 hours" },
	{ value: 720, label: "12 hours" },
	{ value: 1440, label: "24 hours" },
];

export const DEFAULT_ALERT_WINDOW_MINUTES = 15;
export const DEFAULT_ALERT_SENSITIVITY = 3;
export const DEFAULT_ALERT_MIN_SAMPLES = 6;
export const MIN_ALERT_SAMPLES = 2;
export const MAX_ALERT_SAMPLES = 48;
export const MIN_ALERT_SENSITIVITY = 0.5;
export const MAX_ALERT_SENSITIVITY = 10;

export function formatAlertValue(
	metric: string,
	value?: number | null,
): string {
	if (value === null || value === undefined || !Number.isFinite(value)) {
		return "—";
	}
	const { unit } = alertMetricMeta(metric);
	if (unit === "ratio") return `${(value * 100).toFixed(2)}%`;
	if (unit === "ms") return formatDurationMs(value);
	return value.toLocaleString();
}

export function alertComparatorLabel(comparator: string) {
	return comparator === "lt" ? "below" : "above";
}

export function alertModeLabel(mode: string) {
	return (
		ALERT_MODE_OPTIONS.find((option) => option.value === mode)?.label ?? mode
	);
}

export function alertStatusLabel(status: string) {
	if (status === "triggered") return "Triggered";
	if (status === "resolved") return "Resolved";
	return status;
}

export interface AlertTone {
	chip: string;
	text: string;
	dot: string;
}

const ALERT_STATUS_TONES: Record<ITelemetryAlertStatus, AlertTone> = {
	triggered: {
		chip: "border-destructive/40 bg-destructive/5",
		text: "text-destructive",
		dot: "bg-destructive",
	},
	resolved: {
		chip: "border-emerald-500/40 bg-emerald-500/5",
		text: "text-emerald-600 dark:text-emerald-400",
		dot: "bg-emerald-500",
	},
};

const NEUTRAL_TONE: AlertTone = {
	chip: "border-border bg-muted/40",
	text: "text-muted-foreground",
	dot: "bg-muted-foreground",
};

export function alertStatusTone(status: string): AlertTone {
	return ALERT_STATUS_TONES[status as ITelemetryAlertStatus] ?? NEUTRAL_TONE;
}

export function alertRuleSummary(rule: ITelemetryAlertRule): string {
	const metric = alertMetricLabel(rule.metric);
	if (rule.mode === "anomaly") {
		const sensitivity = rule.sensitivity ?? DEFAULT_ALERT_SENSITIVITY;
		return `${metric} deviates by more than ${sensitivity}σ from the previous ${rule.minSamples} windows`;
	}
	const threshold = formatAlertValue(rule.metric, rule.threshold);
	return `${metric} ${alertComparatorLabel(rule.comparator)} ${threshold}`;
}
