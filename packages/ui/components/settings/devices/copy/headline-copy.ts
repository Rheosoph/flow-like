import type {
	Headline,
	HeadlineCode,
	HeadlineRef,
} from "../../../../lib/device-management/model/headline";
import type { DevicesT } from "../primitives/area-context";
import { type CopyTime, defaultCopyTime, formatNames } from "./attention-copy";

const NAME_CAP = 2;

interface PartContext {
	t: DevicesT;
	time: CopyTime;
	num(key: string): number;
	str(key: string): string;
	when(key: string): string;
	rel(key: string): string;
	names(list: string, cap?: number): string;
}

function partContext(
	t: DevicesT,
	part: HeadlineRef,
	time: CopyTime,
): PartContext {
	const p = part.params ?? {};
	return {
		t,
		time,
		num: (key) => (typeof p[key] === "number" ? (p[key] as number) : 0),
		str: (key) => (p[key] === undefined ? "" : String(p[key])),
		when: (key) =>
			typeof p[key] === "number" ? time.at(p[key] as number) : "",
		rel: (key) =>
			typeof p[key] === "number" ? time.ago(p[key] as number, "long") : "",
		names: (list, cap) =>
			formatNames(t, part.lists?.[list] ?? [], time.locale, cap),
	};
}

/**
 * The one coverage sentence (APP §6.3): "Status from 3 of 5 devices you can see;
 * 1 is unknown, 1 hasn't checked in yet and your access doesn't cover {app} on 1 more."
 */
function coverageSentence(c: PartContext): string {
	const readable = c.num("readable");
	const total = c.num("total");
	const states = [
		c.num("unknown") > 0
			? c.t("devices:headline.app.coverageUnknown", {
					count: c.num("unknown"),
					defaultValue_one: "{{count, number}} is unknown",
					defaultValue_other: "{{count, number}} are unknown",
				})
			: "",
		c.num("never") > 0
			? c.t("devices:headline.app.coverageNever", {
					count: c.num("never"),
					defaultValue_one: "{{count, number}} hasn't checked in yet",
					defaultValue_other: "{{count, number}} haven't checked in yet",
				})
			: "",
	]
		.filter(Boolean)
		.join(c.t("devices:headline.app.coverageComma", ", "));
	const access =
		c.num("noAccess") > 0
			? c.t(
					"devices:headline.app.coverageNoAccess",
					"your access doesn't cover {{app}} on {{more, number}} more",
					{ app: c.str("app"), more: c.num("noAccess") },
				)
			: "";
	const rest =
		states && access
			? c.t("devices:headline.app.coverageAnd", "{{states}} and {{access}}", {
					states,
					access,
				})
			: states || access;
	return rest
		? c.t(
				"devices:headline.app.coverageWith",
				"Status from {{readable, number}} of {{total, number}} devices you can see; {{rest}}.",
				{ readable, total, rest },
			)
		: c.t(
				"devices:headline.app.coverage",
				"Status from {{readable, number}} of {{total, number}} devices you can see.",
				{ readable, total },
			);
}

