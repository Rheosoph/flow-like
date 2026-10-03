"use client";

import { useTranslation } from "@flow-like/locales";
import { useAreaTime } from "../primitives/area-context";
import { DvButton } from "../primitives/dv-button";
import { GatedAction } from "../primitives/gate-notice";
import { InlineResult } from "../primitives/inline-result";
import { KeyValueList, KvRow } from "../primitives/key-value-list";
import {
	actionResultKey,
	useDeviceAction,
	useFleetDeviceStates,
	useGate,
	useInlineResults,
} from "../workspace";
import { GateFix } from "./device-header";
import { type DevicePage, gateView } from "./use-device-page";

const LIFETIME_DAYS = 365;

/**
 * IA §6.7 S10: this computer's subscription to the device's encrypted status.
 * The first unlock creates it; Renew signs a new one, Stop receiving removes it on the hub.
 */
export function StatusSubscription({ page }: Readonly<{ page: DevicePage }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const actions = useDeviceAction();
	const reader = useFleetDeviceStates()[page.deviceId]?.reader;
	const gate = gateView(t, time, useGate("fleet_monitor", page.deviceId));
	const resultKey = actionResultKey("fleet_monitor", page.deviceId);
	const results = useInlineResults(resultKey);
	const view = page.identity
		? {
				gate: {
					kind: "policy" as const,
					reason: t(
						"device.identity.blockedShort",
						"Management is blocked until the identity is confirmed.",
					),
				},
			}
		: gate;
	const renew = () =>
		actions.run({
			action: "fleet_monitor",
			deviceId: page.deviceId,
			label: t(
				"device.subscription.renewLabel",
				"Renew the encrypted status subscription of {{device}}",
				{ device: page.name },
			),
			resultKey,
			call: ({ workspace }) => workspace.fleet.renew(page.deviceId),
		});
	const stop = () =>
		actions.run({
			action: "fleet_monitor",
			deviceId: page.deviceId,
			label: t(
				"device.subscription.stopLabel",
				"Stop receiving encrypted status of {{device}}",
				{ device: page.name },
			),
			consequence: {
				what: t(
					"device.subscription.rows.what",
					'This computer stops receiving {{device}}\'s encrypted status. Its services read "No access" here while the device is locked or not connected live.',
					{ device: page.name },
				),
				who: t(
					"device.subscription.rows.who",
					"Only you, on this computer. Other computers and other people keep their subscriptions.",
				),
				stays: t(
					"device.subscription.rows.stays",
					"Your keys, live connections and the device itself.",
				),
				when: t("device.subscription.rows.when", "Immediately."),
				undo: {
					reversible: true,
					text: t(
						"device.subscription.rows.undo",
						"Renew creates a new subscription.",
					),
				},
			},
			confirm: {
				title: t(
					"device.subscription.stopTitle",
					"Stop receiving encrypted status of {{device}}?",
					{ device: page.name },
				),
				tone: "danger",
			},
			resultKey,
			call: ({ workspace }) => workspace.fleet.stopReceiving(page.deviceId),
		});
	const pending = actions.pending(resultKey);
	const renewButton = (
		<DvButton busy={pending} onClick={() => void renew()}>
			{t("device.subscription.renew", "Renew")}
		</DvButton>
	);
	return (
		<>
			{reader ? (
				<KeyValueList>
					<KvRow label={t("device.subscription.expires", "Expires")}>
						<span title={time.abs(reader.expiresAt)}>
							{t("device.subscription.expiresValue", "{{date}} ({{ago}})", {
								date: time.at(reader.expiresAt),
								ago: time.ago(reader.expiresAt, "long"),
							})}
						</span>
					</KvRow>
				</KeyValueList>
			) : (
				<p className="text-ui text-muted-foreground">
					{t(
						"device.subscription.none",
						"No subscription yet. The first unlock on this computer creates one.",
					)}
				</p>
			)}
			<p className="text-xs text-muted-foreground">
				{reader
					? t(
							"device.subscription.hint",
							"Your first unlock created it. It lets this computer read the device's encrypted status for {{days, number}} days.",
							{ days: LIFETIME_DAYS },
						)
					: t(
							"device.subscription.hintNone",
							"A subscription lets this computer read the device's encrypted status for {{days, number}} days without a live connection.",
							{ days: LIFETIME_DAYS },
						)}
			</p>
			<div className="flex flex-wrap items-start gap-2">
				{view ? (
					<>
						<GatedAction gate={view.gate}>{renewButton}</GatedAction>
						<GateFix view={view} />
					</>
				) : (
					renewButton
				)}
				{reader ? (
					<DvButton
						variant="danger-ghost"
						aria-disabled={view || pending ? true : undefined}
						onClick={() => void stop()}
					>
						{t("device.subscription.stop", "Stop receiving…")}
					</DvButton>
				) : null}
			</div>
			{results.map((result) => (
				<InlineResult
					key={result.id}
					tone={result.tone}
					onDismiss={result.dismiss}
				>
					{result.text}
				</InlineResult>
			))}
		</>
	);
}
