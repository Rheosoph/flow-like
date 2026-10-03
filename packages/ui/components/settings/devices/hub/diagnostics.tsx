"use client";

import { useTranslation } from "@flow-like/locales";
import { useState } from "react";
import type { HubStandalone } from "../../../../lib/device-management/hub/endpoints";
import type {
	ReleaseFacts,
	VerifiedRelease,
} from "../../../../lib/device-package";
import { type AreaTime, useAreaTime } from "../primitives/area-context";
import { InlineResult } from "../primitives/inline-result";
import { copyText } from "../primitives/use-copy";
import { useDeviceWorkspace } from "../workspace";
import { type HubView, releaseVerdictOf } from "./hub-view";
import type { HubLimits } from "./use-hub-facts";

/* The report is plain English for the hub operator and never translated (SPEC §3.10). */

const SUPPORT_REPORT = {
	checking: "checking",
	on: "on (standalone.enabled = true)",
	off: "off (standalone.enabled = false)",
};

const supportLines = (view: HubView, time: AreaTime) => {
	const { state, error } = view.hub.support;
	const code = error ? error.code : "no answer";
	const support =
		state === "unreachable" ? `unreachable (${code})` : SUPPORT_REPORT[state];
	const refresh =
		state === "on" && error ? [`Hub refresh: failing (${code})`] : [];
	const api = view.record?.api_base_url ?? `${view.origin}/api/v1`;
	return [
		`Flow-Like hub status · ${view.host}`,
		`Report time: ${time.abs(time.nowS)}`,
		`Device support: ${support}`,
		...refresh,
		`Public device API: ${api}`,
	];
};

const readinessLines = (view: HubView, time: AreaTime) => {
	const { data, error, freshness } = view.readiness;
	if (!data) {
		const why = error ? `not run (${error.code})` : "checking";
		return [`Readiness: ${why}`];
	}
	const at = freshness.at ?? freshness.dataFrom;
	const when = at === undefined ? "at an unknown time" : time.abs(at);
	const rows = data.checks.map((check) => {
		const verdict = check.ready ? "ready" : "NOT READY";
		return `  [${verdict}] ${check.id} · "${check.message}"`;
	});
	return [`Readiness (checked ${when}):`, ...rows];
};

const limitsLine = (limits: HubLimits) => {
	const shown = (value: number | undefined) => value ?? "?";
	return [
		`Limits: max_devices_per_user=${shown(limits.devices)}`,
		`max_pending_enrollments_per_user=${shown(limits.pending)}`,
		`enrollment_ttl_seconds=${shown(limits.lifetimeS)}`,
		`daily package cap=${shown(limits.perDay)}`,
	].join(" · ");
};

const trustLines = (record: HubStandalone | undefined) => {
	if (!record) return [];
	const trust = record.release_trust;
	if (!trust)
		return ["Release trust: not configured (no manifest URL, no keys)"];
	const keys = `${trust.public_keys.length} (${trust.public_keys.join(", ")})`;
	return [
		`Release trust: manifest_url=${trust.manifest_url} · public_keys=${keys} · minimum_sequence=${trust.minimum_sequence}`,
	];
};

const factsText = (facts: ReleaseFacts, time: AreaTime) =>
	`${facts.release_version} · sequence ${facts.sequence} · issued ${time.abs(facts.issued_at)} · expires ${time.abs(facts.expires_at)}`;

const manifestLines = (
	{ manifest }: VerifiedRelease,
	state: string,
	time: AreaTime,
) => {
	const targets = manifest.artifacts.map(
		(artifact) =>
			`${artifact.target} (${artifact.size} B, sha256 ${artifact.sha256})`,
	);
	const image = manifest.container;
	const container = image
		? [`  container: ${image.image} [${image.platforms.join(", ")}]`]
		: [];
	return [
		`Current release (${state}): ${factsText(manifest, time)} · state_schema_version ${manifest.state_schema_version}`,
		`  targets: ${targets.join("; ")}`,
		...container,
	];
};

