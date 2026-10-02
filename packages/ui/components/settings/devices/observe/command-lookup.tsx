"use client";

import { useTranslation } from "@flow-like/locales";
import { Search } from "lucide-react";
import { useEffect, useId, useMemo, useRef, useState } from "react";
import {
	type ManagementResponse,
	managementRejection,
} from "../../../../lib/device-management/types";
import { classifyDeviceError } from "../../../../lib/device-management/workspace/errors";
import type { ActivityItem } from "../../../../lib/device-management/workspace/types";
import { Label } from "../../../ui/label";
import { enumCopy } from "../copy/enum-labels";
import { errorCopy } from "../copy/error-copy";
import { type DevicesT, useAreaTime } from "../primitives/area-context";
import { Block } from "../primitives/block";
import { DvButton } from "../primitives/dv-button";
import { DvInput } from "../primitives/form-fields";
import { FreshnessStamp } from "../primitives/freshness-stamp";
import { type Gate, GatedAction } from "../primitives/gate-notice";
import { InlineResult } from "../primitives/inline-result";
import { KeyValueList, KvRow } from "../primitives/key-value-list";
import { activityTitle } from "../shell/activity-tray";
import { useActivity, useDeviceCall, useGate } from "../workspace";
import { livePhase } from "./live-state";
import { asRecord, whole, word } from "./observe-data";
import { type ObserveTarget, gateLine } from "./use-observe-target";

const COMMAND_ID = /^[A-Za-z0-9_:.-]{1,128}$/u;
const RESPONSE_STATES = [
	"completed",
	"accepted",
	"pending",
	"rejected",
	"failed",
	"staging",
	"draining",
	"requesting",
	"requested",
	"unknown",
	"rolled_back",
] as const;
type ResponseState = (typeof RESPONSE_STATES)[number];

interface Lookup {
	id: string;
	/** Unix seconds. */
	readAt: number;
	state: ResponseState | null;
	service?: string;
	settings?: number;
	instances?: number;
	/** The device's own sentence. */
	says?: string;
	/** The device has no record: unknown, older than 24 hours or sent by someone else. */
	missing: boolean;
}

interface Prefill {
	id: string;
	issuedAt?: number;
	unconfirmed: boolean;
}

function operationOf(
	item: ActivityItem,
): { id: string; issuedAt?: number } | undefined {
	const resume = item.resume;
	if (!resume || !("operationId" in resume)) return undefined;
	return {
		id: resume.operationId,
		...(resume.type === "operation" ? { issuedAt: resume.issuedAt } : {}),
	};
}

/** The newest command without a seen result, else the newest command sent from this computer. */
function prefillOf(items: readonly ActivityItem[]): Prefill | undefined {
	const sent = items
		.flatMap((item) => {
			const operation = operationOf(item);
			return operation ? [{ item, operation }] : [];
		})
		.sort((a, b) => b.item.startedAt - a.item.startedAt);
	const open = sent.find(({ item }) => item.state === "unknown") ?? sent[0];
	return open
		? { ...open.operation, unconfirmed: open.item.state === "unknown" }
		: undefined;
}

const isResponseState = (value: string): value is ResponseState =>
	(RESPONSE_STATES as readonly string[]).includes(value);

/** What the device answered for a command ID, reduced to what a person can read (R3: never the raw result). */
function lookupOf(
	id: string,
	response: ManagementResponse,
	readAt: number,
): Lookup {
	const rejected = response.state === "rejected";
	const result = asRecord(response.result) ?? {};
	return {
		id,
		readAt,
		missing: rejected,
		state: isResponseState(response.state) ? response.state : null,
		service: word(result.placement_id),
		settings: whole(result.config_revision),
		instances: whole(result.desired_replicas),
		says: rejected ? managementRejection(response)?.error : word(result.error),
	};
}

type DeviceErrorCode = ReturnType<typeof classifyDeviceError>["code"];