/** One entry per headline code (SPEC §6.4, APP §7.4). */
const PARTS = {
	"fleet.critical": (c) =>
		c.t("devices:headline.fleet.critical", {
			names: c.names("names", NAME_CAP),
			count: c.num("count"),
			defaultValue_one: "{{names}} needs you now.",
			defaultValue_other: "{{names}} need you now.",
		}),
	"fleet.critical_items": (c) =>
		c.t("devices:headline.fleet.criticalItems", {
			count: c.num("count"),
			defaultValue_one: "{{count, number}} thing needs you now.",
			defaultValue_other: "{{count, number}} things need you now.",
		}),
	"fleet.nothing_broken": (c) =>
		c.t("devices:headline.fleet.nothingBroken", "Nothing is broken."),
	"fleet.all_clear": (c) =>
		c.t("devices:headline.fleet.allClear", "Everything runs as you asked."),
	"fleet.soon_later": (c) => {
		const soon = c.num("soon");
		const later = c.num("later");
		if (soon > 0 && later > 0)
			return c.t("devices:headline.fleet.soonAndLater", {
				count: soon,
				later,
				defaultValue_one:
					"{{count, number}} thing needs a look soon and {{later, number}} can wait.",
				defaultValue_other:
					"{{count, number}} things need a look soon and {{later, number}} can wait.",
			});
		if (soon > 0)
			return c.t("devices:headline.fleet.soon", {
				count: soon,
				defaultValue_one: "{{count, number}} thing needs a look soon.",
				defaultValue_other: "{{count, number}} things need a look soon.",
			});
		return c.t("devices:headline.fleet.later", {
			count: later,
			defaultValue_one:
				"Nothing needs a look soon and {{count, number}} can wait.",
			defaultValue_other:
				"Nothing needs a look soon and {{count, number}} can wait.",
		});
	},
	"fleet.coverage": (c) =>
		c.num("locked") > 0
			? c.t("devices:headline.fleet.coverageLocked", {
					readable: c.num("readable"),
					active: c.num("active"),
					names: c.names("locked", NAME_CAP),
					count: c.num("locked"),
					defaultValue_one:
						"Status comes from {{readable, number}} of {{active, number}} active devices; {{names}} is locked.",
					defaultValue_other:
						"Status comes from {{readable, number}} of {{active, number}} active devices; {{names}} are locked.",
				})
			: c.t(
					"devices:headline.fleet.coverage",
					"Status comes from {{readable, number}} of {{active, number}} active devices.",
					{ readable: c.num("readable"), active: c.num("active") },
				),
	"app.no_devices": (c) =>
		c.t("devices:headline.app.noDevices", "You don't have any devices yet."),
	"app.no_devices_next": (c) =>
		c.t(
			"devices:headline.app.noDevicesNext",
			"Set one up, then deploy {{app}} to it.",
			{ app: c.str("app") },
		),
	"app.all_locked": (c) =>
		c.t("devices:headline.app.allLocked", "Unlock to see where {{app}} runs.", {
			app: c.str("app"),
		}),
	"app.locked_detail": (c) =>
		c.t("devices:headline.app.lockedDetail", {
			count: c.num("count"),
			defaultValue_one:
				"The hub doesn't know which apps run on your devices; only keys on this computer can read it. {{count, number}} device is locked.",
			defaultValue_other:
				"The hub doesn't know which apps run on your devices; only keys on this computer can read it. {{count, number}} devices are locked.",
		}),
	"app.never_deployed": (c) =>
		c.num("seen")
			? c.t(
					"devices:headline.app.neverDeployedSeen",
					"{{app}} isn't on any device you can see yet.",
					{ app: c.str("app") },
				)
			: c.t(
					"devices:headline.app.neverDeployed",
					"{{app}} isn't on any device yet.",
					{ app: c.str("app") },
				),
	"app.mode": (c) =>
		c.num("localOnly")
			? c.t(
					"devices:headline.app.modeLocalOnly",
					"It's a local-only app, so devices will get an offline copy from this computer. {{eligible, number}} of its {{total, number}} events can run on a device.",
					{ eligible: c.num("eligible"), total: c.num("total") },
				)
			: c.t(
					"devices:headline.app.modeOnline",
					"It's an online app, so devices will run it online with its data in the cloud. {{eligible, number}} of its {{total, number}} events can run on a device.",
					{ eligible: c.num("eligible"), total: c.num("total") },
				),
	"app.crashing": (c) =>
		c.t(
			"devices:headline.app.crashing",
			"{{service}} on {{device}} keeps crashing.",
			{
				service: c.str("service"),
				device: c.str("device"),
			},
		),
	"app.crashing_last_known": (c) =>
		c.t(
			"devices:headline.app.crashingLastKnown",
			"{{service}} on {{device}} kept crashing when last seen.",
			{ service: c.str("service"), device: c.str("device") },
		),
	"app.others_ok": (c) =>
		c.t("devices:headline.app.othersOk", {
			count: c.num("count"),
			defaultValue_one: "{{count, number}} other service runs as you asked.",
			defaultValue_other: "{{count, number}} other services run as you asked.",
		}),
	"app.offline_age": (c) =>
		c.t(
			"devices:headline.app.offlineAge",
			"{{device}} has been offline since {{time}} ({{age}}), so this is the last known state.",
			{ device: c.str("device"), time: c.when("since"), age: c.rel("since") },
		),
	"app.writes": (c) =>
		c.t(
			"devices:headline.app.writes",
			"Buffered changes from {{service}} on {{device}} need you.",
			{ service: c.str("service"), device: c.str("device") },
		),
	"app.writes_detail": (c) => {
		const conflicts = c.num("conflicts");
		const paused = c.num("paused");
		if (conflicts > 0 && paused > 0)
			return c.t("devices:headline.app.writesBoth", {
				count: conflicts,
				paused,
				defaultValue_one:
					"{{count, number}} change conflicts with newer cloud data and {{paused, number}} are paused because cloud access changed.",
				defaultValue_other:
					"{{count, number}} changes conflict with newer cloud data and {{paused, number}} are paused because cloud access changed.",
			});
		if (conflicts > 0)
			return c.t("devices:headline.app.writesConflicts", {
				count: conflicts,
				defaultValue_one:
					"{{count, number}} change conflicts with newer cloud data.",
				defaultValue_other:
					"{{count, number}} changes conflict with newer cloud data.",
			});
		return c.t("devices:headline.app.writesPaused", {
			count: paused,
			defaultValue_one:
				"{{count, number}} change is paused because cloud access changed.",
			defaultValue_other:
				"{{count, number}} changes are paused because cloud access changed.",
		});
	},
	"app.updating": (c) =>
		c.t(
			"devices:headline.app.updating",
			"{{service}} on {{device}} is switching to settings v{{to}}.",
			{ service: c.str("service"), device: c.str("device"), to: c.num("to") },
		),
	"app.updating_deadline": (c) =>
		c.t(
			"devices:headline.app.updatingDeadline",
			"If it isn't healthy by {{time}}, the device restores settings v{{from}} on its own.",
			{ time: c.when("deadlineAt"), from: c.num("from") },
		),
	"app.staged": (c) =>
		c.t(
			"devices:headline.app.staged",
			"An update for {{service}} on {{device}} is ready but not active.",
			{ service: c.str("service"), device: c.str("device") },
		),
	"app.staged_detail": (c) =>
		c.num("expiresAt") > 0
			? c.t(
					"devices:headline.app.stagedDetail",
					"Activate it to switch to settings v{{to}}; it's discarded on {{date}} otherwise.",
					{ to: c.num("to"), date: c.when("expiresAt") },
				)
			: c.t(
					"devices:headline.app.stagedDetailUndated",
					"Activate it to switch to settings v{{to}}.",
					{ to: c.num("to") },
				),
	"app.applying": (c) =>
		c.t(
			"devices:headline.app.applying",
			"{{service}} on {{device}} is applying changes.",
			{ service: c.str("service"), device: c.str("device") },
		),
	"app.unknown_state": (c) =>
		c.t(
			"devices:headline.app.unknownState",
			"{{service}} on {{device}} is in an unknown state.",
			{ service: c.str("service"), device: c.str("device") },
		),
	"app.stopped_events_nowhere": (c) =>
		c.t(
			"devices:headline.app.stoppedAsAsked",
			"{{service}} on {{device}} is stopped, as you asked.",
			{ service: c.str("service"), device: c.str("device") },
		),
	"app.events_nowhere": (c) =>
		c.t("devices:headline.app.eventsNowhere", {
			events: c.names("events"),
			count: c.num("count"),
			defaultValue_one: "{{events}} isn't on any device.",
			defaultValue_other: "{{events}} aren't on any device.",
		}),
	"app.upload_paused": (c) =>
		c.t(
			"devices:headline.app.uploadPaused",
			"An upload to {{device}} is paused at {{done, number}} of {{total, number}} files.",
			{ device: c.str("device"), done: c.num("done"), total: c.num("total") },
		),
	"app.all_as_asked": (c) =>
		c.t("devices:headline.app.allAsAsked", {
			app: c.str("app"),
			count: c.num("count"),
			defaultValue_one:
				"{{app}} runs as you asked on {{count, number}} device.",
			defaultValue_other:
				"{{app}} runs as you asked on {{count, number}} devices.",
		}),
	"app.all_stopped": (c) =>
		c.t("devices:headline.app.allStopped", {
			app: c.str("app"),
			count: c.num("count"),
			defaultValue_one:
				"{{app}} is stopped on {{count, number}} device, as you asked.",
			defaultValue_other:
				"{{app}} is stopped on {{count, number}} devices, as you asked.",
		}),
	"app.drift": (c) =>
		c.t("devices:headline.app.drift", {
			version: c.str("version"),
			count: c.num("count"),
			defaultValue_one:
				"{{count, number}} service runs an older version than {{version}}.",
			defaultValue_other:
				"{{count, number}} services run an older version than {{version}}.",
		}),
	"app.coverage": coverageSentence,
	"app.not_checked_in": (c) =>
		c.t("devices:headline.app.notCheckedIn", {
			names: c.names("names", NAME_CAP),
			count: c.num("count"),
			defaultValue_one: "{{names}} hasn't checked in yet.",
			defaultValue_other: "{{names}} haven't checked in yet.",
		}),
} satisfies Record<HeadlineCode, (c: PartContext) => string>;

