"use client";

import { useTranslation } from "@flow-like/locales";
import { useRouteLink } from "../routing/use-devices-route";
import { useDeviceUsage, useHubSupport } from "../workspace";

const LINK = "underline underline-offset-2 hover:text-foreground";

/**
 * "Device slots 6 of 100 · 1 unused setup package · setup packages today 1 of 220."
 * On a hub without the usage route the limits are stated without usage (BG3 interim).
 */
export function SlotsCaption() {
	const { t } = useTranslation("devices");
	const link = useRouteLink();
	const usage = useDeviceUsage();
	const { support } = useHubSupport();
	const hubLimits = (
		<a {...link({ screen: "hub" })} className={LINK}>
			{t("fleet.slots.hubLimits", "Hub limits")}
		</a>
	);

	if (usage.data) {
		const { limits, usage: used } = usage.data;
		// The hub counts an unused setup package as a device slot, like a registered device.
		const slots = used.active_devices + used.pending_enrollments;
		const over = slots > limits.max_devices;
		return (
			<span data-slots="usage">
				{t("fleet.slots.usage", {
					count: used.pending_enrollments,
					used: slots,
					max: limits.max_devices,
					today: used.enrollments_last_24h,
					cap: limits.max_enrollments_per_day,
					defaultValue_one:
						"Device slots {{used, number}} of {{max, number}} · {{count, number}} unused setup package · setup packages today {{today, number}} of {{cap, number}}.",
					defaultValue_other:
						"Device slots {{used, number}} of {{max, number}} · {{count, number}} unused setup packages · setup packages today {{today, number}} of {{cap, number}}.",
				})}{" "}
				{over ? (
					<span data-slots-over="" className="text-warning">
						{t(
							"fleet.slots.over",
							"You have more devices than this hub allows.",
						)}{" "}
					</span>
				) : null}
				{hubLimits}
			</span>
		);
	}

	if (!usage.missingOnHub) return null;
	const limits = support.limits ?? support.configuredLimits;
	if (limits?.max_devices === undefined)
		return (
			<span data-slots="unknown">
				{t(
					"fleet.slots.unknown",
					"This hub doesn't report device slots or how many are in use.",
				)}{" "}
				{hubLimits}
			</span>
		);
	return (
		<span data-slots="limits">
			{limits.max_pending_enrollments === undefined
				? t("fleet.slots.limitDevices", "Up to {{max, number}} devices.", {
						max: limits.max_devices,
					})
				: t(
						"fleet.slots.limits",
						"Up to {{max, number}} devices and {{pending, number}} unused setup packages.",
						{
							max: limits.max_devices,
							pending: limits.max_pending_enrollments,
						},
					)}{" "}
			{t("fleet.slots.noUsage", "This hub doesn't report how many are in use.")}{" "}
			{hubLimits}
		</span>
	);
}
