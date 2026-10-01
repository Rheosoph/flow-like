import { formatMoment, formatRelativeTime } from "../../../../lib/date";
import type {
	AttentionActionCode,
	AttentionItem,
	AttentionKey,
	CopyParams,
} from "../../../../lib/device-management/model/types";
import { humanFileSize } from "../../../../lib/utils";
import type { AreaTime, DevicesT } from "../primitives/area-context";
import { type EnumValues, enumLabel } from "./enum-labels";

/** Satisfied by `useAreaTime()`; without one, times use `Date.now()` and the default locale. */
export type CopyTime = Pick<AreaTime, "now" | "locale" | "ago" | "at">;

export interface AttentionCopyContext {
	time?: CopyTime;
	/** Display name for an account id (people directory); the id otherwise. */
	personName?(userId: string): string | undefined;
	/** Display name for an app id; the id otherwise. */
	appName?(appId: string): string | undefined;
}

export function defaultCopyTime(): CopyTime {
	const now = Date.now();
	return {
		now,
		locale: "en",
		ago: (atS, style = "long") =>
			formatRelativeTime(atS * 1000, style, "", { now }),
		at: (atS) => formatMoment(atS * 1000, { now }),
	};
}

/** Locale list with a cap: "a, b and 3 others". */
export function formatNames(
	t: DevicesT,
	names: readonly string[],
	locale: string,
	cap = names.length,
): string {
	const shown = names.slice(0, cap);
	const more = names.length - shown.length;
	const items =
		more > 0
			? [
					...shown,
					t("devices:headline.others", {
						count: more,
						defaultValue_one: "{{count, number}} other",
						defaultValue_other: "{{count, number}} others",
					}),
				]
			: shown;
	return new Intl.ListFormat(locale, { type: "conjunction" }).format(items);
}

export function formatMoney(micros: number, locale: string): string {
	return new Intl.NumberFormat(locale, {
		style: "currency",
		currency: "EUR",
	}).format(micros / 1_000_000);
}

interface SentenceContext {
	t: DevicesT;
	p: CopyParams;
	item: AttentionCopyItem;
	time: CopyTime;
	device: string;
	service: string;
	person: string;
	personNameOf(userId: string): string;
	app(id: string | number | undefined): string;
	str(key: string): string | undefined;
	num(key: string): number;
	/** "11:00" / "29 Sept, 11:00" for a unix-seconds param. */
	when(key: string): string;
	/** "3 hours ago" / "in 6 hours" for a unix-seconds param. */
	rel(key: string): string;
	resource(): string;
	scope(): string;
}

export type AttentionCopyItem = Pick<
	AttentionItem,
	"key" | "copy" | "lastKnown" | "firstSeenAt" | "action" | "secondary"
>;

type ParamReaders = Pick<SentenceContext, "str" | "num" | "when" | "rel">;

const paramReaders = (p: CopyParams, time: CopyTime): ParamReaders => {
	const numeric = (key: string) =>
		typeof p[key] === "number" ? (p[key] as number) : undefined;
	return {
		str: (key) => (typeof p[key] === "string" ? (p[key] as string) : undefined),
		num: (key) => numeric(key) ?? 0,
		when: (key) => {
			const at = numeric(key);
			return at === undefined ? "" : time.at(at);
		},
		rel: (key) => {
			const at = numeric(key);
			return at === undefined ? "" : time.ago(at, "long");
		},
	};
};

const resourceLabel = (t: DevicesT, readers: ParamReaders) => {
	const name = readers.str("resource") ?? "";
	switch (readers.str("resourceKind")) {
		case "table":
			return t("devices:attention.resource.table", "table {{name}}", { name });
		case "file":
			return t("devices:attention.resource.file", "file {{name}}", { name });
		default:
			return name;
	}
};

const scopeLabel = (
	t: DevicesT,
	scope: string | undefined,
	app: SentenceContext["app"],
) =>
	!scope || scope === "device"
		? t("devices:attention.scope.device", "the whole device")
		: app(scope);

const nameLookups = (ctx: AttentionCopyContext) => ({
	app: (id: string | number | undefined) =>
		id === undefined ? "" : (ctx.appName?.(String(id)) ?? String(id)),
	personNameOf: (userId: string) => ctx.personName?.(userId) ?? userId,
});

function sentenceContext(
	t: DevicesT,
	item: AttentionCopyItem,
	ctx: AttentionCopyContext,
): SentenceContext {
	const p = item.copy.params ?? {};
	const time = ctx.time ?? defaultCopyTime();
	const readers = paramReaders(p, time);
	const { app, personNameOf } = nameLookups(ctx);
	const personId = readers.str("person");
	return {
		t,
		p,
		item,
		time,
		device:
			readers.str("device") ?? t("devices:attention.thisDevice", "this device"),
		service:
			readers.str("service") ??
			t("devices:attention.thisService", "this service"),
		person: personId ? personNameOf(personId) : "",
		personNameOf,
		app,
		...readers,
		resource: () => resourceLabel(t, readers),
		scope: () => scopeLabel(t, readers.str("scope"), app),
	};
}

function known<F extends keyof EnumValues>(
	values: readonly string[],
	value: string | undefined,
): value is EnumValues[F] & string {
	return value !== undefined && values.includes(value);
}

const OBSERVED = [
	"unknown",
	"starting",
	"running",
	"stopping",
	"stopped",
	"backoff",
	"failed",
	"removed",
];
const FAILURE_CODES = [
	"discarded",
	"superseded",
	"stopped",
	"staging_timeout",
	"validation_failed",
	"validation_timeout",
	"activation_timeout",
	"candidate_failed",
	"rollback_timeout",
	"rollback_failed",
];
const ROLLOUT_STATES = [
	"staged",
	"validating",
	"activating",
	"healthy",
	"rolling_back",
	"rolled_back",
	"failed",
	"cancelled",
];
const READINESS_CHECKS = [
	"policy",
	"signing",
	"api",
	"signaling",
	"release",
	"database",
];

