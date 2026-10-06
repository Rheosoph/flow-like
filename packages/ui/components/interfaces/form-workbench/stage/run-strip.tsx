"use client";

import { useTranslation } from "@flow-like/locales";
import {
	type KeyboardEvent,
	useCallback,
	useEffect,
	useMemo,
	useRef,
} from "react";
import { cx } from "../../../settings/devices/primitives/tone";
import type {
	FormSessionActions,
	RunEntry,
	WorkbenchViewProps,
} from "../contracts";
import { Dock } from "../dock/dock";
import type { FormWords } from "../model/date-text";
import { isComposing } from "./key-guards";
import { OverflowMenu } from "./overflow-menu";
import { type PaneEnv, tabIdOf } from "./pane-env";
import { RunTab } from "./run-tab";
import {
	MAX_CHIPS,
	type StripTabs,
	rovingTarget,
	showsPin,
	takeVisible,
} from "./strip-model";

export interface RunStripProps {
	readonly view: WorkbenchViewProps;
	readonly env: PaneEnv;
	readonly tabs: StripTabs;
	readonly now: number;
	readonly selectedId: string | null;
	readonly pinnedId: string | null;
	readonly canPin: boolean;
	/** A form without inputs: the strip carries the coral Run at its left end. */
	readonly zeroFields: boolean;
	readonly locale: string;
	readonly words: FormWords;
}

/** ←/→/Home/End move between tabs and the stage follows (spec M5); nothing runs. */
function useRoving(
	runs: readonly RunEntry[],
	uid: string,
	actions: FormSessionActions,
) {
	return useCallback(
		(run: RunEntry) => (event: KeyboardEvent<HTMLButtonElement>) => {
			if (isComposing(event)) return;
			const to = rovingTarget(
				event.key,
				runs.findIndex((item) => item.id === run.id),
				runs.length,
			);
			const next = to === null ? undefined : runs[to];
			if (!next) return;
			event.preventDefault();
			if (next.id === run.id) return;
			actions.selectRun(next.id, "keyboard");
			event.currentTarget.ownerDocument
				.getElementById(tabIdOf(uid, next.id))
				?.focus();
		},
		[runs, uid, actions],
	);
}

/** Only one tab is a Tab stop: the selected one, else the first. */
const tabbableId = (runs: readonly RunEntry[], selectedId: string | null) =>
	runs.some((run) => run.id === selectedId) ? selectedId : runs[0]?.id;

/** 40 px (44 px with the zero-field Run button); a touch screen needs 44 px targets, so one step more. */
const stripHeight = (zeroFields: boolean, touch: boolean) => {
	if (zeroFields) return touch ? "h-12" : "h-11";
	return touch ? "h-11" : "h-10";
};

function togglePin(env: PaneEnv, pinnedId: string | null) {
	return (run: RunEntry) =>
		env.actions.pinRun(run.id === pinnedId ? null : run.id);
}

/** The desktop strip: a 40 px row of tabs, newest first; "N more" for the rest. */
export function RunStrip({
	view,
	env,
	tabs,
	now,
	selectedId,
	pinnedId,
	canPin,
	zeroFields,
	locale,
	words,
}: Readonly<RunStripProps>) {
	const { t } = useTranslation("interfaces");
	const keyDownOf = useRoving(tabs.visible, env.uid, env.actions);
	const tabbable = tabbableId(tabs.visible, selectedId);
	const onPin = togglePin(env, pinnedId);
	return (
		<div
			data-fw-strip=""
			className={cx(
				"flex shrink-0 items-stretch border-hairline border-b bg-surface-sunken pr-2",
				zeroFields && "pl-3",
				stripHeight(zeroFields, env.touch),
			)}
		>
			{zeroFields ? (
				<div className="flex items-center pl-1">
					<Dock {...view} variant="strip" />
				</div>
			) : null}
			<div
				role="tablist"
				aria-label={t("workbench.stage.strip.label", "Runs")}
				className="flex min-w-0 items-stretch"
			>
				{tabs.visible.map((run) => (
					<RunTab
						key={run.id}
						run={run}
						uid={env.uid}
						now={now}
						selected={run.id === selectedId}
						tabbable={run.id === tabbable}
						pinned={run.id === pinnedId}
						showPin={showsPin(run, { canPin, selectedId, pinnedId })}
						chip={false}
						touch={env.touch}
						onSelect={(item) => env.actions.selectRun(item.id, "tab")}
						onPin={onPin}
						onKeyDown={keyDownOf(run)}
					/>
				))}
			</div>
			<span className="flex-1" />
			{tabs.overflow.length > 0 ? (
				<div className="flex items-center">
					<OverflowMenu
						state={view.state}
						env={env}
						tabs={tabs}
						now={now}
						locale={locale}
						words={words}
					/>
				</div>
			) : null}
		</div>
	);
}

/** The narrow layout's chips: a horizontally scrolling row of every run, the selected one kept in view. */
export function RunChips({
	runs: all,
	env,
	now,
	selectedId,
}: Readonly<{
	runs: readonly RunEntry[];
	env: PaneEnv;
	now: number;
	selectedId: string | null;
}>) {
	const { t } = useTranslation("interfaces");
	const runs = useMemo(
		() => takeVisible(all, selectedId, null, MAX_CHIPS),
		[all, selectedId],
	);
	const keyDownOf = useRoving(runs, env.uid, env.actions);
	const tabbable = tabbableId(runs, selectedId);
	const rowRef = useRef<HTMLDivElement>(null);
	useEffect(() => {
		if (selectedId === null) return;
		const chip = rowRef.current?.querySelector<HTMLElement>(
			'[aria-selected="true"]',
		);
		chip?.scrollIntoView?.({ inline: "nearest", block: "nearest" });
	}, [selectedId]);
	return (
		<div
			ref={rowRef}
			role="tablist"
			aria-label={t("workbench.stage.strip.label", "Runs")}
			className="no-scrollbar flex shrink-0 items-center gap-1.5 overflow-x-auto px-4 pt-2 pb-1"
		>
			{runs.map((run) => (
				<RunTab
					key={run.id}
					run={run}
					uid={env.uid}
					now={now}
					selected={run.id === selectedId}
					tabbable={run.id === tabbable}
					pinned={false}
					showPin={false}
					chip
					touch={env.touch}
					onSelect={(item) => env.actions.selectRun(item.id, "tab")}
					onKeyDown={keyDownOf(run)}
				/>
			))}
		</div>
	);
}