export function headlinePartCopy(
	t: DevicesT,
	part: HeadlineRef,
	time: CopyTime = defaultCopyTime(),
): string {
	return PARTS[part.code](partContext(t, part, time));
}

const NAME_PARAMS = ["device", "service"] as const;
const NAME_LISTS = ["names", "locked"] as const;

const isName = (value: unknown): value is string =>
	typeof value === "string" && value !== "";

function partNames(part: HeadlineRef): string[] {
	const params = NAME_PARAMS.map((key) => part.params?.[key]);
	const lists = NAME_LISTS.flatMap((key) => part.lists?.[key] ?? []);
	return [...params, ...lists].filter(isName);
}

/** The device and service names a headline's sentences contain, for `Headline`'s `names` (set in mono, R15). */
export function headlineNames(headline: Headline): string[] {
	return [...new Set([headline, ...headline.rest].flatMap(partNames))];
}

/** `{ lead, rest }` for the `Headline` primitive. */
export function headlineCopy(
	t: DevicesT,
	headline: Headline,
	time: CopyTime = defaultCopyTime(),
): { lead: string; rest: string } {
	return {
		lead: headlinePartCopy(t, headline, time),
		rest: headline.rest
			.map((part) => headlinePartCopy(t, part, time))
			.filter(Boolean)
			.join(" "),
	};
}
