"use client";

import { Trans, useTranslation } from "@flow-like/locales";
import { ChevronDown, ChevronRight, Rocket, SquareX } from "lucide-react";
import { useState } from "react";
import type {
	AppVersionPin,
	MatrixCell as CellModel,
	MatrixRow,
} from "../../../../lib/device-management/model/app-plan";
import {
	fleetFacts,
	isLastKnown,
	keysLocked,
} from "../../../../lib/device-management/model/device-view";
import type { ServiceView } from "../../../../lib/device-management/model/types";
import { answersRequests, cellLines } from "../app/kind-lines";
import { RunNowAction } from "../app/run-now";
import { appCopy } from "../copy/app-copy";
import {
	agentTooOldCopy,
	cantHereCopy,
	eligibilityCopy,
	eligibilityInput,
} from "../copy/eligibility-copy";
import { enumLabel } from "../copy/enum-labels";
import { scheduleRunNames } from "../copy/schedule-copy";
import { ModeChip } from "../primitives/app-chips";
import { type DevicesT, useAreaTime } from "../primitives/area-context";
import { Block } from "../primitives/block";
import { DvButton } from "../primitives/dv-button";
import { DvSheet } from "../primitives/dv-sheet";
import { DvTable, GroupRow, Td, Th, Tr } from "../primitives/dv-table";
import {
	EventCell as EventCellView,
	eventRunsLine,
} from "../primitives/event-cell";
import { FreshnessStamp } from "../primitives/freshness-stamp";
import { GateNotice } from "../primitives/gate-notice";
import { ModeExplainer } from "../primitives/how-runs";
import { IdRef } from "../primitives/id-ref";
import { MatrixCell, type MatrixCellProps } from "../primitives/matrix-cell";
import { initialsOf } from "../primitives/person-chip";
import { PresenceGlyph } from "../primitives/presence-glyph";
import { StateView } from "../primitives/state-view";
import { useDevicesRoute, useRouteLink } from "../routing/use-devices-route";
import { stampOf } from "../shell/attention-popover";
import { type AppViewRead, useAttentionState } from "../workspace";
import { type DeployTarget, useDeployTarget } from "./device-header";
import {
	DeviceDataState,
	desiredRun,
	observedRun,
	useLinkDelegate,
} from "./services-tab";
import { type DevicePage, gateView, servicesOfApp } from "./use-device-page";

const LINK =
	"underline decoration-border-strong underline-offset-2 hover:decoration-current";
const MATRIX_COLS = ["46%", "54%"] as const;

const triple = (version: readonly number[]) => version.join(".");

function pinText(t: DevicesT, pin: Omit<AppVersionPin, "eventId">): string {
	return t("devices:device.app.pin", "{{event}} · flow {{flow}}", {
		event: triple(pin.eventVersion),
		flow: triple(pin.boardVersion),
	});
}

/** The line under an event: its kind's own words when it can run here, else why it can't. */
function runsText(t: DevicesT, row: Omit<MatrixRow, "cells">): string | null {
	const { eligibility } = row;
	if (eligibility.eligible) return eventRunsLine(t, eligibility);
	return eligibility.code
		? eligibilityCopy(
				t,
				eligibilityInput(
					{ ...eligibility, code: eligibility.code },
					row.eventType,
				),
			).long
		: null;
}

/** APP §2.10 event cell: type tile, name, type, pins and the kind's own line. */
function EventCell({ row }: Readonly<{ row: Omit<MatrixRow, "cells"> }>) {
	const { t } = useTranslation("devices");
	const { followsLatest, eventVersion } = row.eligibility;
	const runs = runsText(t, row);
	return (
		<EventCellView
			eventType={row.eventType}
			hasPage={row.hasPage}
			name={row.name}
			eventId={row.eventId}
			followsLatest={followsLatest}
			{...(row.pin
				? {
						pin: {
							event: triple(row.pin.eventVersion),
							flow: triple(row.pin.boardVersion),
						},
					}
				: followsLatest && eventVersion
					? { pinNote: appCopy(t).pinLatest(triple(eventVersion)) }
					: {})}
			{...(row.newIn ? { newIn: row.newIn } : {})}
			{...(runs ? { runs } : {})}
		/>
	);
}