function observedLabel(c: SentenceContext, key: string) {
	const value = c.str(key);
	return known<"observed">(OBSERVED, value)
		? enumLabel(c.t, "observed", value)
		: (value ?? "");
}

function desiredLabel(c: SentenceContext) {
	const value = c.str("desired");
	return value === "running" || value === "stopped"
		? enumLabel(c.t, "desired", value)
		: (value ?? "");
}

function failureLabel(c: SentenceContext) {
	const value = c.str("failure");
	if (known<"failureCode">(FAILURE_CODES, value))
		return enumLabel(c.t, "failureCode", value);
	return value ?? c.t("devices:attention.unknownReason", "reason unknown");
}

function renewalSentence(c: SentenceContext): string {
	const { t } = c;
	switch (c.str("renewal")) {
		case "authority_expired":
			return t(
				"devices:attention.renewal.authorityExpired",
				"It can't renew itself: the authority that renews it expired at {{time}}.",
				{ time: c.when("renewalAt") },
			);
		case "authority_expiring":
			return t(
				"devices:attention.renewal.authorityExpiring",
				"The authority that renews it expires {{date}}.",
				{ date: c.when("renewalAt") },
			);
		case "delegation_error":
			return t(
				"devices:attention.renewal.delegationError",
				"It can't renew itself: automatic renewal failed.",
			);
		case "acme_error":
			return t(
				"devices:attention.renewal.acmeError",
				"Let's Encrypt couldn't renew it. Next attempt {{time}}.",
				{ time: c.when("renewalAt") },
			);
		default:
			return typeof c.p.renewsAt === "number"
				? t(
						"devices:attention.renewal.automatic",
						"It will renew automatically on {{date}}.",
						{ date: c.when("renewsAt") },
					)
				: "";
	}
}

const join = (...parts: string[]) => parts.filter(Boolean).join(" ");

