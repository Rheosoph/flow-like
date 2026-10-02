"use client";

import { useTranslation } from "@flow-like/locales";
import { RefreshCw } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import {
	type ArtifactBudget,
	type ArtifactQuota,
	agentSupports,
	readArtifactUsage,
} from "../../../../lib/device-management/agent-reads";
import {
	type DeviceErrorCode,
	classifyDeviceError,
} from "../../../../lib/device-management/workspace/errors";
import { humanFileSize } from "../../../../lib/utils";
import { errorCopy } from "../copy/error-copy";
import { useAreaTime } from "../primitives/area-context";
import { DvButton } from "../primitives/dv-button";
import { GatedAction } from "../primitives/gate-notice";
import { InlineResult } from "../primitives/inline-result";
import { KeyValueList, KvRow } from "../primitives/key-value-list";
import { Meter } from "../primitives/meter";
import { useDeviceCall, useGate } from "../workspace";
import { GateFix } from "./device-header";
import { type DevicePage, gateView } from "./use-device-page";

const CERTIFICATE_SLOTS = 32;
const CONNECTION_SLOTS = 8;
const OWNER_SLOTS = 2;
const SLOTS_PER_PERSON = 2;

type UsageState =
	| { kind: "idle" }
	| { kind: "reading" }
	| { kind: "ok"; budget: ArtifactBudget | null; at: number }
	| { kind: "failed"; code: DeviceErrorCode; reason?: string };

function Quota({
	quota,
	label,
	format = String,
}: Readonly<{
	quota: ArtifactQuota;
	label: string;
	format?: (value: number) => string;
}>) {
	const { t } = useTranslation("devices");
	const text =
		quota.max === null
			? t("device.capacity.quotaNoMax", "{{label}}: {{used}}", {
					label,
					used: format(quota.used),
				})
			: t("device.capacity.quota", "{{label}}: {{used}} of {{max}}", {
					label,
					used: format(quota.used),
					max: format(quota.max),
				});
	if (quota.max === null || quota.max === 0)
		return <span className="text-ui">{text}</span>;
	const percent = (quota.used / quota.max) * 100;
	return (
		<Meter
			className="max-w-[320px]"
			segments={[
				{
					value: percent,
					tone:
						percent >= 90 ? "critical" : percent >= 75 ? "warning" : "neutral",
				},
			]}
			label={text}
			caption={text}
		/>
	);
}