interface CellContext {
	t: DevicesT;
	time: ReturnType<typeof useAreaTime>;
	page: DevicePage;
	read: AppViewRead;
	deploy: DeployTarget;
	register: ReturnType<typeof useLinkDelegate>["register"];
	ports: Record<string, { port?: number; tls: boolean } | undefined>;
	activate(serviceId: string): void;
}

function servedCell(
	context: CellContext,
	row: MatrixRow,
	cell: CellModel,
	service: ServiceView,
): MatrixCellProps {
	const { t, time, page, read, register, ports } = context;
	const appRow = read.view?.services.find(
		(entry) =>
			entry.deviceId === page.deviceId && entry.serviceId === service.serviceId,
	);
	const observed = observedRun(service.observed);
	const note = appCopy(t).servedNote(cell.drift);
	const lines = cellLines(
		t,
		row,
		service,
		() =>
			scheduleRunNames(t, {
				deviceId: page.deviceId,
				serviceId: service.serviceId,
				device: page.name,
				eventId: row.eventId,
				...(row.where ? { where: row.where } : {}),
				...(page.services ? { siblings: page.services } : {}),
			}),
		time,
	);
	const stagedHash =
		appRow?.lastChange?.kind === "update" ? appRow.lastChange.hash : undefined;
	const staged = stagedHash
		? read.view?.versions.find((version) => version.hash === stagedHash)?.label
		: undefined;
	const port = ports[service.serviceId];
	return {
		state: cell.state === "staged" ? "staged" : "served",
		serviceId: service.serviceId,
		href: register({
			screen: "service",
			deviceId: page.deviceId,
			serviceId: service.serviceId,
			tab: "status",
		}),
		desired: desiredRun(service.desired),
		observed,
		conv: service.conv,
		actual: isLastKnown(service.freshness)
			? t("devices:device.app.actualLastKnown", "{{state}} · last known", {
					state: enumLabel(t, "observed", observed),
				})
			: enumLabel(t, "observed", observed),
		pin: cell.pin
			? pinText(t, cell.pin)
			: t("devices:device.app.pinUnknown", "version unknown"),
		...(cell.behind && row.pin
			? {
					target: triple(row.pin.eventVersion),
					targetTitle: t(
						"devices:device.app.targetTitle",
						"The newest version pins event {{event}} · flow {{flow}}",
						{
							event: triple(row.pin.eventVersion),
							flow: triple(row.pin.boardVersion),
						},
					),
				}
			: {}),
		...(note ? { note } : {}),
		...(row.eligibility.followsLatest ? { tag: true } : {}),
		...(lines.length ? { lines } : {}),
		...(port?.port === undefined || !answersRequests(row.eligibility.kind)
			? {}
			: { port: `:${port.port}`, tls: port.tls }),
		...(cell.state === "staged"
			? {
					stagedVersion:
						staged ?? t("devices:device.app.updateStaged", "Update"),
					onActivate: () => context.activate(service.serviceId),
				}
			: {}),
		...(cell.serviceIds.length > 1 ? { also: cell.serviceIds.slice(1) } : {}),
	};
}

function cellProps(
	context: CellContext,
	row: MatrixRow,
	cell: CellModel | undefined,
): MatrixCellProps {
	const { t, time, page, deploy, register } = context;
	const route = register({
		screen: "deploy",
		mode: "new",
		eventId: row.eventId,
		deviceIds: [page.deviceId],
	});
	if (!cell)
		return {
			state: "not_served",
			deployHref: route,
			gate: deploy.gate?.gate ?? null,
		};
	if (cell.state === "served" || cell.state === "staged") {
		const service = page.services?.find(
			(entry) => entry.serviceId === cell.serviceIds[0],
		);
		return service
			? servedCell(context, row, cell, service)
			: { state: "unknown", why: "notloaded" };
	}
	if (cell.state === "not_served")
		return {
			state: "not_served",
			deployHref: route,
			gate:
				deploy.gate?.gate ??
				(cell.gate ? (gateView(t, time, cell.gate)?.gate ?? null) : null),
		};
	if (cell.state === "cant_here")
		return {
			state: "cant_here",
			reason:
				cell.why === "agent" ? (
					<>
						{cantHereCopy(t, cell, page.name)}{" "}
						<a
							href={register({
								screen: "device",
								deviceId: page.deviceId,
								tab: "settings",
							})}
							className={LINK}
						>
							{agentTooOldCopy(t, page.name, cell.feature).fix}
						</a>
					</>
				) : (
					cantHereCopy(t, cell, page.name)
				),
		};
	if (cell.state === "no_access") return { state: "no_access" };
	const kind = cell.unknown?.kind;
	const why =
		kind === "locked" ||
		kind === "nokeys" ||
		kind === "offline" ||
		kind === "error" ||
		kind === "snapshot"
			? kind
			: "notloaded";
	const since =
		cell.unknown && "since" in cell.unknown ? cell.unknown.since : undefined;
	return { state: "unknown", why, ...(since === undefined ? {} : { since }) };
}