/** One entry per IA §6.5 condition key; every key reaches a literal t() call. */
const SENTENCES = {
	offline_since: (c) =>
		join(
			c.t(
				"devices:attention.offline_since.sentence",
				"{{device}} hasn't checked in since {{time}} ({{age}}).",
				{ device: c.device, time: c.when("since"), age: c.rel("since") },
			),
			c.num("running") > 0
				? c.t("devices:attention.offline_since.running", {
						count: c.num("running"),
						defaultValue_one:
							"Its last known status had {{count, number}} service requested running.",
						defaultValue_other:
							"Its last known status had {{count, number}} services requested running.",
					})
				: "",
		),
	late: (c) =>
		c.t(
			"devices:attention.late.sentence",
			"{{device}} missed a check-in (last seen {{age}}).",
			{ device: c.device, age: c.rel("since") },
		),
	no_heartbeat_since_enrollment: (c) =>
		c.t(
			"devices:attention.no_heartbeat_since_enrollment.sentence",
			"{{device}} was registered {{age}} but has never checked in.",
			{ device: c.device, age: c.rel("registeredAt") },
		),
	revoked: (c) =>
		typeof c.p.revokedAt === "number"
			? c.t(
					"devices:attention.revoked.sentenceDated",
					"{{device}} was revoked on {{date}}.",
					{ device: c.device, date: c.when("revokedAt") },
				)
			: c.t("devices:attention.revoked.sentence", "{{device}} was revoked.", {
					device: c.device,
				}),
	you_still_pay_for_a_revoked_device: (c) =>
		c.str("kind") === "billing"
			? c.t(
					"devices:attention.you_still_pay_for_a_revoked_device.sentence",
					"You still pay for a spending limit on {{device}}, which has been revoked: {{used}} of {{limit}} used, active until {{date}}.",
					{
						device: c.device,
						used: formatMoney(c.num("usedMicros"), c.time.locale),
						limit: formatMoney(c.num("limitMicros"), c.time.locale),
						date: c.when("expiresAt"),
					},
				)
			: c.t(
					"devices:attention.you_still_pay_for_a_revoked_device.approval",
					"You still have active cloud access on {{device}}, which has been revoked.",
					{ device: c.device },
				),
	identity_mismatch: (c) =>
		c.t(
			"devices:attention.identity_mismatch.sentence",
			"The hub reports different keys for {{device}} than the ones you trusted on {{date}}. Management is blocked.",
			{ device: c.device, date: c.when("pinnedAt") },
		),
	snapshot_integrity_error: (c) =>
		c.t(
			"devices:attention.snapshot_integrity_error.sentence",
			"The hub served older or altered status for {{device}}. It was ignored.",
			{ device: c.device },
		),
	clock_skew: (c) =>
		c.str("device")
			? c.t("devices:attention.clock_skew.device", {
					device: c.device,
					count: c.num("minutes"),
					defaultValue_one:
						"{{device}}'s clock looks {{count, number}} minute off. Check-ins and connections can fail.",
					defaultValue_other:
						"{{device}}'s clock looks {{count, number}} minutes off. Check-ins and connections can fail.",
				})
			: c.t("devices:attention.clock_skew.computer", {
					count: c.num("minutes"),
					defaultValue_one:
						"This computer's clock is {{count, number}} minute off.",
					defaultValue_other:
						"This computer's clock is {{count, number}} minutes off.",
				}),
	access_denied: (c) =>
		c.t(
			"devices:attention.access_denied.sentence",
			"{{device}} stopped checking in after the hub refused it. Run recover-enrollment on the device.",
			{ device: c.device },
		),
	status_stale_while_online: (c) =>
		c.t(
			"devices:attention.status_stale_while_online.sentence",
			"{{device}} is online, but its encrypted status was last updated {{age}}.",
			{ device: c.device, age: c.rel("observedAt") },
		),
	background_task_failing: (c) =>
		c.num("more") > 0
			? c.t("devices:attention.background_task_failing.more", {
					device: c.device,
					task: c.str("task"),
					count: c.num("more"),
					defaultValue_one:
						"Background work on {{device}} is failing: {{task}} and {{count, number}} more.",
					defaultValue_other:
						"Background work on {{device}} is failing: {{task}} and {{count, number}} more.",
				})
			: c.t(
					"devices:attention.background_task_failing.sentence",
					"Background work on {{device}} is failing: {{task}}.",
					{ device: c.device, task: c.str("task") },
				),
	agent_update_available: (c) =>
		c.item.lastKnown
			? c.t(
					"devices:attention.agent_update_available.lastKnown",
					"Agent {{available}} is available for {{device}}. It ran {{running}} when last read live.",
					{
						device: c.device,
						available: c.str("available"),
						running: c.str("running"),
					},
				)
			: c.t(
					"devices:attention.agent_update_available.sentence",
					"Agent {{available}} is available for {{device}} (running {{running}}).",
					{
						device: c.device,
						available: c.str("available"),
						running: c.str("running"),
					},
				),
	rebooted_unexpectedly: (c) =>
		typeof c.p.bootedAt === "number"
			? c.t(
					"devices:attention.rebooted_unexpectedly.sentence",
					"{{device}} restarted at {{time}}.",
					{ device: c.device, time: c.when("bootedAt") },
				)
			: c.t(
					"devices:attention.rebooted_unexpectedly.undated",
					"{{device}} restarted.",
					{ device: c.device },
				),
	status_subscription_expiring: (c) =>
		c.t(
			"devices:attention.status_subscription_expiring.sentence",
			"Encrypted status for {{device}} stops reaching this computer on {{date}}.",
			{ device: c.device, date: c.when("expiresAt") },
		),
	device_slots_nearly_full: (c) =>
		c.num("pending") > 0
			? c.t("devices:attention.device_slots_nearly_full.withPending", {
					used: c.num("used"),
					max: c.num("max"),
					count: c.num("pending"),
					defaultValue_one:
						"You're using {{used, number}} of {{max, number}} device slots, including {{count, number}} unused setup package.",
					defaultValue_other:
						"You're using {{used, number}} of {{max, number}} device slots, including {{count, number}} unused setup packages.",
				})
			: c.t(
					"devices:attention.device_slots_nearly_full.sentence",
					"You're using {{used, number}} of {{max, number}} device slots.",
					{ used: c.num("used"), max: c.num("max") },
				),
	pending_setup_waiting: (c) =>
		c.t(
			"devices:attention.pending_setup_waiting.sentence",
			"The setup package for {{name}} is waiting to be started. It expires {{in}}.",
			{ name: c.str("name"), in: c.rel("expiresAt") },
		),
	pending_setup_expired: (c) =>
		c.t(
			"devices:attention.pending_setup_expired.sentence",
			"The setup package for {{name}} expired unused on {{date}} and no longer works.",
			{ name: c.str("name"), date: c.when("expiredAt") },
		),
	hub_not_ready: (c) => {
		const check = c.str("check");
		return known<"readinessCheck">(READINESS_CHECKS, check)
			? c.t(
					"devices:attention.hub_not_ready.sentence",
					"This hub can't set up devices right now: {{check}}.",
					{ check: enumLabel(c.t, "readinessCheck", check) },
				)
			: c.t(
					"devices:attention.hub_not_ready.unknown",
					"This hub can't set up devices right now.",
				);
	},
	release_trust_missing: (c) =>
		c.t(
			"devices:attention.release_trust_missing.sentence",
			"This hub has no signed agent releases, so agents can't be updated remotely.",
		),
	keys_missing_here: (c) =>
		c.t(
			"devices:attention.keys_missing_here.sentence",
			"This computer has no keys for {{device}}.",
			{ device: c.device },
		),
	keys_not_backed_up_to_account: (c) =>
		c.t(
			"devices:attention.keys_not_backed_up_to_account.sentence",
			"The keys for {{device}} exist only on this computer. If you lose this computer, you can't manage the device.",
			{ device: c.device },
		),
	account_backup_upload_pending: (c) =>
		c.t(
			"devices:attention.account_backup_upload_pending.sentence",
			"The latest backup of {{device}}'s keys (version {{revision, number}}) hasn't reached your account yet.",
			{ device: c.device, revision: c.num("revision") },
		),
	account_backup_old_password: (c) =>
		c.t(
			"devices:attention.account_backup_old_password.sentence",
			"Your account backup for {{device}} still opens with your old device password.",
			{ device: c.device },
		),
	account_backup_hub_newer: (c) =>
		c.t(
			"devices:attention.account_backup_hub_newer.sentence",
			"Your account has a newer backup of {{device}}'s keys than this computer.",
			{ device: c.device },
		),
	storage_not_persistent: (c) =>
		c.t("devices:attention.storage_not_persistent.sentence", {
			count: c.num("count"),
			defaultValue_one:
				"This browser may delete the keys for {{count, number}} device.",
			defaultValue_other:
				"This browser may delete the keys for {{count, number}} devices.",
		}),
	request_keys_unbacked: (c) =>
		c.t(
			"devices:attention.request_keys_unbacked.sentence",
			"Your request for {{device}} isn't approved yet. Its keys exist only on this computer.",
			{ device: c.device },
		),
	backup_slots_nearly_full: (c) =>
		c.t(
			"devices:attention.backup_slots_nearly_full.sentence",
			"You're using {{used, number}} of {{max, number}} account backup slots.",
			{ used: c.num("used"), max: c.num("max") },
		),
	stale_local_keys: (c) =>
		c.t(
			"devices:attention.stale_local_keys.sentence",
			"Keys for {{device}} are stored here but can't be used any more.",
			{ device: c.device },
		),
	shared_access_expiring: (c) =>
		c.t(
			"devices:attention.shared_access_expiring.sentence",
			"Your access to {{device}} ends {{in}}.",
			{ device: c.device, in: c.rel("expiresAt") },
		),
	shared_access_ended: (c) =>
		c.t(
			"devices:attention.shared_access_ended.sentence",
			"Your access to {{device}} has ended.",
			{ device: c.device },
		),
	grant_expiring: (c) =>
		c.t(
			"devices:attention.grant_expiring.sentence",
			"{{person}}'s access to {{device}} ends {{in}} ({{time}}).",
			{
				person: c.person,
				device: c.device,
				in: c.rel("expiresAt"),
				time: c.when("expiresAt"),
			},
		),
	sharing_policy_waiting_for_device: (c) =>
		c.t(
			"devices:attention.sharing_policy_waiting_for_device.sentence",
			"{{device}} hasn't applied your access change yet. You saved access rules v{{version}}; the device still uses v{{applied}}.",
			{
				device: c.device,
				version: c.num("version"),
				applied: c.num("applied"),
			},
		),
	sharing_policy_expiring: (c) =>
		c.t(
			"devices:attention.sharing_policy_expiring.sentence",
			"Access rules on {{device}} expire {{date}}. Shared access and retained history stop then.",
			{ device: c.device, date: c.when("expiresAt") },
		),
	sharing_policy_expired: (c) =>
		c.t(
			"devices:attention.sharing_policy_expired.sentence",
			"Access rules on {{device}} expired {{date}}. Shared access has ended and retained history is paused.",
			{ device: c.device, date: c.when("expiredAt") },
		),
	access_slots_nearly_full: (c) =>
		c.t(
			"devices:attention.access_slots_nearly_full.sentence",
			"{{device}} uses {{used, number}} of {{max, number}} access slots.",
			{ device: c.device, used: c.num("used"), max: c.num("max") },
		),
	access_request_pending: (c) => {
		const owner = c.str("owner");
		return owner
			? c.t(
					"devices:attention.access_request_pending.sentence",
					"Waiting for {{owner}} to approve your access to {{device}}.",
					{ owner: c.personNameOf(owner), device: c.device },
				)
			: c.t(
					"devices:attention.access_request_pending.anyOwner",
					"Waiting for the owner to approve your access to {{device}}.",
					{ device: c.device },
				);
	},
	code_running_access_without_sandbox: (c) =>
		c.t(
			"devices:attention.code_running_access_without_sandbox.sentence",
			"{{person}} can run code on {{device}} with the agent's full access.",
			{ person: c.person, device: c.device },
		),
	service_crash_looping: (c) => {
		const values = {
			service: c.service,
			device: c.device,
			ready: c.num("ready"),
			requested: c.num("requested"),
		};
		const error = c.str("lastError");
		return join(
			c.item.lastKnown
				? c.t(
						"devices:attention.service_crash_looping.lastKnown",
						"{{service}} on {{device}} kept crashing when last seen: {{ready, number}} of {{requested, number}} instances ready.",
						values,
					)
				: c.t(
						"devices:attention.service_crash_looping.sentence",
						"{{service}} on {{device}} keeps crashing: {{ready, number}} of {{requested, number}} instances ready.",
						values,
					),
			error
				? c.t(
						"devices:attention.service_crash_looping.lastError",
						"Last error: {{error}}",
						{ error },
					)
				: c.item.lastKnown
					? c.t(
							"devices:attention.service_crash_looping.reasonOnDevice",
							"The reason is only on the device.",
						)
					: "",
		);
	},
	service_not_as_requested: (c) =>
		c.t(
			"devices:attention.service_not_as_requested.sentence",
			"{{service}} on {{device}} hasn't been as you asked since {{time}} (requested: {{desired}}, actual: {{observed}}).",
			{
				service: c.service,
				device: c.device,
				time: c.time.at(c.item.firstSeenAt),
				desired: desiredLabel(c),
				observed: observedLabel(c, "observed"),
			},
		),
	service_settings_not_applied: (c) =>
		c.t(
			"devices:attention.service_settings_not_applied.sentence",
			"{{service}} on {{device}} still runs settings v{{applied}}; v{{latest}} hasn't been applied.",
			{
				service: c.service,
				device: c.device,
				applied: c.num("applied"),
				latest: c.num("latest"),
			},
		),
	service_degraded: (c) =>
		c.t(
			"devices:attention.service_degraded.sentence",
			"{{service}} on {{device}} has {{ready, number}} of {{requested, number}} instances ready.",
			{
				service: c.service,
				device: c.device,
				ready: c.num("ready"),
				requested: c.num("requested"),
			},
		),
	rollout_in_progress: (c) => {
		const state = c.str("state");
		return c.t(
			"devices:attention.rollout_in_progress.sentence",
			"Updating {{service}} on {{device}}: {{state}}.",
			{
				service: c.service,
				device: c.device,
				state: known<"rollout">(ROLLOUT_STATES, state)
					? enumLabel(c.t, "rollout", state)
					: (state ?? ""),
			},
		);
	},
	rollout_failed_service_stopped: (c) =>
		c.t(
			"devices:attention.rollout_failed_service_stopped.sentence",
			"The update to {{service}} on {{device}} failed and the previous version couldn't be restored. The service is stopped.",
			{ service: c.service, device: c.device },
		),
	rollout_rolled_back: (c) =>
		c.t(
			"devices:attention.rollout_rolled_back.sentence",
			"The update to {{service}} on {{device}} was rolled back: {{reason}}.",
			{ service: c.service, device: c.device, reason: failureLabel(c) },
		),
	rollout_not_applied: (c) =>
		c.t(
			"devices:attention.rollout_not_applied.sentence",
			"The update to {{service}} on {{device}} wasn't applied: {{reason}}. The current version is still running.",
			{ service: c.service, device: c.device, reason: failureLabel(c) },
		),
	rollout_staged_waiting: (c) =>
		typeof c.p.deadlineAt === "number"
			? c.t(
					"devices:attention.rollout_staged_waiting.sentence",
					"An update for {{service}} on {{device}} is ready but not activated. It expires {{in}}.",
					{ service: c.service, device: c.device, in: c.rel("deadlineAt") },
				)
			: c.t(
					"devices:attention.rollout_staged_waiting.undated",
					"An update for {{service}} on {{device}} is ready but not activated.",
					{ service: c.service, device: c.device },
				),
	secret_write_pending: (c) =>
		c.str("secret")
			? c.t(
					"devices:attention.secret_write_pending.sentence",
					"{{device}} hasn't confirmed the new {{secret}} for {{service}} yet.",
					{ device: c.device, secret: c.str("secret"), service: c.service },
				)
			: c.t(
					"devices:attention.secret_write_pending.unnamed",
					"{{device}} hasn't confirmed the new secret for {{service}} yet.",
					{ device: c.device, service: c.service },
				),
	secret_write_failed: (c) =>
		c.str("secret")
			? c.t(
					"devices:attention.secret_write_failed.sentence",
					"Saving {{secret}} for {{service}} on {{device}} failed.",
					{ secret: c.str("secret"), service: c.service, device: c.device },
				)
			: c.t(
					"devices:attention.secret_write_failed.unnamed",
					"Saving a secret for {{service}} on {{device}} failed.",
					{ service: c.service, device: c.device },
				),
	endpoint_unencrypted_exposed: (c) =>
		c.t(
			"devices:attention.endpoint_unencrypted_exposed.sentence",
			"{{service}} on {{device}} serves its web page without encryption on {{address}}:{{port}}.",
			{
				service: c.service,
				device: c.device,
				address: c.str("address"),
				port: c.num("port"),
			},
		),
	event_tokens_after_revoke: (c) =>
		c.t(
			"devices:attention.event_tokens_after_revoke.sentence",
			"Access tokens deployed to revoked {{device}} may still be valid. Rotate them.",
			{ device: c.device },
		),
	offline_writes_conflict: (c) =>
		join(
			c.t(
				"devices:attention.offline_writes_conflict.sentence",
				"A queued change to {{resource}} from {{service}} on {{device}} conflicts with newer cloud data.",
				{ resource: c.resource(), service: c.service, device: c.device },
			),
			c.num("behind") > 0
				? c.t("devices:attention.offline_writes_conflict.behind", {
						count: c.num("behind"),
						defaultValue_one: "{{count, number}} more change waits behind it.",
						defaultValue_other:
							"{{count, number}} more changes wait behind it.",
					})
				: "",
		),
	offline_writes_blocked: (c) =>
		c.str("error")
			? c.t(
					"devices:attention.offline_writes_blocked.sentence",
					"Queued changes from {{service}} on {{device}} are stuck at {{resource}}: {{error}}",
					{
						service: c.service,
						device: c.device,
						resource: c.resource(),
						error: c.str("error"),
					},
				)
			: c.t(
					"devices:attention.offline_writes_blocked.noError",
					"Queued changes from {{service}} on {{device}} are stuck at {{resource}}.",
					{ service: c.service, device: c.device, resource: c.resource() },
				),
	offline_writes_outcome_unknown: (c) =>
		c.t(
			"devices:attention.offline_writes_outcome_unknown.sentence",
			"We can't tell whether a queued change to {{resource}} from {{service}} reached the cloud.",
			{ resource: c.resource(), service: c.service },
		),
	offline_writes_quarantined: (c) =>
		typeof c.p.oldestAt === "number"
			? c.t("devices:attention.offline_writes_quarantined.sentence", {
					service: c.service,
					device: c.device,
					count: c.num("count"),
					date: c.when("oldestAt"),
					defaultValue_one:
						"Queued changes from {{service}} on {{device}} are paused because its cloud access changed. {{count, number}} change is kept, from {{date}}.",
					defaultValue_other:
						"Queued changes from {{service}} on {{device}} are paused because its cloud access changed. {{count, number}} changes are kept, the oldest from {{date}}.",
				})
			: c.t("devices:attention.offline_writes_quarantined.undated", {
					service: c.service,
					device: c.device,
					count: c.num("count"),
					defaultValue_one:
						"Queued changes from {{service}} on {{device}} are paused because its cloud access changed. {{count, number}} change is kept.",
					defaultValue_other:
						"Queued changes from {{service}} on {{device}} are paused because its cloud access changed. {{count, number}} changes are kept.",
				}),
	offline_writes_backlog: (c) =>
		c.t("devices:attention.offline_writes_backlog.sentence", {
			service: c.service,
			count: c.num("count"),
			time: c.when("oldestAt"),
			defaultValue_one:
				"{{count, number}} change from {{service}} is waiting to reach the cloud (since {{time}}).",
			defaultValue_other:
				"{{count, number}} changes from {{service}} are waiting to reach the cloud (oldest {{time}}).",
		}),
	offline_mirror_error: (c) =>
		c.t(
			"devices:attention.offline_mirror_error.sentence",
			"The local copy of cloud data for {{service}} on {{device}} couldn't refresh.",
			{ service: c.service, device: c.device },
		),
	cloud_access_invalid: (c) =>
		c.str("reason") === "revoked"
			? c.t(
					"devices:attention.cloud_access_invalid.revoked",
					"{{service}} on {{device}} uses cloud access that was revoked. Its cloud calls fail.",
					{ service: c.service, device: c.device },
				)
			: c.t(
					"devices:attention.cloud_access_invalid.expired",
					"{{service}} on {{device}} uses cloud access that expired. Its cloud calls fail.",
					{ service: c.service, device: c.device },
				),
	cloud_access_ending: (c) =>
		c.t(
			"devices:attention.cloud_access_ending.sentence",
			"Cloud access for {{service}} ends {{in}} ({{date}}).",
			{ service: c.service, in: c.rel("expiresAt"), date: c.when("expiresAt") },
		),
	spending_limit_low: (c) =>
		c.t(
			"devices:attention.spending_limit_low.sentence",
			"{{service}} has used {{used}} of its {{limit}} spending limit.",
			{
				service: c.service,
				used: formatMoney(c.num("usedMicros"), c.time.locale),
				limit: formatMoney(c.num("limitMicros"), c.time.locale),
			},
		),
	spending_limit_ending: (c) =>
		c.t(
			"devices:attention.spending_limit_ending.sentence",
			"The spending limit for {{service}} ends {{in}} ({{date}}).",
			{ service: c.service, in: c.rel("expiresAt"), date: c.when("expiresAt") },
		),
	online_files_read_only: (c) =>
		c.t(
			"devices:attention.online_files_read_only.sentence",
			"{{service}} can only read project files because project storage is full.",
			{ service: c.service },
		),
	certificate_expired: (c) =>
		join(
			c.str("label")
				? c.t(
						"devices:attention.certificate_expired.sentence",
						"Certificate {{label}} on {{device}} expired {{date}}.",
						{
							label: c.str("label"),
							device: c.device,
							date: c.when("expiredAt"),
						},
					)
				: c.t(
						"devices:attention.certificate_expired.unnamed",
						"A certificate on {{device}} expired on {{date}} (ID {{id}}). Its name shows once the device is read live.",
						{
							device: c.device,
							date: c.when("expiredAt"),
							id: c.str("certificateId"),
						},
					),
			c.num("services") > 0
				? c.t("devices:attention.certificate_expired.services", {
						count: c.num("services"),
						defaultValue_one: "{{count, number}} service uses it.",
						defaultValue_other: "{{count, number}} services use it.",
					})
				: "",
			renewalSentence(c),
		),
	certificate_expiring: (c) =>
		join(
			c.str("label")
				? c.t("devices:attention.certificate_expiring.sentence", {
						label: c.str("label"),
						device: c.device,
						count: c.num("days"),
						date: c.when("expiresAt"),
						defaultValue_one:
							"Certificate {{label}} on {{device}} expires in {{count, number}} day ({{date}}).",
						defaultValue_other:
							"Certificate {{label}} on {{device}} expires in {{count, number}} days ({{date}}).",
					})
				: c.t("devices:attention.certificate_expiring.unnamed", {
						device: c.device,
						id: c.str("certificateId"),
						count: c.num("days"),
						date: c.when("expiresAt"),
						defaultValue_one:
							"A certificate on {{device}} (ID {{id}}) expires in {{count, number}} day ({{date}}).",
						defaultValue_other:
							"A certificate on {{device}} (ID {{id}}) expires in {{count, number}} days ({{date}}).",
					}),
			renewalSentence(c),
		),
	certificate_not_yet_valid: (c) =>
		c.t(
			"devices:attention.certificate_not_yet_valid.sentence",
			"Certificate {{label}} on {{device}} isn't valid until {{date}}.",
			{
				label: c.str("label") ?? c.str("certificateId"),
				device: c.device,
				date: c.when("validFrom"),
			},
		),
	renewal_delegation_error: (c) =>
		c.t(
			"devices:attention.renewal_delegation_error.sentence",
			"Automatic renewal of {{label}} on {{device}} failed: {{message}}",
			{
				label: c.str("label") ?? c.str("certificateId"),
				device: c.device,
				message: c.str("message"),
			},
		),
	renewal_authority_expiring: (c) =>
		c.num("authorityExpiresAt") <= c.time.now / 1000
			? c.t(
					"devices:attention.renewal_authority_expiring.expired",
					"The authority that renews {{label}} on {{device}} expired {{date}}.",
					{
						label: c.str("label") ?? c.str("certificateId"),
						device: c.device,
						date: c.when("authorityExpiresAt"),
					},
				)
			: c.t(
					"devices:attention.renewal_authority_expiring.sentence",
					"The authority that renews {{label}} on {{device}} expires {{date}}.",
					{
						label: c.str("label") ?? c.str("certificateId"),
						device: c.device,
						date: c.when("authorityExpiresAt"),
					},
				),
	acme_error: (c) =>
		c.t(
			"devices:attention.acme_error.sentence",
			"Let's Encrypt couldn't renew {{label}} on {{device}}. Next attempt {{time}}.",
			{
				label: c.str("label") ?? c.str("certificateId"),
				device: c.device,
				time: c.when("nextAttemptAt"),
			},
		),
	acme_staging_in_use: (c) =>
		c.t(
			"devices:attention.acme_staging_in_use.sentence",
			"{{label}} on {{device}} uses a Let's Encrypt test certificate that browsers don't trust.",
			{ label: c.str("label") ?? c.str("certificateId"), device: c.device },
		),
	signing_request_attention: (c) =>
		c.str("reason") === "stale"
			? c.t(
					"devices:attention.signing_request_attention.stale",
					"The signing request for {{label}} on {{device}} is out of date because the certificate changed.",
					{ label: c.str("label"), device: c.device },
				)
			: c.t(
					"devices:attention.signing_request_attention.sentence",
					"The signing request for {{label}} on {{device}} expires {{date}}.",
					{
						label: c.str("label"),
						device: c.device,
						date: c.when("expiresAt"),
					},
				),
	certificate_inventory_stale: (c) =>
		typeof c.p.updatedAt === "number"
			? c.t(
					"devices:attention.certificate_inventory_stale.sentence",
					"{{device}} last confirmed its certificates {{age}}.",
					{ device: c.device, age: c.rel("updatedAt") },
				)
			: c.t(
					"devices:attention.certificate_inventory_stale.never",
					"{{device}} hasn't reported its certificates.",
					{ device: c.device },
				),
	certificate_slots_nearly_full: (c) =>
		c.t(
			"devices:attention.certificate_slots_nearly_full.sentence",
			"{{device}} uses {{used, number}} of {{max, number}} certificate slots.",
			{ device: c.device, used: c.num("used"), max: c.num("max") },
		),
	org_ca_signing_key_expiring: (c) =>
		c.num("at") <= c.time.now / 1000
			? c.t(
					"devices:attention.org_ca_signing_key_expiring.expired",
					"Your authority {{label}} stopped signing on {{date}}.",
					{ label: c.str("label"), date: c.when("at") },
				)
			: c.t(
					"devices:attention.org_ca_signing_key_expiring.sentence",
					"Your authority {{label}} stops signing on {{date}}.",
					{ label: c.str("label"), date: c.when("at") },
				),
	org_ca_root_expiring: (c) =>
		c.t(
			"devices:attention.org_ca_root_expiring.sentence",
			"The root of authority {{label}} expires {{date}}. Certificates it signed stop being trusted then.",
			{ label: c.str("label"), date: c.when("at") },
		),
	history_paused_readers_expired: (c) =>
		c.str("kind") === "logs"
			? c.t(
					"devices:attention.history_paused_readers_expired.logs",
					"Retained logs for {{scope}} on {{device}} stopped recording on {{date}} because the readers list expired.",
					{ scope: c.scope(), device: c.device, date: c.when("since") },
				)
			: c.t(
					"devices:attention.history_paused_readers_expired.metrics",
					"Retained metrics for {{scope}} on {{device}} stopped recording on {{date}} because the readers list expired.",
					{ scope: c.scope(), device: c.device, date: c.when("since") },
				),
	history_paused_access_changed: (c) =>
		c.str("kind") === "logs"
			? c.t(
					"devices:attention.history_paused_access_changed.logs",
					"Retained logs for {{scope}} on {{device}} stopped recording after access changed on {{date}}.",
					{ scope: c.scope(), device: c.device, date: c.when("since") },
				)
			: c.t(
					"devices:attention.history_paused_access_changed.metrics",
					"Retained metrics for {{scope}} on {{device}} stopped recording after access changed on {{date}}.",
					{ scope: c.scope(), device: c.device, date: c.when("since") },
				),
	history_not_stored_by_plan: (c) =>
		c.t(
			"devices:attention.history_not_stored_by_plan.sentence",
			"Your plan doesn't keep device history in the cloud.",
		),
	history_storage_nearly_full: (c) =>
		c.t(
			"devices:attention.history_storage_nearly_full.sentence",
			"Device history uses {{used}} of {{max}}; the oldest records are dropped first.",
			{
				used: humanFileSize(c.num("usedBytes")),
				max: humanFileSize(c.num("maxBytes")),
			},
		),
	metric_readers_expiring: (c) =>
		c.t(
			"devices:attention.metric_readers_expiring.sentence",
			"Shared live metrics for {{scope}} on {{device}} stop {{in}}.",
			{ scope: c.scope(), device: c.device, in: c.rel("expiresAt") },
		),
	unconfirmed_command: (c) =>
		c.str("service")
			? c.t(
					"devices:attention.unconfirmed_command.service",
					"We sent a command to {{service}} on {{device}} but got no reply. It may have run.",
					{ service: c.service, device: c.device },
				)
			: c.t(
					"devices:attention.unconfirmed_command.sentence",
					"We sent a command to {{device}} but got no reply. It may have run.",
					{ device: c.device },
				),
	device_operation_failed: (c) =>
		c.str("operation") === "reboot"
			? c.t(
					"devices:attention.device_operation_failed.reboot",
					"Restarting {{device}} failed.",
					{ device: c.device },
				)
			: c.t(
					"devices:attention.device_operation_failed.updateAgent",
					"The agent update on {{device}} failed.",
					{ device: c.device },
				),
	device_operation_unknown: (c) =>
		c.str("operation") === "reboot"
			? c.t(
					"devices:attention.device_operation_unknown.reboot",
					"We can't confirm whether {{device}} restarted.",
					{ device: c.device },
				)
			: c.t(
					"devices:attention.device_operation_unknown.updateAgent",
					"We can't confirm whether {{device}} installed the agent update.",
					{ device: c.device },
				),
	upload_paused: (c) =>
		typeof c.p.until === "number"
			? c.t(
					"devices:attention.upload_paused.sentence",
					"Uploading {{app}} to {{device}} is paused. You can resume until {{time}}.",
					{ app: c.app(c.p.app), device: c.device, time: c.when("until") },
				)
			: c.t(
					"devices:attention.upload_paused.undated",
					"Uploading {{app}} to {{device}} is paused.",
					{ app: c.app(c.p.app), device: c.device },
				),
} satisfies Record<AttentionKey, (c: SentenceContext) => string>;

