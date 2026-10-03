"use client";

import { useTranslation } from "@flow-like/locales";
import { type DevicesT, useAreaTime } from "../primitives/area-context";
import { DvButton } from "../primitives/dv-button";
import { type Gate, GateNotice } from "../primitives/gate-notice";
import { useRouteLink } from "../routing/use-devices-route";
import type { ObserveTarget } from "./use-observe-target";

/** The owner renews the rules on the device's Access tab. */
export function RenewRulesLink({ deviceId }: Readonly<{ deviceId: string }>) {
	const { t } = useTranslation("devices");
	const link = useRouteLink();
	return (
		<DvButton asChild size="sm">
			<a {...link({ screen: "device", deviceId, tab: "access" })}>
				{t("observe.history.renewRules", "Renew access rules")}
			</a>
		</DvButton>
	);
}

/**
 * When the device's access rules ran out, as this computer knows them (Unix
 * seconds); undefined while they are valid or unknown. Until the owner renews
 * them, history is paused for everyone and no readers list can be signed (BG31).
 */
export function rulesExpiredAt(
	target: ObserveTarget,
	now: number,
): number | undefined {
	const until = target.policy?.expires_at;
	return until !== undefined && until <= now ? until : undefined;
}

/** The reason next to a history action that expired rules block. */
export const rulesGate = (t: DevicesT): Gate => ({
	kind: "policy",
	reason: t("observe.history.rulesGate", "Renew the access rules first."),
});

/** Says once why every history action waits, with the way out. */
export function RulesExpiredNotice({
	target,
	expiredAt,
}: Readonly<{ target: ObserveTarget; expiredAt: number }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	return (
		<GateNotice
			kind="policy"
			title={t(
				"observe.history.rulesExpiredTitle",
				"Access rules on {{device}} expired {{date}}.",
				{ device: target.name, date: time.at(expiredAt) },
			)}
			text={t(
				"observe.history.rulesExpiredText",
				"Retained history is paused for everyone, you included, and a readers list can't be signed until the rules are renewed.",
			)}
			actions={<RenewRulesLink deviceId={target.deviceId} />}
		/>
	);
}
