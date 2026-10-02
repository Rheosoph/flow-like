"use client";

import { useTranslation } from "@flow-like/locales";
import { LoaderCircle, OctagonX } from "lucide-react";
import type {
	DevicesRoute,
	HubDeviceSupport,
} from "../../../../lib/device-management/model/types";
import { StatusChip } from "../primitives/status-chip";
import { cx } from "../primitives/tone";
import { useRouteLink } from "../routing/use-devices-route";

const HUB_STATUS: DevicesRoute = { screen: "hub" };

/**
 * SPEC §3.2 item 9: shown only while the hub isn't "on"; opens Hub status.
 * Below 480 px the bar keeps only its buttons (SPEC §3.8); the area gate and the status bar say the same.
 */
export function HubPill({
	state,
	className,
}: Readonly<{ state: HubDeviceSupport["state"]; className?: string }>) {
	const { t } = useTranslation("devices");
	const link = useRouteLink();
	if (state === "on") return null;
	const labels = {
		checking: t("shell.hub.checking", "Checking this hub…"),
		unreachable: t("shell.hub.unreachable", "Hub unreachable"),
		off: t("shell.hub.off", "Devices are off on this hub"),
	} satisfies Record<Exclude<HubDeviceSupport["state"], "on">, string>;
	const checking = state === "checking";
	return (
		<a
			{...link(HUB_STATUS)}
			data-hub={state}
			aria-label={labels[state]}
			title={labels[state]}
			className={cx(
				"inline-flex shrink-0 rounded-full focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring @max-[480px]/devices:hidden",
				className,
			)}
		>
			<StatusChip
				tone={checking ? "warning" : "critical"}
				icon={checking ? LoaderCircle : OctagonX}
				spin={checking}
			>
				<span className="@max-[900px]/devices:hidden">{labels[state]}</span>
			</StatusChip>
		</a>
	);
}
