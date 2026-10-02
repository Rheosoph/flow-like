"use client";

import { useTranslation } from "@flow-like/locales";
import { Cloud, HardDrive } from "lucide-react";
import { useState } from "react";
import { appCopy } from "../copy/app-copy";
import { useAreaTime } from "../primitives/area-context";
import { Block } from "../primitives/block";
import { DvSheet } from "../primitives/dv-sheet";
import { FreshnessStamp } from "../primitives/freshness-stamp";
import { HowRunsStrip, ModeExplainer } from "../primitives/how-runs";
import { cx } from "../primitives/tone";
import { TABLE_RESET, useAppPage } from "./app-shared";
import { versionName } from "./app-view-local";

/*
 * The strip's sentence keeps the first line with the chips and the newest
 * version wraps under it (prototype); the app's base layer would give the
 * sentence a 28 px line height.
 */
const STRIP = "[&>p]:flex-[1_1_480px] [&>p]:text-ui [&>span:empty]:hidden";

/* The explainer's own table and paragraphs, freed from the same base-layer rules (see `TABLE_RESET`). */
const EXPLAINER = cx("[&_p]:text-ui [&_table]:my-0", TABLE_RESET);

/** APP §2.5: how the app runs on devices, stated as a fact, with "Online or offline?" in a sheet. */
export function HowRunsBlock() {
	const { t } = useTranslation("devices");
	const { view } = useAppPage();
	const [open, setOpen] = useState(false);
	const copy = appCopy(t);
	const { mode, visibility, newest } = view.howRuns;
	return (
		<>
			<HowRunsStrip
				app={view.app.name}
				visibility={visibility}
				mode={mode}
				{...(newest?.builtAt
					? {
							newest: {
								label: versionName(newest),
								hash: newest.hash,
								at: newest.builtAt,
							},
						}
					: {})}
				onExplain={() => setOpen(true)}
				className={STRIP}
			/>
			<DvSheet
				open={open}
				onOpenChange={setOpen}
				wide
				icon={mode === "online" ? Cloud : HardDrive}
				title={copy.explainerTitle()}
				sub={copy.explainerSub(mode, view.app.name)}
			>
				<ModeExplainer app={view.app.name} mode={mode} className={EXPLAINER} />
			</DvSheet>
		</>
	);
}

/** The same comparison inline, for the never-deployed layout (APP §2.18). */
export function ModeBlock() {
	const { t } = useTranslation("devices");
	const { view, data } = useAppPage();
	const time = useAreaTime();
	const copy = appCopy(t);
	const { mode } = view.howRuns;
	const checkedAt = view.app.localOnly ? undefined : data.readAt;
	return (
		<Block
			id="ad-mode"
			icon={mode === "online" ? Cloud : HardDrive}
			title={copy.explainerTitle()}
			stamp={
				<FreshnessStamp
					source={view.app.localOnly ? "local" : "hub"}
					age="current"
					text={
						checkedAt
							? t(
									"app.howRuns.stampChecked",
									"app settings · checked {{ago}}",
									{ ago: time.ago(checkedAt) },
								)
							: t("app.howRuns.stamp", "app settings")
					}
					{...(checkedAt ? { observedAt: checkedAt } : {})}
				/>
			}
		>
			<ModeExplainer app={view.app.name} mode={mode} className={EXPLAINER} />
		</Block>
	);
}