/** The same verdict the page shows: a list that failed a check is never reported as the current release. */
const releaseLines = (view: HubView, time: AreaTime) => {
	const verdict = releaseVerdictOf(view.record, view.release, time.nowS);
	switch (verdict.kind) {
		case "ok":
		case "ends_soon": {
			const { unreached } = verdict;
			const state = `verified${verdict.kind === "ends_soon" ? `, ${verdict.daysLeft} days left` : ""}${unreached ? `, last check did not reach the list (${unreached.code})` : ""}`;
			return manifestLines(verdict.release, state, time);
		}
		case "expired":
			return verdict.release
				? manifestLines(verdict.release, "expired", time)
				: [`Current release: expired · ${factsText(verdict.facts, time)}`];
		case "failed":
			return [
				`Current release: failed check "${verdict.check}" (${verdict.detail})${verdict.facts ? ` · ${factsText(verdict.facts, time)}` : ""}`,
			];
		case "unfetched":
			return [
				`Current release: not fetched (${verdict.error.code}: ${verdict.error.message})`,
			];
		default:
			return [];
	}
};

const tierLines = (record: HubStandalone | undefined) => {
	const tiers = Object.entries(record?.telemetry_tiers ?? {}).map(
		([key, tier]) => `${key} ${tier.max_bytes} B / ${tier.retention_seconds} s`,
	);
	return tiers.length ? [`Telemetry tiers: ${tiers.join(" · ")}`] : [];
};

/** What a hub operator needs to see the page's state: no keys, passwords or device data. */
export const diagnosticsText = (view: HubView, time: AreaTime) =>
	[
		...supportLines(view, time),
		"",
		...readinessLines(view, time),
		"",
		limitsLine(view.limits),
		...trustLines(view.record),
		...releaseLines(view, time),
		...tierLines(view.record),
		"",
		"Contains no keys, passwords or device data.",
	].join("\n");

export interface Diagnostics {
	copied?: { at: number; text: string; ok: boolean };
	copy(): Promise<void>;
	dismiss(): void;
}

export function useDiagnostics(view: HubView): Diagnostics {
	const time = useAreaTime();
	const workspace = useDeviceWorkspace();
	const [copied, setCopied] = useState<Diagnostics["copied"]>();
	return {
		...(copied ? { copied } : {}),
		copy: async () => {
			const text = diagnosticsText(view, time);
			const ok = await copyText(text);
			setCopied({ at: Math.floor(workspace.clock.now() / 1000), text, ok });
		},
		dismiss: () => setCopied(undefined),
	};
}

/** The outcome of "Copy diagnostics", with the report itself behind a disclosure. */
export function DiagnosticsResult({
	diagnostics,
}: Readonly<{ diagnostics: Diagnostics }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const { copied } = diagnostics;
	if (!copied) return null;
	return (
		<div data-hub="diagnostics" className="flex min-w-0 flex-col gap-1.5">
			<InlineResult
				tone={copied.ok ? "good" : "warning"}
				onDismiss={diagnostics.dismiss}
			>
				{copied.ok
					? t(
							"hub.diagnostics.copied",
							"Copied a plain-text report at {{time}} for your hub operator: device support, the check results, limits and the release. It contains no keys, passwords or device data.",
							{ time: time.clock(copied.at) },
						)
					: t(
							"hub.diagnostics.notCopied",
							"This browser didn't allow copying. Open the report below and copy it by hand. It contains no keys, passwords or device data.",
						)}
			</InlineResult>
			<details open={!copied.ok} className="min-w-0">
				<summary className="cursor-pointer text-xs text-muted-foreground">
					{t("hub.diagnostics.show", "Show the report")}
				</summary>
				<pre className="mt-1.5 max-h-60 overflow-auto rounded-lg border border-border bg-surface-sunken px-3 py-2.5 font-mono text-xs/[18px] whitespace-pre text-ink-2">
					{copied.text}
				</pre>
			</details>
		</div>
	);
}