/** BG17: how much of the device's app storage is used. Read on demand only: the device walks every kept file. */
function AppStorage({ page }: Readonly<{ page: DevicePage }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const call = useDeviceCall(page.deviceId);
	const features = page.inspection?.features;
	const result = useGate("refresh_status", page.deviceId);
	const [state, setState] = useState<UsageState>({ kind: "idle" });
	const alive = useRef(true);
	useEffect(() => {
		alive.current = true;
		return () => {
			alive.current = false;
		};
	}, []);
	const supported = agentSupports(features, "artifact_capacity");
	if (page.inspection && !supported)
		return (
			<span className="text-muted-foreground">
				{t(
					"device.capacity.storageUnsupported",
					"Not reported. Update the device agent to see how much storage app versions use.",
				)}
			</span>
		);
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
		: gateView(t, time, result);
	const read = async () => {
		setState({ kind: "reading" });
		try {
			const usage = await readArtifactUsage(call, features, {});
			if (!alive.current) return;
			setState(
				usage.kind === "ok"
					? {
							kind: "ok",
							budget: usage.data.device,
							at: Math.floor(time.now / 1000),
						}
					: { kind: "failed", code: "rejected_unsupported" },
			);
		} catch (error) {
			if (!alive.current) return;
			const failure = classifyDeviceError(error);
			setState({
				kind: "failed",
				code: failure.code,
				...(failure.rejection?.error
					? { reason: failure.rejection.error }
					: {}),
			});
		}
	};
	const button = (
		<DvButton
			size="sm"
			icon={RefreshCw}
			busy={state.kind === "reading"}
			onClick={() => void read()}
		>
			{state.kind === "ok"
				? t("device.capacity.storageAgain", "Check again")
				: t("device.capacity.storageCheck", "Check app storage")}
		</DvButton>
	);
	return (
		<span className="flex flex-col items-start gap-2">
			{state.kind === "ok" ? (
				state.budget ? (
					<span className="flex w-full flex-col gap-2">
						<Quota
							quota={state.budget.revisions}
							label={t("device.capacity.versions", "App versions kept")}
						/>
						<Quota
							quota={state.budget.bytes}
							label={t("device.capacity.bytes", "Storage")}
							format={(value) => humanFileSize(value)}
						/>
						<Quota
							quota={state.budget.entries}
							label={t("device.capacity.entries", "Files and folders")}
						/>
						<span className="text-xs text-muted-foreground">
							{t(
								"device.capacity.readAt",
								"Read from the device at {{time}}.",
								{
									time: time.clock(state.at),
								},
							)}
						</span>
					</span>
				) : (
					<span className="text-muted-foreground">
						{t(
							"device.capacity.storageNoAccess",
							"The device shows its total only to the owner and to people with whole-device Deploy & configure.",
						)}
					</span>
				)
			) : (
				<span className="text-muted-foreground">
					{t(
						"device.capacity.storageIdle",
						"Not read yet. The device counts every kept file, so it is read only when you ask.",
					)}
				</span>
			)}
			{view ? (
				<span className="inline-flex flex-wrap items-start gap-2">
					<GatedAction gate={view.gate}>{button}</GatedAction>
					<GateFix view={view} />
				</span>
			) : (
				button
			)}
			{state.kind === "failed" ? (
				<InlineResult
					tone="critical"
					onDismiss={() => setState({ kind: "idle" })}
				>
					{errorCopy(t, state.code, { device: page.name })}
					{state.reason ? ` “${state.reason}”` : null}
				</InlineResult>
			) : null}
		</span>
	);
}

/** Device settings › Capacity: how much room the device has for services, certificates, connections and app versions. */
export function CapacityBlock({ page }: Readonly<{ page: DevicePage }>) {
	const { t } = useTranslation("devices");
	const services = page.services ?? page.lockedRows?.services;
	const certificates = page.view.certificates;
	return (
		<KeyValueList>
			<KvRow label={t("device.capacity.services", "Services")}>
				{services ? (
					<span className="font-mono tabular-nums">{services.length}</span>
				) : (
					<span className="text-muted-foreground">
						{t("device.capacity.servicesUnknown", "Unknown until unlocked")}
					</span>
				)}
			</KvRow>
			<KvRow label={t("device.capacity.certificates", "Certificate slots")}>
				{certificates && certificates.updated_at !== null ? (
					<>
						{t(
							"device.capacity.certificatesValue",
							"{{used, number}} of {{max, number}}",
							{
								used: certificates.certificates.length,
								max: CERTIFICATE_SLOTS,
							},
						)}
						<span className="ml-1.5 text-xs text-muted-foreground">
							{t(
								"device.capacity.certificatesNote",
								"pending requests and Let's Encrypt policies use slots too",
							)}
						</span>
					</>
				) : (
					<span className="text-muted-foreground">
						{t(
							"device.capacity.certificatesUnknown",
							"Not reported · up to {{max, number}}",
							{ max: CERTIFICATE_SLOTS },
						)}
					</span>
				)}
			</KvRow>
			<KvRow label={t("device.capacity.connections", "Connection slots")}>
				{t(
					"device.capacity.connectionsValue",
					"{{max, number}} live connections at once · {{owner, number}} reserved for the owner · {{each, number}} per person",
					{ max: CONNECTION_SLOTS, owner: OWNER_SLOTS, each: SLOTS_PER_PERSON },
				)}
			</KvRow>
			<KvRow label={t("device.capacity.storage", "Uploaded app versions")}>
				<AppStorage page={page} />
			</KvRow>
		</KeyValueList>
	);
}