/** Primary/secondary labels (IA §6.5 "Primary action"). */
const ACTIONS = {
	diagnose: (t) => t("devices:attention.action.diagnose", "Diagnose"),
	show_start_instructions: (t) =>
		t(
			"devices:attention.action.showStartInstructions",
			"Show start instructions",
		),
	delete_keys: (t) => t("devices:attention.action.deleteKeys", "Delete keys…"),
	remove_from_computer: (t) =>
		t(
			"devices:attention.action.removeFromComputer",
			"Remove from this computer",
		),
	revoke: (t) => t("devices:attention.action.revoke", "Revoke…"),
	revoke_spending_limit: (t) =>
		t("devices:attention.action.revokeSpendingLimit", "Revoke spending limit…"),
	review_identity: (t) =>
		t("devices:attention.action.reviewIdentity", "Review identity"),
	review_diagnostics: (t) =>
		t("devices:attention.action.reviewDiagnostics", "Review"),
	fix_clock: (t) =>
		t("devices:attention.action.fixClock", "How to fix the clock"),
	show_recovery_steps: (t) =>
		t("devices:attention.action.showRecoverySteps", "Show recovery steps"),
	connect_live: (t) =>
		t("devices:attention.action.connectLive", "Connect live"),
	update_agent: (t) =>
		t("devices:attention.action.updateAgent", "Update agent…"),
	how_to_update: (t) =>
		t("devices:attention.action.howToUpdate", "How to update"),
	view_activity: (t) =>
		t("devices:attention.action.viewActivity", "View activity"),
	renew: (t) => t("devices:attention.action.renew", "Renew"),
	review_pending_setups: (t) =>
		t("devices:attention.action.reviewPendingSetups", "Review pending setups"),
	set_up_again: (t) => t("devices:attention.action.setUpAgain", "Set up again"),
	view_hub_status: (t) =>
		t("devices:attention.action.viewHubStatus", "View hub status"),
	restore_keys: (t) =>
		t("devices:attention.action.restoreKeys", "Restore keys…"),
	back_up_to_account: (t) =>
		t("devices:attention.action.backUpToAccount", "Back up to account"),
	retry_upload: (t) =>
		t("devices:attention.action.retryUpload", "Retry upload"),
	update_account_backup: (t) =>
		t("devices:attention.action.updateAccountBackup", "Update account backup"),
	review_backups: (t) =>
		t("devices:attention.action.reviewBackups", "Review backups"),
	keep_keys_safely: (t) =>
		t("devices:attention.action.keepKeysSafely", "Keep keys safely"),
	download_request_again: (t) =>
		t(
			"devices:attention.action.downloadRequestAgain",
			"Download request again",
		),
	ask_to_renew: (t) => t("devices:attention.action.askToRenew", "Ask to renew"),
	view_access: (t) => t("devices:attention.action.viewAccess", "View access"),
	renew_access_rules: (t) =>
		t("devices:attention.action.renewAccessRules", "Renew access rules"),
	review_access: (t) =>
		t("devices:attention.action.reviewAccess", "Review access"),
	view_status: (t) => t("devices:attention.action.viewStatus", "View status"),
	view_instances: (t) =>
		t("devices:attention.action.viewInstances", "View instances"),
	follow: (t) => t("devices:attention.action.follow", "Follow"),
	view_update: (t) => t("devices:attention.action.viewUpdate", "View update"),
	activate: (t) => t("devices:attention.action.activate", "Activate"),
	check_again: (t) => t("devices:attention.action.checkAgain", "Check again"),
	try_again: (t) => t("devices:attention.action.tryAgain", "Try again"),
	assign_certificate: (t) =>
		t("devices:attention.action.assignCertificate", "Assign certificate"),
	open_app_events: (t) =>
		t("devices:attention.action.openAppEvents", "Open app events"),
	review_change: (t) =>
		t("devices:attention.action.reviewChange", "Review change"),
	fix_cloud_access: (t) =>
		t("devices:attention.action.fixCloudAccess", "Fix cloud access"),
	view_queue: (t) => t("devices:attention.action.viewQueue", "View queue"),
	replace_approval: (t) =>
		t("devices:attention.action.replaceApproval", "Replace approval"),
	replace_limit: (t) =>
		t("devices:attention.action.replaceLimit", "Replace limit"),
	open_app_storage: (t) =>
		t("devices:attention.action.openAppStorage", "Open project storage"),
	view_certificate: (t) =>
		t("devices:attention.action.viewCertificate", "View certificate"),
	fix_renewal: (t) => t("devices:attention.action.fixRenewal", "Fix renewal"),
	install_renewal_authority: (t) =>
		t(
			"devices:attention.action.installRenewalAuthority",
			"Install new renewal authority",
		),
	review: (t) => t("devices:attention.action.review", "Review"),
	switch_to_production: (t) =>
		t("devices:attention.action.switchToProduction", "Switch to production"),
	finish_or_discard: (t) =>
		t("devices:attention.action.finishOrDiscard", "Finish or discard"),
	review_certificates: (t) =>
		t("devices:attention.action.reviewCertificates", "Review certificates"),
	renew_signing_key: (t) =>
		t("devices:attention.action.renewSigningKey", "Renew signing key"),
	plan_replacement: (t) =>
		t("devices:attention.action.planReplacement", "Plan replacement"),
	resume_recording: (t) =>
		t("devices:attention.action.resumeRecording", "Resume recording"),
	see_plans: (t) => t("devices:attention.action.seePlans", "See plans"),
	renew_readers: (t) =>
		t("devices:attention.action.renewReaders", "Renew readers"),
	check_result: (t) =>
		t("devices:attention.action.checkResult", "Check result"),
	view_details: (t) =>
		t("devices:attention.action.viewDetails", "View details"),
	check_status: (t) =>
		t("devices:attention.action.checkStatus", "Check status"),
	resume: (t) => t("devices:attention.action.resume", "Resume"),
} satisfies Record<AttentionActionCode, (t: DevicesT) => string>;

export function attentionActionLabel(
	t: DevicesT,
	code: AttentionActionCode,
): string {
	return ACTIONS[code](t);
}

export interface AttentionCopy {
	sentence: string;
	action?: string;
	secondary?: string;
}

/** The translated sentence and action labels of one attention item. */
export function attentionCopy(
	t: DevicesT,
	item: AttentionCopyItem,
	ctx: AttentionCopyContext = {},
): AttentionCopy {
	const copy: AttentionCopy = {
		sentence: SENTENCES[item.copy.code](sentenceContext(t, item, ctx)),
	};
	if (item.action) copy.action = ACTIONS[item.action.code](t);
	if (item.secondary) copy.secondary = ACTIONS[item.secondary.code](t);
	return copy;
}
