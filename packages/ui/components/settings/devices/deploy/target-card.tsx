"use client";

import { useTranslation } from "@flow-like/locales";
import { Check, Copy, KeyRound, Lock, LockOpen, Radio } from "lucide-react";
import type { ReactNode } from "react";
import type { DevicesRoute } from "../../../../lib/device-management/model/types";
import { Checkbox } from "../../../ui/checkbox";
import { fixLabel } from "../copy/gate-copy";
import { type DevicesT, useAreaTime } from "../primitives/area-context";
import { DvButton } from "../primitives/dv-button";
import { GateInline } from "../primitives/gate-notice";
import { PresenceGlyph } from "../primitives/presence-glyph";
import { RelationshipChip } from "../primitives/status-chip";
import { cx } from "../primitives/tone";
import { useCopy } from "../primitives/use-copy";
import { useDevicesRoute } from "../routing/use-devices-route";
import { useOverlay } from "../workspace/overlay-store";
import { useFixAction } from "../workspace/use-gate";
import { whereGateKind, whereGateText } from "./deploy-copy";
import type { DeployDevice, WhereGate } from "./deploy-facts";

/* APP §3.7 item 1: one device as a checkbox card with the facts that shape later steps and, when gated, the reason under it (R7). */

const PLATFORM_LABEL: Record<string, (t: DevicesT) => string> = {
	linux: (t) => t("devices:deploy.card.linux", "Linux"),
	macos: (t) => t("devices:deploy.card.mac", "Mac"),
	windows: (t) => t("devices:deploy.card.windows", "Windows"),
};

const ISOLATION_LABEL: Record<
	NonNullable<DeployDevice["isolation"]>,
	(t: DevicesT) => string
> = {
	required: (t) => t("devices:deploy.card.sandboxRequired", "sandbox required"),
	optional: (t) =>
		t("devices:deploy.card.sandboxAvailable", "sandbox available"),
	none: (t) => t("devices:deploy.card.noSandbox", "can't sandbox"),
};

function platformText(t: DevicesT, platform: string | undefined): string {
	if (!platform)
		return t("devices:deploy.card.platformUnknown", "platform unknown");
	return PLATFORM_LABEL[platform]?.(t) ?? platform;
}

function agentText(t: DevicesT, agent: string | undefined): string {
	return agent
		? t("devices:deploy.card.agent", "agent {{version}}", { version: agent })
		: t("devices:deploy.card.agentUnknown", "agent unknown");
}

function isolationText(t: DevicesT, device: DeployDevice): string {
	return device.isolation
		? ISOLATION_LABEL[device.isolation](t)
		: t("devices:deploy.card.isolationUnknown", "isolation unknown");
}

/** Nothing read yet: the facts come with the unlock, or with the live connection. */
function pendingFacts(t: DevicesT, device: DeployDevice): string | null {
	if (device.locked)
		return t(
			"devices:deploy.card.loadsOnUnlock",
			"Platform, agent and isolation load when you unlock",
		);
	const known = device.platform ?? device.agent ?? device.isolation;
	return known
		? null
		: t(
				"devices:deploy.card.loadsOnConnect",
				"Platform, agent and isolation load once it's connected",
			);
}

function factsText(t: DevicesT, device: DeployDevice): string[] {
	const pending = pendingFacts(t, device);
	if (pending) return [pending];
	return [
		platformText(t, device.platform),
		agentText(t, device.agent),
		isolationText(t, device),
	];
}

