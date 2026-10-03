import type {
	RunFailureCode,
	RunRejection,
} from "../../../../lib/device-management/event-run";
import type { FieldProblem, FormSupport } from "../../../../lib/event-form";
import type { DevicesT } from "../primitives/area-context";

/* The sentences of "Run now…" (design R2 §6.5). Codes never reach the screen; a device's own text is quoted as plain text. */

/** A type, not an interface: interpolation options need its implicit index signature. */
export type RunNames = {
	device: string;
	service: string;
};

/** Device text is shown as it came, never longer than this. */
export const DEVICE_TEXT_MAX = 480;

export function cap(text: string, max = DEVICE_TEXT_MAX): string {
	const chars = [...text];
	return chars.length > max ? `${chars.slice(0, max - 1).join("")}…` : text;
}

interface FailureExtra {
	fields?: readonly string[];
	took?: string;
}

type FailureCopy = (
	t: DevicesT,
	names: RunNames,
	extra: FailureExtra,
) => string;

const capName = (name: string) => cap(name, 64);

const FAILURE_COPY: Record<RunFailureCode, FailureCopy> = {
	flow_failed: (t, names) =>
		t(
			"devices:runNow.failure.flowFailed",
			"The flow failed. People with Logs can read why in {{service}}'s logs.",
			names,
		),
	invalid_fields: (t, names, extra) =>
		t(
			"devices:runNow.failure.invalidFields",
			"{{device}} refused these fields: {{fields}}.",
			{ ...names, fields: (extra.fields ?? []).map(capName).join(", ") },
		),
	cancelled: (t) => t("devices:runNow.failure.cancelled", "Stopped."),
	timed_out: (t, names, extra) =>
		extra.took
			? t("devices:runNow.failure.timedOut", "Stopped after {{limit}}.", {
					limit: extra.took,
				})
			: t(
					"devices:runNow.failure.timedOutLimit",
					"Stopped at {{service}}'s time limit.",
					names,
				),
	interrupted: (t, names) =>
		t(
			"devices:runNow.failure.interrupted",
			"{{service}} stopped, updated or restarted during the run. It isn't started again.",
			names,
		),
	not_started: (t, names) =>
		t(
			"devices:runNow.failure.notStarted",
			"No running instance of {{service}} took the run within 30 seconds.",
			names,
		),
	needs_interaction: (t) =>
		t(
			"devices:runNow.failure.needsInteraction",
			"The flow asked a question. That can't be answered from here.",
		),
};

/** One sentence per way a run can end without success. */
export function failureSentence(
	t: DevicesT,
	code: RunFailureCode | undefined,
	names: RunNames,
	extra: FailureExtra = {},
): string {
	return code
		? FAILURE_COPY[code](t, names, extra)
		: t("devices:runNow.failure.other", "The run didn't succeed.");
}

const REJECTION_COPY: Record<
	RunRejection,
	(t: DevicesT, names: RunNames) => string
> = {
	busy: (t, names) =>
		t(
			"devices:runNow.rejected.busy",
			"{{service}} is running as many actions as it accepts. Try again in a moment.",
			names,
		),
	revision_conflict: (t, names) =>
		t(
			"devices:runNow.rejected.revisionConflict",
			"{{service}} changed or isn't running. Reload the form.",
			names,
		),
	limit: (t, names) =>
		t(
			"devices:runNow.rejected.limit",
			"{{device}} has recorded as many of your commands today as it accepts. Try again later, or ask the device's owner.",
			names,
		),
	unauthorized: (t, names) =>
		t(
			"devices:runNow.rejected.unauthorized",
			"{{device}} doesn't let you start {{service}}, so you can't run its actions and forms.",
			names,
		),
	invalid: (t, names) =>
		t(
			"devices:runNow.rejected.invalid",
			"{{device}} refused the run: it isn't an action or form of {{service}} any more, or the inputs are larger than it takes.",
			names,
		),
	unsupported: (t, names) =>
		t(
			"devices:runNow.rejected.unsupported",
			"{{device}}'s agent can't run forms and quick actions. Update the device agent.",
			names,
		),
};

/** Why `run_event` was refused before anything was recorded; a refusal this client has no words for quotes the device. */
export function rejectionSentence(
	t: DevicesT,
	code: RunRejection | "other",
	names: RunNames,
	reason?: string,
): string {
	if (code !== "other") return REJECTION_COPY[code](t, names);
	return reason
		? t(
				"devices:runNow.rejected.reason",
				"{{device}} refused the run: “{{reason}}”",
				{ ...names, reason: cap(reason) },
			)
		: t("devices:runNow.rejected.other", "{{device}} refused the run.", names);
}

/** The line under a field the run can't send as it is. */
export function fieldProblemText(t: DevicesT, problem: FieldProblem): string {
	const texts: Record<FieldProblem, () => string> = {
		required: () => t("devices:runNow.field.required", "Enter a value."),
		integer: () => t("devices:runNow.field.integer", "Enter a whole number."),
		number: () => t("devices:runNow.field.number", "Enter a number."),
		date: () =>
			t("devices:runNow.field.date", "Enter a date, like 2026-10-03."),
		json: () => t("devices:runNow.field.json", "This isn't valid JSON."),
		object: () =>
			t("devices:runNow.field.object", 'Enter a JSON object, like {"key": 1}.'),
		array: () =>
			t("devices:runNow.field.array", "Enter a JSON list, like [1, 2]."),
		unique: () =>
			t("devices:runNow.field.unique", "Each value may appear only once."),
		items: () =>
			t(
				"devices:runNow.field.items",
				"A value in the list has a type this field doesn't take.",
			),
		option: () => t("devices:runNow.field.option", "Choose one of the values."),
		file: () =>
			t(
				"devices:runNow.field.file",
				"Files can only be sent from the service page.",
			),
		unknown: () =>
			t("devices:runNow.field.unknown", "This app can't show this field."),
	};
	return texts[problem]();
}

/** Why the form isn't offered here. */
export function supportSentence(
	t: DevicesT,
	reason: Extract<FormSupport, { ok: false }>["reason"],
): string {
	switch (reason) {
		case "file":
			return t(
				"devices:runNow.support.file",
				"This form takes a file. Open it on the service page.",
			);
		case "unknown_field":
			return t(
				"devices:runNow.support.unknownField",
				"This form has a field this app can't show. Update the app, or open it on the service page.",
			);
		case "truncated":
			return t(
				"devices:runNow.support.truncated",
				"This form has more fields than fit here. Open it on the service page.",
			);
	}
}

export function payloadProblemSentence(
	t: DevicesT,
	problem: "too_large" | "too_many",
): string {
	return problem === "too_many"
		? t("devices:runNow.payload.tooMany", "A run takes at most 64 fields.")
		: t(
				"devices:runNow.payload.tooLarge",
				"What you entered is larger than the 12 KiB a run takes. Shorten it.",
			);
}

/** "1.0.0". */
export const versionText = (version: readonly number[]) => version.join(".");