/** Run now… under a person-started event that a service of this device runs. */
function CellRunNow({
	page,
	row,
}: Readonly<{ page: DevicePage; row: MatrixRow }>) {
	const cell = row.cells[page.deviceId];
	if (row.eligibility.kind !== "on_demand" || !cell) return null;
	if (cell.state !== "served" && cell.state !== "staged") return null;
	const service = page.services?.find(
		(entry) => entry.serviceId === cell.serviceIds[0],
	);
	return service ? (
		<div className="mt-1.5">
			<RunNowAction
				deviceId={page.deviceId}
				view={service}
				eventId={row.eventId}
			/>
		</div>
	) : null;
}

function EventMatrix({
	page,
	read,
	deploy,
}: Readonly<{ page: DevicePage; read: AppViewRead; deploy: DeployTarget }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const { input } = useAttentionState();
	const delegate = useLinkDelegate();
	const { navigate } = useDevicesRoute();
	const [open, setOpen] = useState(false);
	const view = read.view;
	if (!view) return null;
	const placements =
		fleetFacts(input).byId.get(page.deviceId)?.liveInput?.placements ?? {};
	const ports = Object.fromEntries(
		Object.entries(placements).map(([id, facts]) => [
			id,
			{ port: facts.port, tls: !!facts.tlsCertificateId },
		]),
	);
	const context: CellContext = {
		t,
		time,
		page,
		read,
		deploy,
		register: delegate.register,
		ports,
		activate: (serviceId) =>
			navigate({
				screen: "service",
				deviceId: page.deviceId,
				serviceId,
				tab: "status",
			}),
	};
	const { rows, ineligible } = view.events;
	return (
		// biome-ignore lint/a11y/useKeyWithClickEvents: delegates clicks on the links inside the table; the links are keyboard reachable
		<div onClick={delegate.onClick}>
			<DvTable
				cols={MATRIX_COLS}
				stackAt={560}
				label={t("device.app.matrixLabel", "How {{app}} runs on {{device}}", {
					app: view.app.name,
					device: page.name,
				})}
				head={
					<tr>
						<Th>{t("device.app.event", "Event")}</Th>
						<Th>
							<span className="inline-flex items-center gap-1.5 font-mono normal-case tracking-normal text-foreground">
								<PresenceGlyph
									kind={page.view.presence.kind}
									label={enumLabel(t, "presence", page.view.presence.kind)}
								/>
								{page.name}
							</span>
						</Th>
					</tr>
				}
			>
				{rows.map((row) => (
					<Tr key={row.eventId} data-event={row.eventId}>
						<Td label={t("device.app.event", "Event")} kind="name">
							<EventCell row={row} />
						</Td>
						<Td label={page.name}>
							<MatrixCell
								{...cellProps(context, row, row.cells[page.deviceId])}
							/>
							<CellRunNow page={page} row={row} />
						</Td>
					</Tr>
				))}
				{ineligible.length ? (
					<GroupRow colSpan={MATRIX_COLS.length}>
						<button
							type="button"
							aria-expanded={open}
							onClick={() => setOpen((current) => !current)}
							className="inline-flex cursor-pointer items-center gap-1.5 font-semibold text-ink-2 hover:text-foreground"
						>
							{open ? (
								<ChevronDown aria-hidden className="size-3.5" />
							) : (
								<ChevronRight aria-hidden className="size-3.5" />
							)}
							{t("device.app.cantRun", {
								count: ineligible.length,
								defaultValue_one:
									"Can't run on devices · {{count, number}} event",
								defaultValue_other:
									"Can't run on devices · {{count, number}} events",
							})}
						</button>
					</GroupRow>
				) : null}
				{open
					? ineligible.map((row) => (
							<Tr key={row.eventId} data-event={row.eventId} dim>
								<Td label={t("device.app.event", "Event")} kind="name">
									<EventCell row={row} />
								</Td>
								<Td label={page.name}>
									{/* A service that was deployed before keeps running it: the cell stays. */}
									{row.cells[page.deviceId] ? (
										<MatrixCell
											{...cellProps(context, row, row.cells[page.deviceId])}
										/>
									) : (
										<span className="text-xs text-muted-foreground">
											{t("device.app.cantRunCell", "Can't run on a device")}
										</span>
									)}
								</Td>
							</Tr>
						))
					: null}
			</DvTable>
		</div>
	);
}

