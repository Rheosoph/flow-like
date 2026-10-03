"use client";

import { useTranslation } from "@flow-like/locales";
import { CloudUpload, ShieldCheck, TriangleAlert } from "lucide-react";
import { type ReactNode, useCallback, useState } from "react";
import type { LocalSummary } from "../../../../lib/device-management/workspace/types";
import type { DevicesT } from "../primitives/area-context";
import { Banner } from "../primitives/banner";
import { DvButton } from "../primitives/dv-button";
import { GatedAction } from "../primitives/gate-notice";
import { InlineResult } from "../primitives/inline-result";
import { useKeyResults } from "./key-store";
import { useFlowGuard, useKeyActions } from "./use-key-actions";

export const STORAGE_RESULT = "storage";

/** The browser may delete this site's data, and with it the only copy of some keys. */
export function storageAtRisk(local: LocalSummary): boolean {
	return (
		local.platform === "web" &&
		(local.persistence === "denied" || local.persistence === "unavailable")
	);
}

export function storageLabel(t: DevicesT, local: LocalSummary): string {
	if (local.platform === "desktop")
		return t("devices:keys.storage.desktop", "Desktop app · kept safely");
	if (local.persistence === "persisted")
		return t("devices:keys.storage.webSafe", "Web · kept safely");
	if (local.persistence === "unknown")
		return t("devices:keys.storage.webChecking", "Web · checking storage…");
	return t(
		"devices:keys.storage.webRisk",
		"Web · this browser may delete keys",
	);
}

export function storageHint(t: DevicesT, local: LocalSummary): string {
	if (local.platform === "desktop")
		return t(
			"devices:keys.storage.desktopHint",
			"The desktop app keeps its storage until you delete it.",
		);
	if (local.persistence === "persisted")
		return t(
			"devices:keys.storage.webSafeHint",
			"The browser granted persistent storage, so it won't delete the keys on its own. Clearing this site's data still removes them.",
		);
	if (local.persistence === "unavailable")
		return t(
			"devices:keys.storage.webUnavailableHint",
			"This browser can't promise to keep site data. Back the keys up to your account, or use the desktop app.",
		);
	return t(
		"devices:keys.storage.webRiskHint",
		"This site hasn't been granted persistent storage. Clearing site data, or the browser running low on disk, can remove the keys.",
	);
}

/** "Keep keys safely": asks the browser for persistent storage and reports what it answered. */
export function useKeepKeysSafe(keysHere: number) {
	const { t } = useTranslation("devices");
	const actions = useKeyActions();
	const results = useKeyResults();
	const guard = useFlowGuard();
	const [busy, setBusy] = useState(false);
	const run = useCallback(async () => {
		if (busy) return;
		const flow = guard();
		setBusy(true);
		try {
			const persistence = await actions.keepSafe();
			const at = actions.timeNow();
			if (persistence === "persisted")
				results.put(
					STORAGE_RESULT,
					"good",
					t("keys.storage.granted", {
						at,
						count: keysHere,
						defaultValue_one:
							"The browser granted persistent storage at {{at}}. Keys for {{count, number}} device are kept safely here.",
						defaultValue_other:
							"The browser granted persistent storage at {{at}}. Keys for {{count, number}} devices are kept safely here.",
					}),
				);
			else
				results.put(
					STORAGE_RESULT,
					"warning",
					persistence === "denied"
						? t(
								"keys.storage.refused",
								"The browser didn't grant persistent storage at {{at}}. Some browsers only grant it to sites you use often or have installed. Back the keys up to your account.",
								{ at },
							)
						: t(
								"keys.storage.unavailable",
								"This browser can't keep site data safely. Back the keys up to your account, or use the desktop app.",
							),
				);
		} finally {
			if (flow.alive()) setBusy(false);
		}
	}, [busy, guard, actions, results, t, keysHere]);
	return { busy, run };
}

export function StorageResult() {
	const results = useKeyResults();
	const result = results.of(STORAGE_RESULT);
	if (!result) return null;
	return (
		<InlineResult
			tone={result.tone}
			onDismiss={() => results.dismiss(STORAGE_RESULT)}
		>
			{result.text}
		</InlineResult>
	);
}

/** SPEC §5.9 `web-risk`: the page-level warning with "Keep keys safely" as the one primary action. */
export function PersistenceNotice({
	local,
	keysHere,
	onShowBackups,
	extra,
}: Readonly<{
	local: LocalSummary;
	keysHere: number;
	onShowBackups?: () => void;
	extra?: ReactNode;
}>) {
	const { t } = useTranslation("devices");
	const keep = useKeepKeysSafe(keysHere);
	if (!storageAtRisk(local) || keysHere === 0) return null;
	const unavailable = local.persistence === "unavailable";
	const button = (
		<DvButton
			variant={unavailable ? "default" : "primary"}
			size="sm"
			icon={ShieldCheck}
			busy={keep.busy}
			onClick={() => void keep.run()}
		>
			{t("keys.storage.keepSafe", "Keep keys safely")}
		</DvButton>
	);
	return (
		<Banner
			tone="warning"
			icon={TriangleAlert}
			title={t(
				"keys.storage.bannerTitle",
				"This site hasn't been granted persistent storage.",
			)}
			actions={
				<>
					<GatedAction
						gate={
							unavailable
								? {
										kind: "platform",
										reason: t(
											"keys.storage.cantAsk",
											"This browser has no way to ask for it.",
										),
									}
								: null
						}
					>
						{button}
					</GatedAction>
					{onShowBackups ? (
						<DvButton size="sm" icon={CloudUpload} onClick={onShowBackups}>
							{t("keys.storage.showBackups", "Check what's backed up")}
						</DvButton>
					) : null}
					{extra}
				</>
			}
		>
			{t("keys.storage.bannerText", {
				count: keysHere,
				defaultValue_one:
					"Clearing site data, or the browser running low on disk, can remove the keys for {{count, number}} device. Without keys you can't manage it from here. Backups on your account aren't affected.",
				defaultValue_other:
					"Clearing site data, or the browser running low on disk, can remove the keys for {{count, number}} devices. Without keys you can't manage those devices from here. Backups on your account aren't affected.",
			})}
		</Banner>
	);
}