/** Asks the device for one command's result; a failed read comes back as its error code. */
async function readCommand(
	call: ReturnType<typeof useDeviceCall>,
	id: string,
	readAt: number,
): Promise<{ found: Lookup } | { failure: DeviceErrorCode }> {
	try {
		const response = await call({ type: "operation", operation_id: id });
		return { found: lookupOf(id, response, readAt) };
	} catch (error) {
		return { failure: classifyDeviceError(error).code };
	}
}

/** Says where the prefilled ID came from, until the person types another one. */
function prefillHint(
	t: DevicesT,
	prefill: Prefill | undefined,
	typed: string | null,
): string | undefined {
	if (!prefill || (typed !== null && typed !== prefill.id)) return undefined;
	return prefill.unconfirmed
		? t(
				"devices:observe.lookup.prefilledUnconfirmed",
				"Prefilled from the last command you sent whose result you haven't seen.",
			)
		: t(
				"devices:observe.lookup.prefilled",
				"Prefilled from the last command sent from this computer.",
			);
}

/** The tray item of a command sent from this computer. */
const sentItem = (items: readonly ActivityItem[], id: string) =>
	items.find((item) => operationOf(item)?.id === id);

function LookupStamp({
	target,
	found,
}: Readonly<{ target: ObserveTarget; found: Lookup | null }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	// A look-up is read once: the stamp's default wording would promise a refresh.
	if (found)
		return (
			<FreshnessStamp
				source="live"
				age="live"
				observedAt={found.readAt}
				text={t("observe.lookup.readAgo", "read {{ago}} · on demand", {
					ago: time.ago(Math.min(found.readAt, time.nowS)),
				})}
			/>
		);
	return livePhase(target) === "open" ? (
		<FreshnessStamp
			source="live"
			age="live"
			text={t("observe.lookup.onDemand", "read on demand")}
		/>
	) : (
		<FreshnessStamp
			source="live"
			age="notloaded"
			text={t("observe.lookup.needsLive", "needs a live connection")}
		/>
	);
}

function LookupResult({
	target,
	found,
	sent,
	onDismiss,
}: Readonly<{
	target: ObserveTarget;
	found: Lookup;
	/** The tray item, when the command was sent from this computer. */
	sent: ActivityItem | undefined;
	onDismiss(): void;
}>) {
	const { t } = useTranslation("devices");
	if (found.missing)
		return (
			<InlineResult tone="unknown" onDismiss={onDismiss}>
				{t(
					"observe.lookup.missing",
					"{{device}} has no result for this command. It may be older than 24 hours, sent by someone else, or never have arrived.",
					{ device: target.name },
				)}
			</InlineResult>
		);
	const state = found.state
		? enumCopy(t, "responseState", found.state)
		: undefined;
	return (
		<KeyValueList>
			<KvRow label={t("observe.lookup.state", "Result")}>
				{state?.label ?? t("observe.lookup.stateUnknown", "Unknown")}
				{state?.explain ? (
					<span className="ml-1.5 text-xs text-muted-foreground">
						{state.explain}
					</span>
				) : null}
			</KvRow>
			{sent ? (
				<KvRow label={t("observe.lookup.command", "Command")}>
					{activityTitle(t, sent)}
				</KvRow>
			) : null}
			<KvRow label={t("observe.lookup.target", "Target")}>
				{found.service ? (
					<span className="font-mono">{found.service}</span>
				) : (
					t("observe.lookup.wholeDevice", "{{device}} (the whole device)", {
						device: target.name,
					})
				)}
			</KvRow>
			{found.settings === undefined ? null : (
				<KvRow label={t("observe.lookup.settings", "Settings")}>
					{t("observe.lookup.settingsValue", "v{{version, number}}", {
						version: found.settings,
					})}
				</KvRow>
			)}
			{found.instances === undefined ? null : (
				<KvRow label={t("observe.lookup.instances", "Instances requested")}>
					<span className="font-mono tabular-nums">{found.instances}</span>
				</KvRow>
			)}
			{found.says ? (
				<KvRow label={t("observe.lookup.says", "The device says")}>
					<q>{found.says}</q>
				</KvRow>
			) : null}
		</KeyValueList>
	);
}