type AppState = "unreadable" | "none" | "running";

function ModeLine({
	page,
	read,
	state,
}: Readonly<{ page: DevicePage; read: AppViewRead; state: AppState }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const [explain, setExplain] = useState(false);
	const view = read.view;
	if (!view || !page.app) return null;
	const app = page.app.name;
	const device = page.name;
	const offline = view.app.mode === "offline";
	const components = {
		1: <span>{app}</span>,
		2: <span className="font-mono">{device}</span>,
		3: <b className="font-semibold text-foreground" />,
	};
	const seeded = view.services.find(
		(row) => row.deviceId === page.deviceId && row.data.since !== undefined,
	)?.data.since;
	const sentence =
		state === "unreadable" ? (
			offline ? (
				<Trans
					t={t}
					i18nKey="device.app.mode.unreadableOffline"
					defaults="<1/> is a local-only app, so any service of it on <2/> runs an <3>offline copy</3> made on this computer, with its data only on the device."
					components={components}
				/>
			) : (
				<Trans
					t={t}
					i18nKey="device.app.mode.unreadableOnline"
					defaults="<1/> is an online app, so any service of it on <2/> <3>runs online</3> at the version deployed from the hub, with its data in the cloud."
					components={components}
				/>
			)
		) : state === "none" ? (
			offline ? (
				<Trans
					t={t}
					i18nKey="device.app.mode.noneOffline"
					defaults="<1/> is a local-only app. Deployed here, <2/> gets an <3>offline copy</3> made on this computer and keeps its data only on the device; nothing syncs back."
					components={components}
				/>
			) : (
				<Trans
					t={t}
					i18nKey="device.app.mode.noneOnline"
					defaults="<1/> is an online app. Deployed here, <2/> <3>runs it online</3> at the version you deploy from the hub. Data stays in the cloud, so <2/> needs internet."
					components={components}
				/>
			)
		) : offline ? (
			seeded === undefined ? (
				<Trans
					t={t}
					i18nKey="device.app.mode.runningOffline"
					defaults="<1/> is a local-only app, so <2/> runs an <3>offline copy</3> made on this computer. Its data lives only on <2/>; nothing syncs back."
					components={components}
				/>
			) : (
				<Trans
					t={t}
					i18nKey="device.app.mode.runningOfflineSince"
					defaults="<1/> is a local-only app, so <2/> runs an <3>offline copy</3> made on this computer. Its data lives only on <2/>, since {{date}}; nothing syncs back."
					values={{ date: time.at(seeded) }}
					components={components}
				/>
			)
		) : (
			<Trans
				t={t}
				i18nKey="device.app.mode.runningOnline"
				defaults="<1/> is an online app, so <2/> <3>runs it online</3> at the version you deployed from the hub. Data stays in the cloud, so <2/> needs internet. Services get cloud access 10 minutes at a time."
				components={components}
			/>
		);
	const copy = appCopy(t);
	return (
		<div className="flex flex-wrap items-start gap-x-3 gap-y-2 border-b border-hairline px-4 py-3">
			<ModeChip mode={view.app.mode} app={app} />
			<p className="max-w-[78ch] min-w-0 flex-1 basis-[320px] text-ui text-ink-2">
				{sentence}
			</p>
			<DvButton variant="link" onClick={() => setExplain(true)}>
				{copy.explainButton()}
			</DvButton>
			<DvSheet
				open={explain}
				onOpenChange={setExplain}
				title={copy.explainerTitle()}
				sub={copy.explainerSub(view.app.mode, app)}
				wide
				foot={
					<DvButton onClick={() => setExplain(false)}>
						{t("device.app.close", "Close")}
					</DvButton>
				}
			>
				<ModeExplainer app={app} mode={view.app.mode} />
			</DvSheet>
		</div>
	);
}

