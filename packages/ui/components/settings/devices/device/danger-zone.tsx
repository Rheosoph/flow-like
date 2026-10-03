"use client";

import { Trans, useTranslation } from "@flow-like/locales";
import { Power } from "lucide-react";
import { type ReactNode, useEffect, useRef } from "react";
import { Block } from "../primitives/block";
import { DvButton } from "../primitives/dv-button";
import { InlineResult } from "../primitives/inline-result";
import { useInlineResults } from "../workspace";
import { useRevokeFlow } from "./revoke-flow";
import type { DevicePage } from "./use-device-page";

export interface DangerZoneProps {
	page: DevicePage;
	/** The settings tab passes its layer title; on its own the block reads "Danger zone". */
	title?: ReactNode;
	/** The route asked for the revoke flow (`action=revoke`): open it once. */
	autoOpen: boolean;
	/** Called when the flow opened from the route, so the one-shot param can be dropped. */
	onAutoOpened(): void;
}

/** Device settings › Danger zone (owner only): revoking is permanent and happens at the hub. */
export function DangerZone({
	page,
	title,
	autoOpen,
	onAutoOpened,
}: Readonly<DangerZoneProps>) {
	const { t } = useTranslation("devices");
	const flow = useRevokeFlow(page);
	const results = useInlineResults(flow.resultKey);
	const opened = useRef(false);
	const run = flow.run;
	const latest = useRef({ run, onAutoOpened });
	useEffect(() => {
		latest.current = { run, onAutoOpened };
	});
	useEffect(() => {
		if (!autoOpen || opened.current) return;
		opened.current = true;
		latest.current.onAutoOpened();
		void latest.current.run();
	}, [autoOpen]);
	return (
		<Block
			danger
			id="danger-zone"
			icon={title ? undefined : Power}
			title={title ?? t("device.danger.title", "Danger zone")}
		>
			<p className="max-w-[78ch] text-ui">
				<Trans
					t={t}
					i18nKey="device.danger.text"
					defaults="<1>Revoke this device.</1> The hub refuses <2/> for good and everyone loses access. Services on it keep running until someone stops them there. It works while the device is offline and doesn't need your device password."
					components={{
						1: <b className="font-semibold" />,
						2: <span className="font-mono">{page.name}</span>,
					}}
				/>
			</p>
			<div className="flex flex-wrap items-center gap-2">
				<DvButton
					variant="danger-ghost"
					icon={Power}
					busy={flow.pending}
					onClick={() => void flow.run()}
				>
					{t("device.danger.revoke", "Revoke {{device}}…", {
						device: page.name,
					})}
				</DvButton>
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
		</Block>
	);
}