export function CommandLookup({ target }: Readonly<{ target: ObserveTarget }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const fieldId = useId();
	const { deviceId, serviceId } = target;
	const call = useDeviceCall(deviceId);
	const tray = useActivity(serviceId ? { deviceId, serviceId } : { deviceId });
	const prefill = useMemo(() => prefillOf(tray.items), [tray.items]);
	const [typed, setTyped] = useState<string | null>(null);
	const [busy, setBusy] = useState(false);
	const [found, setFound] = useState<Lookup | null>(null);
	const [failure, setFailure] = useState<string | null>(null);
	const alive = useRef(true);
	useEffect(() => {
		alive.current = true;
		return () => {
			alive.current = false;
		};
	}, []);

	const value = typed ?? prefill?.id ?? "";
	const id = value.trim();
	const known = sentItem(tray.items, id);
	const issuedAt = known ? operationOf(known)?.issuedAt : undefined;
	const gate = useGate(
		"check_unconfirmed",
		deviceId,
		issuedAt === undefined
			? undefined
			: { extra: { operationIssuedAt: issuedAt } },
	);
	const invalid = id !== "" && !COMMAND_ID.test(id);
	const refused = gateLine(t, gate, time);
	// The shared sentence for a locked device speaks of changing settings; a look-up changes nothing.
	const lookUpGate: Gate | null =
		refused?.kind === "locked"
			? {
					kind: "locked",
					reason: t(
						"observe.lookup.locked",
						"Unlock {{device}} to look up a command.",
						{ device: target.name },
					),
				}
			: refused;

	const lookUp = async () => {
		if (!id || invalid || busy) return;
		setBusy(true);
		setFailure(null);
		const outcome = await readCommand(call, id, Math.floor(time.now / 1000));
		if (!alive.current) return;
		setBusy(false);
		if ("found" in outcome) setFound(outcome.found);
		else setFailure(errorCopy(t, outcome.failure));
	};

	const hint = prefillHint(t, prefill, typed);

	return (
		<Block
			id="observe-lookup"
			icon={Search}
			title={t("observe.lookup.title", "Look up a command")}
			stamp={<LookupStamp target={target} found={found} />}
		>
			<div className="flex min-w-0 flex-col gap-1.5" data-field="">
				<Label htmlFor={fieldId} className="text-[13px]/[18px] font-medium">
					{t("observe.lookup.label", "Command ID")}
				</Label>
				<div className="flex flex-wrap items-start gap-2">
					<DvInput
						id={fieldId}
						mono
						value={value}
						autoComplete="off"
						spellCheck={false}
						aria-invalid={invalid || undefined}
						aria-describedby={`${fieldId}-hint`}
						className="min-w-0 flex-[1_1_240px]"
						onChange={(event) => setTyped(event.target.value)}
						onKeyDown={(event) => {
							if (event.key === "Enter" && gate.ok) void lookUp();
						}}
					/>
					<GatedAction gate={lookUpGate}>
						<DvButton
							icon={Search}
							busy={busy}
							disabled={!id || invalid}
							onClick={() => void lookUp()}
						>
							{t("observe.lookup.action", "Look up")}
						</DvButton>
					</GatedAction>
				</div>
				<p id={`${fieldId}-hint`} className="text-xs text-muted-foreground">
					{invalid ? (
						<span className="text-critical">
							{t(
								"observe.lookup.invalid",
								"A command ID has letters, digits and dashes only.",
							)}
						</span>
					) : (
						(hint ??
						t(
							"observe.lookup.hint",
							"The ID is on every command in the timeline and in the activity tray.",
						))
					)}
				</p>
			</div>
			{failure ? (
				<InlineResult tone="critical" onDismiss={() => setFailure(null)}>
					{failure}
				</InlineResult>
			) : null}
			{found ? (
				<LookupResult
					target={target}
					found={found}
					sent={sentItem(tray.items, found.id)}
					onDismiss={() => setFound(null)}
				/>
			) : null}
			<p className="text-xs text-muted-foreground">
				{t(
					"observe.lookup.kept",
					"Results are kept for 24 hours and only for the person who sent the command.",
				)}
			</p>
		</Block>
	);
}