function HowToLine({
	page,
	read,
	deploy,
	matrix,
}: Readonly<{
	page: DevicePage;
	read: AppViewRead;
	deploy: DeployTarget;
	matrix: boolean;
}>) {
	const { t } = useTranslation("devices");
	const link = useRouteLink();
	const view = read.view;
	if (!view || !page.app || page.revoked) return null;
	const app = page.app.name;
	const device = page.name;
	const { keys, presence } = page.view;
	const name = { 1: <span className="font-mono">{device}</span> };
	const prefix =
		presence.kind === "never" ? (
			<Trans
				t={t}
				i18nKey="device.app.how.afterCheckIn"
				defaults="Once <1/> checks in, "
				components={name}
			/>
		) : keys.state === "none" || keys.state === "stale" ? (
			<Trans
				t={t}
				i18nKey="device.app.how.afterKeys"
				defaults="Once this computer has keys for <1/>, "
				components={name}
			/>
		) : keysLocked(keys) ? (
			<Trans
				t={t}
				i18nKey="device.app.how.afterUnlock"
				defaults="After you unlock <1/>, "
				components={name}
			/>
		) : null;
	const bold = {
		1: <b className="font-semibold text-foreground">{deploy.label}</b>,
		2: <b className="font-semibold text-foreground" />,
	};
	return (
		<div className="flex items-start gap-2.5 border-b border-hairline px-4 py-3 text-ui text-ink-2">
			<Rocket
				aria-hidden
				className="mt-0.5 size-4 shrink-0 text-muted-foreground"
			/>
			<p className="max-w-[90ch] min-w-0 text-ui">
				{prefix}
				{matrix ? (
					<Trans
						t={t}
						i18nKey="device.app.how.withMatrix"
						defaults="<1/> puts the events you pick into one service; <2>Deploy here</2> on an event starts with only that event."
						components={bold}
					/>
				) : (
					<Trans
						t={t}
						i18nKey="device.app.how.plain"
						defaults="<1/> puts the events you pick into one service."
						components={bold}
					/>
				)}{" "}
				{view.app.mode === "offline"
					? t(
							"device.app.how.needsOffline",
							"Each deploy sends a copy from this computer and works only in the desktop app.",
						)
					: t(
							"device.app.how.needsOnline",
							"A new service needs cloud access, approved by an Admin or Owner of {{app}}.",
							{ app },
						)}{" "}
				<Trans
					t={t}
					i18nKey="device.app.how.several"
					defaults="To deploy to several devices at once, use Deploy to devices… on <1/>."
					components={{
						1: (
							<a
								{...link({ screen: "app-devices", by: "device" })}
								className={LINK}
							>
								{t("device.app.how.appDevices", "{{app}} › Devices", { app })}
							</a>
						),
					}}
				/>
			</p>
		</div>
	);
}

function AppFoot({
	page,
	read,
}: Readonly<{ page: DevicePage; read: AppViewRead }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const link = useRouteLink();
	const view = read.view;
	if (!view || !page.app) return null;
	const newest = view.howRuns.newest;
	return (
		<>
			{newest ? (
				<span className="inline-flex flex-wrap items-center gap-x-1.5 gap-y-1">
					{t("device.app.newest", "Newest version")}
					<span className="font-mono">{newest.label ?? newest.short}</span>
					<IdRef
						id={newest.hash}
						copyLabel={t("device.app.copyHash", "Copy app version hash")}
					/>
					{newest.builtAt === null
						? null
						: view.app.localOnly
							? t(
									"device.app.changedHere",
									"· changed on this computer {{when}}",
									{ when: time.at(newest.builtAt) },
								)
							: t("device.app.built", "· built {{when}}", {
									when: time.at(newest.builtAt),
								})}
				</span>
			) : null}
			<span>{appCopy(t).publishNote(view.app.mode)}</span>
			<a
				{...link({
					screen: "app-devices",
					by: "device",
					focusDeviceId: page.deviceId,
				})}
				className={LINK}
			>
				{t("device.app.whereElse", "Where else {{app}} runs", {
					app: page.app.name,
				})}
			</a>
		</>
	);
}

