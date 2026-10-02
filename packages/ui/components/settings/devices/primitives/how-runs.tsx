"use client";

import { Trans, useTranslation } from "@flow-like/locales";
import { Check, Info } from "lucide-react";
import { EXPLAINER_ROWS, appCopy } from "../copy/app-copy";
import {
	type AppModeValue,
	type AppVisibilityValue,
	MODE_ICON,
	ModeChip,
	VisibilityChip,
} from "./app-chips";
import { type AreaTime, useAreaTime } from "./area-context";
import { DvButton } from "./dv-button";
import { DvTable, Td, Th, Tr } from "./dv-table";
import { IdRef } from "./id-ref";
import { StatusChip } from "./status-chip";
import { cx } from "./tone";

const MODES: readonly AppModeValue[] = ["online", "offline"];

/* App names go into <Trans> as component children: as `values` they are parsed as markup. */
const MODE_WORD = <b className="font-semibold text-foreground" />;

function sameDay(aS: number, bS: number) {
	return (
		new Date(aS * 1000).toDateString() === new Date(bS * 1000).toDateString()
	);
}

function useDayTime(time: AreaTime) {
	const { t } = useTranslation("devices");
	return (atS: number) =>
		sameDay(atS, time.nowS)
			? t("view.howRuns.today", "today {{time}}", { time: time.at(atS) })
			: time.at(atS);
}

export interface NewestVersion {
	/** "v1.5.0". */
	label: string;
	hash: string;
	/** Unix seconds: built on the hub, or changed on this computer (local-only). */
	at: number;
}

/**
 * APP §2.5: one line under the app header stating how the app runs on
 * devices as a fact (A1), with the newest version and "Online or offline?".
 */
export function HowRunsStrip({
	app,
	visibility,
	mode,
	newest,
	onExplain,
	className,
}: Readonly<{
	app: string;
	visibility: AppVisibilityValue;
	mode: AppModeValue;
	newest?: NewestVersion;
	onExplain?: () => void;
	className?: string;
}>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const dayTime = useDayTime(time);
	const copy = appCopy(t);
	return (
		<section
			data-mode={mode}
			aria-label={t("view.howRuns.label", "How {{app}} runs on devices", {
				app,
			})}
			className={cx(
				"@container/howruns flex flex-wrap items-center gap-x-2.5 gap-y-1.5 rounded-lg border border-border bg-surface-sunken px-3 py-2 text-ui",
				className,
			)}
		>
			<VisibilityChip visibility={visibility} />
			<ModeChip mode={mode} app={app} />
			<p className="min-w-0 text-ui text-ink-2 @max-[900px]/howruns:order-3 @max-[900px]/howruns:basis-full">
				{copy.mode(mode, app).sentence}
			</p>
			<span className="flex-1 @max-[900px]/howruns:hidden" />
			{newest ? (
				<span className="inline-flex flex-wrap items-center gap-1 text-xs text-muted-foreground @max-[900px]/howruns:order-4">
					{t("view.howRuns.newest", "Newest")}
					<span className="font-mono text-foreground">{newest.label}</span>
					<IdRef
						id={newest.hash}
						copyLabel={t("view.drift.copyHash", "Copy app version hash")}
					/>
					{mode === "offline"
						? t("view.howRuns.changed", "· changed on this computer {{when}}", {
								when: dayTime(newest.at),
							})
						: t("view.howRuns.built", "· built {{when}}", {
								when: dayTime(newest.at),
							})}
				</span>
			) : null}
			{onExplain ? (
				<DvButton
					variant="link"
					size="xs"
					onClick={onExplain}
					className="whitespace-nowrap @max-[900px]/howruns:order-5"
				>
					{copy.explainButton()}
				</DvButton>
			) : null}
		</section>
	);
}

/**
 * APP §7.2: the online vs offline comparison with this app's column marked,
 * plus why the other way isn't available. Used in a sheet, in deploy step 2
 * and in the never-deployed state.
 */
export function ModeExplainer({
	app,
	mode,
	className,
}: Readonly<{ app: string; mode: AppModeValue; className?: string }>) {
	const { t } = useTranslation("devices");
	const copy = appCopy(t);
	const texts = {
		online: copy.mode("online", app),
		offline: copy.mode("offline", app),
	};
	return (
		<div
			data-mode-explainer={mode}
			className={cx("flex min-w-0 flex-col gap-3", className)}
		>
			<p className="max-w-[72ch] leading-[inherit] text-ink-2">
				{mode === "offline" ? (
					<Trans
						t={t}
						i18nKey="view.howRuns.leadOffline"
						defaults="<1/> is a local-only app, so every device gets an <2>offline copy</2>. How an app runs follows the app; it isn't chosen per deploy."
						components={{ 1: <span>{app}</span>, 2: MODE_WORD }}
					/>
				) : (
					<Trans
						t={t}
						i18nKey="view.howRuns.leadOnline"
						defaults="<1/> is an online app, so every device <2>runs it online</2>. How an app runs follows the app; it isn't chosen per deploy."
						components={{ 1: <span>{app}</span>, 2: MODE_WORD }}
					/>
				)}
			</p>
			<DvTable
				label={copy.explainerTitle()}
				cols={["22%", "39%", "39%"]}
				stackAt={560}
				wrapperClassName="rounded-lg border border-border"
				head={
					<tr>
						<Th>
							<span className="sr-only">
								{t("view.howRuns.topic", "Topic")}
							</span>
						</Th>
						{MODES.map((column) => {
							const Icon = MODE_ICON[column];
							const mine = column === mode;
							return (
								<Th
									key={column}
									data-mine={mine || undefined}
									className={cx(
										"align-top whitespace-normal normal-case tracking-normal",
										mine && "bg-good-bg/60",
									)}
								>
									<span className="inline-flex items-center gap-1.5 text-ui font-semibold text-foreground">
										<Icon aria-hidden className="size-4" />
										{texts[column].chip}
									</span>
									{mine ? (
										<StatusChip
											tone="good"
											icon={Check}
											className="mt-1 flex w-fit"
										>
											{copy.thisApp()}
										</StatusChip>
									) : (
										<span className="mt-1 block text-xs font-normal text-muted-foreground">
											{t("view.howRuns.notThisApp", "Not for this app")}
										</span>
									)}
								</Th>
							);
						})}
					</tr>
				}
			>
				{EXPLAINER_ROWS.map((row) => (
					<Tr key={row} className="hover:bg-transparent">
						<Td
							label={copy.explainerRowLabel(row)}
							kind="name"
							className="font-semibold"
						>
							{copy.explainerRowLabel(row)}
						</Td>
						{MODES.map((column) => (
							<Td
								key={column}
								label={texts[column].chip}
								data-mine={column === mode || undefined}
								className={
									column === mode ? "bg-good-bg/60" : "text-muted-foreground"
								}
							>
								{texts[column].rows[row]}
							</Td>
						))}
					</Tr>
				))}
			</DvTable>
			<div
				data-mode-why=""
				className="flex items-start gap-2 rounded-lg border border-border bg-surface-sunken px-3 py-2.5 text-ui text-ink-2"
			>
				<Info
					aria-hidden
					className="mt-px size-4 shrink-0 text-muted-foreground"
				/>
				<p className="max-w-[80ch] text-ui">{texts[mode].whyNot}</p>
			</div>
		</div>
	);
}
