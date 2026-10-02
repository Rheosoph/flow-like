"use client";

import { useTranslation } from "@flow-like/locales";
import { Copy, RefreshCw } from "lucide-react";
import type { ReactNode } from "react";
import type {
	FixAction,
	GateFailure,
	GateResult,
} from "../../../../lib/device-management/model/types";
import { gateCopy } from "../copy/gate-copy";
import { type AreaTime, useAreaTime } from "../primitives/area-context";
import type { DevicesT } from "../primitives/area-context";
import { DvButton } from "../primitives/dv-button";
import { FreshnessStamp } from "../primitives/freshness-stamp";
import { type Gate, GateNotice } from "../primitives/gate-notice";
import { InlineResult, type ResultTone } from "../primitives/inline-result";
import { StateView } from "../primitives/state-view";
import { cx } from "../primitives/tone";
import { useCopy } from "../primitives/use-copy";
import { useDevicesRoute } from "../routing/use-devices-route";
import { stampOf } from "../shell/attention-popover";
import { useFixAction, useInlineResults } from "../workspace";
import type { ServiceConfigRead } from "./use-service-config";

/* Pieces the three settings tabs share: why settings aren't shown, gate lines, results. */

export function Mono({
	className,
	children,
}: Readonly<{ className?: string; children: ReactNode }>) {
	return (
		<span className={cx("font-mono wrap-anywhere", className)}>{children}</span>
	);
}

export const LINK =
	"underline decoration-border-strong underline-offset-2 hover:decoration-current";

/** A shadcn Select dressed like the area's inputs (no `DvSelect` primitive yet). */
export const SELECT_TRIGGER =
	"h-8.5 w-full rounded-lg border-input bg-card text-[13px]/[18px] shadow-none";
export const SELECT_CONTENT =
	"border-border-strong bg-popover shadow-none backdrop-blur-none";
export const SELECT_ITEM =
	"text-[13px]/[18px] focus:bg-row-hover focus:text-foreground";

/** The app's global table styles add cell borders and margins a `DvTable` doesn't have (requests.md → W4-SWITCH). */
export const TABLE_RESET =
	"my-0 [&_td]:border-x-0 [&_td]:border-b-0 [&_th]:border-x-0 [&_th]:border-t-0";

/**
 * A facts list for half-width columns and cards: label and value stay side by
 * side down to 380 px (`KeyValueList` stacks below 520 px). Takes `KvRow` and
 * `KvGroup` children.
 */
export function FactList({
	className,
	children,
}: Readonly<{ className?: string; children: ReactNode }>) {
	return (
		<div className="@container/facts min-w-0">
			<dl
				className={cx(
					"grid grid-cols-[fit-content(40%)_minmax(0,1fr)] gap-x-4 gap-y-1.5 text-ui [&>div>dt]:min-w-32 @max-[380px]/facts:grid-cols-1 @max-[380px]/facts:gap-y-0.5 @max-[380px]/facts:[&>div>dt]:mt-1.5 @max-[380px]/facts:[&>div>dt]:min-w-0",
					className,
				)}
			>
				{children}
			</dl>
		</div>
	);
}

/** The one-line reason of a failed gate, for `GatedAction` (R7). */
export function gateLine(
	t: DevicesT,
	time: AreaTime,
	result: GateResult | GateFailure | null | undefined,
): Gate | null {
	if (!result || ("ok" in result && result.ok)) return null;
	const failure = result as GateFailure;
	return { kind: failure.kind, reason: gateCopy(t, failure, time).inline };
}

export function ConfigStamp({ read }: Readonly<{ read: ServiceConfigRead }>) {
	return <FreshnessStamp {...stampOf(read.freshness)} />;
}

/** Runs the fix a gate names: an overlay, a key action, or a place in the area. */
export function FixButton({
	fix,
	label,
}: Readonly<{ fix: FixAction; label: string }>) {
	const run = useFixAction();
	const { navigate } = useDevicesRoute();
	return (
		<DvButton
			size="sm"
			data-act="gate-fix"
			onClick={() => {
				const outcome = run(fix);
				if (outcome.kind === "navigate") navigate(outcome.route);
			}}
		>
			{label}
		</DvButton>
	);
}

const SESSION_REASONS = new Set([
	"offline_needs_live",
	"never_connected_needs_live",
]);