function ConnectionLine({ device }: Readonly<{ device: DeployDevice }>) {
	const { t } = useTranslation("devices");
	const { live } = device;
	const line = (() => {
		if (live.kind === "live" || live.kind === "renewing")
			return {
				icon: Radio,
				text:
					live.transport === "websocket"
						? t("deploy.card.liveRelayed", "Live · relayed")
						: t("deploy.card.liveDirect", "Live · direct"),
			};
		if (device.locked)
			return {
				icon: Lock,
				text: t("deploy.card.locked", "Locked on this computer"),
			};
		if (device.keyState !== "unlocked")
			return { icon: KeyRound, text: t("deploy.card.noKeys", "No keys here") };
		// A device that can't be picked right now makes no promise about connecting.
		if (device.gate)
			return {
				icon: LockOpen,
				text: t("deploy.card.unlockedOnly", "Unlocked"),
			};
		return {
			icon: LockOpen,
			text:
				live.kind === "connecting" || live.kind === "reconnecting"
					? t("deploy.card.connecting", "Unlocked · connecting…")
					: t(
							"deploy.card.unlocked",
							"Unlocked · connects live when you pick it",
						),
		};
	})();
	const Icon = line.icon;
	return (
		<span className="flex min-w-0 items-start gap-1 text-xs text-muted-foreground">
			<Icon aria-hidden className="mt-0.5 size-3 shrink-0" />
			<span className="min-w-0">{line.text}</span>
		</span>
	);
}

/** The fix buttons of a Where gate (APP §3.7 "Fix" column). */
export function GateFixes({
	device,
	gate,
	appName,
	size = "xs",
}: Readonly<{
	device: DeployDevice;
	gate: WhereGate;
	appName: string;
	size?: "xs" | "sm";
}>) {
	const { t } = useTranslation("devices");
	const { navigate } = useDevicesRoute();
	const { openDiagnose } = useOverlay();
	const runFix = useFixAction();
	const { copied, copy } = useCopy();
	const go = (route: DevicesRoute) => navigate(route);
	const toService = () =>
		go({
			screen: "service",
			deviceId: device.id,
			serviceId: gate.serviceId ?? "",
			tab: "status",
		});
	switch (gate.code) {
		case "never":
			return (
				<DvButton
					size={size}
					onClick={() =>
						go({ screen: "device", deviceId: device.id, tab: "overview" })
					}
				>
					{t("deploy.gate.startInstructions", "Start instructions")}
				</DvButton>
			);
		case "offline":
			return (
				<DvButton size={size} onClick={() => openDiagnose(device.id)}>
					{t("deploy.gate.diagnose", "Diagnose")}
				</DvButton>
			);
		case "nokeys":
			return (
				<>
					<DvButton
						size={size}
						onClick={() => go({ screen: "keys", focusDeviceId: device.id })}
					>
						{t("deploy.gate.restoreKeys", "Restore keys…")}
					</DvButton>
					<DvButton
						size={size}
						variant="ghost"
						onClick={() =>
							go({ screen: "access", tab: "shared", action: "request" })
						}
					>
						{t("deploy.gate.requestAccess", "Request access")}
					</DvButton>
				</>
			);
		case "no_deploy":
			return (
				<DvButton
					size={size}
					icon={copied ? Check : Copy}
					onClick={() =>
						void copy(
							t(
								"deploy.gate.askOwnerText",
								"Hi, could you give me Deploy & configure on {{device}} for {{app}}? I'd like to deploy it there.",
								{ device: device.name, app: appName },
							),
						)
					}
				>
					{copied
						? t("deploy.gate.copied", "Copied")
						: t("deploy.gate.askOwner", "Ask the owner")}
				</DvButton>
			);
		case "busy":
			return (
				<DvButton size={size} onClick={toService}>
					{t("deploy.gate.follow", "Follow")}
				</DvButton>
			);
		case "staged":
			return (
				<DvButton size={size} onClick={toService}>
					{t("deploy.gate.openService", "Open {{service}}", {
						service: gate.serviceId ?? "",
					})}
				</DvButton>
			);
		default: {
			const { fix } = gate.failure;
			if (!fix) return null;
			return (
				<DvButton
					size={size}
					onClick={() => {
						const outcome = runFix(fix);
						if (outcome.kind === "navigate") go(outcome.route);
					}}
				>
					{fixLabel(t, fix)}
				</DvButton>
			);
		}
	}
}