function AppBody({
	page,
	read,
	deploy,
}: Readonly<{ page: DevicePage; read: AppViewRead; deploy: DeployTarget }>) {
	const { t } = useTranslation("devices");
	const view = read.view;
	const app = page.app?.name ?? "";
	if (page.revoked)
		return (
			<div className="px-4 py-3">
				<StateView
					kind="notloaded"
					icon={SquareX}
					title={t("device.state.revokedTitle", "Not available")}
					text={t(
						"device.app.revoked",
						"Revoked devices aren't read, so nothing runs here any more.",
					)}
				/>
			</div>
		);
	if (page.unavailable)
		return (
			<div className="px-4 py-3">
				<DeviceDataState page={page} what="app" />
			</div>
		);
	if (!view)
		return (
			<div className="px-4 py-3">
				{read.error ? (
					<StateView
						kind="error"
						title={t("device.app.loadError", "Couldn't load {{app}}'s events", {
							app,
						})}
						text={t(
							"device.app.loadErrorText",
							"The services below still show what the device reports.",
						)}
					/>
				) : (
					<StateView kind="loading" rows={3} />
				)}
			</div>
		);
	if (!view.app.canReadFlows)
		return (
			<div className="px-4 py-3">
				<GateNotice
					kind="role"
					title={t("device.app.noFlows", "Events on this device")}
					text={t(
						"device.app.noFlowsText",
						"Seeing which of {{app}}'s events run here needs permission to read its flows. Ask an Admin or Owner of {{app}}.",
						{ app },
					)}
				/>
			</div>
		);
	if (!view.events.rows.length && !view.events.ineligible.length)
		return (
			<div className="px-4 py-3">
				<StateView
					kind="empty"
					title={t("device.app.noEvents", "{{app}} has no events", { app })}
					text={t(
						"device.app.noEventsText",
						"Add an event in Events before you deploy it.",
					)}
				/>
			</div>
		);
	return <EventMatrix page={page} read={read} deploy={deploy} />;
}

/** APP §1.10: how this app runs on this one device, event by event. */
export function DeviceAppBlock({
	page,
	app,
}: Readonly<{ page: DevicePage; app: AppViewRead | null }>) {
	const { t } = useTranslation("devices");
	const deploy = useDeployTarget(page, app);
	if (!page.app || !app) return null;
	const mine = servicesOfApp(page.services ?? [], page.app.id);
	const readable = !page.revoked && !page.unavailable;
	const state: AppState = !readable
		? "unreadable"
		: mine.length
			? "running"
			: "none";
	const first = page.services?.[0];
	const matrix =
		readable && !!app.view?.events.rows.length && app.view.app.canReadFlows;
	return (
		<Block
			id="dv-app"
			title={
				<span className="inline-flex min-w-0 items-center gap-2">
					<span
						aria-hidden
						className="inline-flex size-6 shrink-0 items-center justify-center rounded-md border border-border bg-surface-sunken text-[11px]/none font-semibold tracking-[0.02em] text-ink-2"
					>
						{initialsOf(page.app.name)}
					</span>
					<span className="min-w-0">
						<Trans
							t={t}
							i18nKey="device.app.title"
							defaults="<1/> on <2/>"
							components={{
								1: <span>{page.app.name}</span>,
								2: <span className="font-mono">{page.name}</span>,
							}}
						/>
					</span>
				</span>
			}
			summary={
				readable
					? t("device.app.serviceCount", {
							count: mine.length,
							defaultValue_one: "{{count, number}} service",
							defaultValue_other: "{{count, number}} services",
						})
					: undefined
			}
			stamp={first ? <FreshnessStamp {...stampOf(first.freshness)} /> : null}
			flush
			foot={<AppFoot page={page} read={app} />}
		>
			<ModeLine page={page} read={app} state={state} />
			<HowToLine page={page} read={app} deploy={deploy} matrix={matrix} />
			<AppBody page={page} read={app} deploy={deploy} />
		</Block>
	);
}