function NoDeploy({
	read,
	serviceId,
	have,
}: Readonly<{ read: ServiceConfigRead; serviceId: string; have?: string }>) {
	const { t } = useTranslation("devices");
	const { copied, copy } = useCopy();
	const request = t(
		"serviceConfig.gate.requestText",
		"Please add Deploy & configure for {{service}} on {{device}} to my access.",
		{ service: serviceId, device: read.deviceLabel },
	);
	return (
		<GateNotice
			kind="noaccess"
			title={t(
				"serviceConfig.gate.noDeploy",
				"Needs Deploy & configure on this service to read its settings.",
			)}
			text={t(
				"serviceConfig.gate.noDeployText",
				"Settings include variables, the names of secrets and limits; they're readable only with that permission.",
			)}
			have={have}
			actions={
				<DvButton
					size="sm"
					icon={Copy}
					data-act="copy-request"
					onClick={() => void copy(request)}
				>
					{copied
						? t("serviceConfig.gate.requestCopied", "Copied")
						: t("serviceConfig.gate.request", "Copy a request for the owner")}
				</DvButton>
			}
		/>
	);
}

function GateState({
	read,
	serviceId,
	gate,
}: Readonly<{
	read: ServiceConfigRead;
	serviceId: string;
	gate: GateFailure;
}>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const copy = gateCopy(t, gate, time);
	const fix =
		gate.fix && copy.fix ? <FixButton fix={gate.fix} label={copy.fix} /> : null;
	if (gate.kind === "noaccess")
		return <NoDeploy read={read} serviceId={serviceId} have={copy.have} />;
	if (gate.kind !== "live")
		return (
			<GateNotice
				kind={gate.kind}
				title={copy.title}
				text={copy.text}
				actions={fix}
			/>
		);
	if (SESSION_REASONS.has(gate.copy.code))
		return (
			<StateView
				kind="notloaded"
				title={t("serviceConfig.state.unknown", "Settings aren't known")}
				text={t(
					"serviceConfig.state.unknownText",
					"{{device}} is offline. Settings are read live from the device, and none were read before it went offline.",
					{ device: read.deviceLabel },
				)}
				actions={fix}
			/>
		);
	return (
		<StateView
			kind="notloaded"
			title={t("serviceConfig.state.needsLive", "Needs a live connection")}
			text={t(
				"serviceConfig.state.needsLiveText",
				"Settings are read live from {{device}}.",
				{ device: read.deviceLabel },
			)}
			actions={fix}
		/>
	);
}

/**
 * Why the settings aren't on screen: never "empty" (R6). Renders nothing
 * once settings were read; they then stay with their stamp while a gate fails.
 */
export function ConfigUnavailable({
	read,
	serviceId,
}: Readonly<{ read: ServiceConfigRead; serviceId: string }>) {
	const { t } = useTranslation("devices");
	if (read.configuration) return null;
	if (read.gate)
		return <GateState read={read} serviceId={serviceId} gate={read.gate} />;
	if (read.refused) return <NoDeploy read={read} serviceId={serviceId} />;
	if (!read.service)
		return (
			<StateView
				kind="notloaded"
				title={t(
					"serviceConfig.state.noService",
					"No service {{service}} on {{device}}",
					{ service: serviceId, device: read.deviceLabel },
				)}
				text={t(
					"serviceConfig.state.noServiceText",
					"It may have been removed, or deployed again under another ID.",
				)}
			/>
		);
	if (read.failed)
		return (
			<StateView
				kind="error"
				title={t(
					"serviceConfig.state.failed",
					"The settings couldn't be read from {{device}}",
					{ device: read.deviceLabel },
				)}
				text={
					read.failure
						? t(
								"serviceConfig.state.failedReason",
								"{{device}} answered: “{{reason}}” Nothing was changed.",
								{ device: read.deviceLabel, reason: read.failure },
							)
						: t(
								"serviceConfig.state.failedText",
								"The device answered with something this app can't use, or the connection dropped. Nothing was changed.",
							)
				}
				actions={
					<DvButton
						size="sm"
						icon={RefreshCw}
						data-act="config-retry"
						onClick={() => void read.refresh()}
					>
						{t("serviceConfig.state.retry", "Try again")}
					</DvButton>
				}
			/>
		);
	return (
		<StateView
			kind="loading"
			rows={4}
			title={t("serviceConfig.state.loading", "Reading the settings…")}
		/>
	);
}

export interface Note {
	tone: ResultTone;
	text: ReactNode;
	actions?: ReactNode;
}

/** What the action layer reports while it runs or when it fails, then the screen's own sentence (R9). */
export function ActionResults({
	resultKey,
	note,
	onDismiss,
}: Readonly<{
	resultKey: string;
	note?: Note | null;
	onDismiss?: () => void;
}>) {
	const results = useInlineResults(resultKey).filter(
		(result) => result.state !== "done",
	);
	if (!results.length && !note) return null;
	return (
		<div data-results="" className="flex min-w-0 flex-col gap-1.5">
			{results.map((result) => (
				<InlineResult
					key={result.id}
					tone={result.tone}
					onDismiss={result.dismiss}
				>
					{result.text}
				</InlineResult>
			))}
			{note ? (
				<InlineResult
					tone={note.tone}
					actions={note.actions}
					onDismiss={onDismiss}
				>
					{note.text}
				</InlineResult>
			) : null}
		</div>
	);
}