export interface TargetCardProps {
	device: DeployDevice;
	checked: boolean;
	/** The device of a one-service update can't be changed. */
	fixed?: boolean;
	appName: string;
	/** "Storage: 1.9 GiB of 16 GiB used", when the agent reports it (BG17). */
	storage?: string;
	/** Plan lines, shown while ticked. */
	plan?: ReactNode;
	onCheckedChange(checked: boolean): void;
}

export function TargetCard({
	device,
	checked,
	fixed = false,
	appName,
	storage,
	plan,
	onCheckedChange,
}: Readonly<TargetCardProps>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const { gate } = device;
	// A staged update leaves the card ticked; its banner offers the way out.
	const blocks = gate !== null && gate.code !== "staged";
	const id = `deploy-device-${device.id}`;
	const facts = [...factsText(t, device), ...(storage ? [storage] : [])];
	return (
		<div
			data-device={device.id}
			data-ok={!blocks}
			data-locked={device.locked}
			className="flex min-w-0 flex-col gap-2"
		>
			<label
				htmlFor={id}
				className={cx(
					"flex min-w-0 cursor-pointer items-start gap-2.5 rounded-lg border border-border bg-card px-3 py-2.5 hover:border-border-strong has-focus-visible:outline-2 has-focus-visible:outline-offset-2 has-focus-visible:outline-ring",
					checked && "border-foreground bg-row-selected",
					(blocks || fixed) && "cursor-not-allowed",
					blocks && !checked && "bg-surface-sunken opacity-70",
				)}
			>
				<Checkbox
					id={id}
					checked={checked}
					disabled={blocks || (fixed && checked)}
					onCheckedChange={(next) => onCheckedChange(next === true)}
					className="mt-0.5 border-border-strong shadow-none focus-visible:ring-0 data-[state=checked]:border-foreground data-[state=checked]:bg-foreground data-[state=checked]:text-background dark:data-[state=checked]:bg-foreground"
				/>
				<span className="flex min-w-0 flex-col gap-1">
					<span className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1">
						<PresenceGlyph kind={device.presence.kind} />
						<b className="font-mono text-ui font-semibold wrap-anywhere">
							{device.name}
						</b>
						<RelationshipChip
							relationship={device.relationship}
							{...(device.row.access_expires_at
								? { endsAt: device.row.access_expires_at }
								: {})}
						/>
					</span>
					<span className="flex min-w-0 items-start gap-1 text-xs text-muted-foreground">
						{device.locked ? (
							<Lock aria-hidden className="mt-0.5 size-3 shrink-0" />
						) : null}
						<span className="min-w-0">{facts.join(" · ")}</span>
					</span>
					<ConnectionLine device={device} />
				</span>
			</label>
			{checked && plan ? (
				<div
					data-plan-line=""
					className="flex min-w-0 flex-col gap-1.5 rounded-lg border border-hairline bg-surface-sunken px-3 py-2 text-ui"
				>
					{plan}
					{blocks && !fixed ? (
						<DvButton
							variant="link"
							size="xs"
							className="w-fit"
							onClick={() => onCheckedChange(false)}
						>
							{t("deploy.card.remove", "Remove from this deploy")}
						</DvButton>
					) : null}
					{fixed ? (
						<p className="text-xs text-muted-foreground">
							{t(
								"deploy.card.fixedDevice",
								"The device of a one-service update is fixed. Deploy from the app to pick others.",
							)}
						</p>
					) : null}
				</div>
			) : null}
			{gate && blocks ? (
				<div className="flex flex-col items-start gap-1.5 px-0.5">
					<GateInline kind={whereGateKind(gate)} className="max-w-none">
						{whereGateText(t, gate, device.name, time)}
					</GateInline>
					<span className="flex flex-wrap gap-1.5">
						<GateFixes device={device} gate={gate} appName={appName} />
					</span>
				</div>
			) : null}
		</div>
	);
}
