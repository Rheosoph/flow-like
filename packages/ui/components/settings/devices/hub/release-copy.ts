import type { HubError } from "../../../../lib/device-management/hub/endpoints";
import type { HubErrorCode } from "../../../../lib/device-management/model/types";
import type { ReleaseCheck } from "../../../../lib/device-package";
import type { AreaTime, DevicesT } from "../primitives/area-context";
import { dayText } from "../primitives/day";
import { hubErrorCopy } from "../workspace";
import { DAY, type ReleaseVerdict, daysLeft } from "./hub-view";

/* The words for the agent release, shared by the Hub status page, setup and the device Settings tab. */

/** Which check a release list failed, to finish "…failed a check: …". */
export function releaseCheckText(t: DevicesT, check: ReleaseCheck): string {
	const texts: Record<ReleaseCheck, () => string> = {
		signature: () =>
			t(
				"devices:hub.release.reason.signature",
				"it isn't signed by a key the hub operator pinned",
			),
		sequence: () =>
			t(
				"devices:hub.release.reason.sequence",
				"its release number is below the hub's minimum",
			),
		not_yet_valid: () =>
			t(
				"devices:hub.release.reason.notYetValid",
				"it isn't valid yet; check this computer's clock",
			),
		expired: () => t("devices:hub.release.reason.expired", "it has run out"),
		lifetime: () =>
			t(
				"devices:hub.release.reason.lifetime",
				"it is valid for longer than this app accepts; update the app",
			),
		invalid: () =>
			t("devices:hub.release.reason.invalid", "it isn't a valid release list"),
	};
	return texts[check]();
}

const UNREACHED: ReadonlySet<HubErrorCode> = new Set([
	"network",
	"timeout",
	"server_error",
	"rate_limited",
]);

/** Why the release list didn't arrive, as one sentence. */
export function releaseFetchCause(t: DevicesT, error: HubError): string {
	return UNREACHED.has(error.code)
		? hubErrorCopy(t, error.code)
		: t(
				"devices:hub.release.unfetchedOther",
				"The release address didn't answer with a release list.",
			);
}

/** The last day of a release: its end is a time of day, and "1 day left" would overstate it. */
export const isLastDay = (expiresAt: number, nowS: number) =>
	expiresAt - nowS < DAY;

/** "12 days left" in the last 30 days of a release; "less than a day left" on its last day. */
export function releaseLeftText(
	t: DevicesT,
	expiresAt: number,
	nowS: number,
): string {
	if (isLastDay(expiresAt, nowS))
		return t("devices:hub.release.lastDay", "less than a day left");
	return t("devices:hub.release.daysLeft", {
		count: daysLeft(expiresAt, nowS),
		defaultValue_one: "{{count, number}} day left",
		defaultValue_other: "{{count, number}} days left",
	});
}

/** The end of a release as a day, or as a time once less than a day is left. */
export const releaseEndText = (
	time: Pick<AreaTime, "at" | "locale" | "now" | "nowS">,
	expiresAt: number,
) =>
	isLastDay(expiresAt, time.nowS)
		? time.at(expiresAt)
		: dayText(time, expiresAt);

export type ShownVerdict = Exclude<
	ReleaseVerdict,
	{ kind: "waiting" | "missing" }
>;

/** The one sentence the release block opens with. */
export function verdictSentence(
	t: DevicesT,
	time: AreaTime,
	verdict: ShownVerdict,
): string {
	switch (verdict.kind) {
		case "ok":
			return t(
				"devices:hub.release.verdict.ok",
				"New and updated devices get agent {{version}}. The release is genuine and current. Nothing to do until {{date}}.",
				{
					version: verdict.facts.release_version,
					date: dayText(time, verdict.facts.expires_at),
				},
			);
		case "ends_soon":
			return t(
				"devices:hub.release.verdict.endsSoon",
				"Agent {{version}} can be used until {{date}} ({{left}}). After that no device can be set up or updated until the hub operator publishes or renews a release. Devices that are running keep running.",
				{
					version: verdict.facts.release_version,
					date: releaseEndText(time, verdict.facts.expires_at),
					left: releaseLeftText(t, verdict.facts.expires_at, time.nowS),
				},
			);
		case "expired":
			return t(
				"devices:hub.release.verdict.expired",
				"The release of agent {{version}} ran out on {{date}}. No device can be set up or updated until the hub operator publishes or renews a release. Devices that are running keep running.",
				{
					version: verdict.facts.release_version,
					date: dayText(time, verdict.facts.expires_at),
				},
			);
		case "failed":
			return t(
				"devices:hub.release.verdict.failed",
				"The hub's agent release failed a check: {{check}}. Don't set up or update devices until this is resolved.",
				{ check: releaseCheckText(t, verdict.check) },
			);
		case "unfetched":
			return t(
				"devices:hub.release.verdict.unfetched",
				"The release list couldn't be fetched, so nothing is verified. Setup and agent updates wait. {{cause}}",
				{ cause: releaseFetchCause(t, verdict.error) },
			);
		default:
			return t("devices:hub.release.verifying", "Checking the release…");
	}
}
