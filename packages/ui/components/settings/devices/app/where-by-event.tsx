"use client";

import { useTranslation } from "@flow-like/locales";
import { ChevronDown, ChevronRight, Info } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import {
	type AppServiceRow,
	MATRIX_MAX_COLUMNS,
	type MatrixRow,
	type MatrixCell as ModelCell,
} from "../../../../lib/device-management/model/app-plan";
import { appCopy } from "../copy/app-copy";
import {
	agentTooOldCopy,
	cantHereCopy,
	eligibilityCopy,
	eligibilityFixLabel,
	eligibilityInput,
} from "../copy/eligibility-copy";
import { enumLabel } from "../copy/enum-labels";
import { scheduleRunNames } from "../copy/schedule-copy";
import { AreaEventsDevices } from "../events/events-devices";
import { RunsOnCell } from "../events/runs-on-cell";
import { useAreaTime } from "../primitives/area-context";
import { CellSub, DvTable, Td, Th, Tr } from "../primitives/dv-table";
import {
	EventCell as EventCellView,
	eventRunsLine,
} from "../primitives/event-cell";
import { MatrixCell, type MatrixUnknownCell } from "../primitives/matrix-cell";
import { PairedPins } from "../primitives/paired-pins";
import { PresenceGlyph } from "../primitives/presence-glyph";
import { KeyChip } from "../primitives/status-chip";
import { cx } from "../primitives/tone";
import { appEventsHref } from "../routing/devices-href";
import { useDevicesRoute, useRouteLink } from "../routing/use-devices-route";
import { keyChipOf } from "../shell/keys-popover";
import {
	APP_LINKS,
	HostLink,
	LINK,
	UnknownAction,
	useAppPage,
	useDeviceNames,
	useGateText,
	useLinkCapture,
} from "./app-shared";
import {
	blocksDeploy,
	pinText,
	stagedVersionOf,
	versionName,
} from "./app-view-local";
import { answersRequests, cellLines } from "./kind-lines";
import { RunNowAction } from "./run-now";
import { useStagedActions } from "./use-service-actions";
import { desiredRun, observedRun, useEndpoint } from "./where-by-device";

type EventRow = Omit<MatrixRow, "cells">;

/** APP §2.10 event cell: type tile, name, type, pins and the kind's own line (an Endpoint's path, when a schedule runs). */
export function EventCell({ row }: Readonly<{ row: EventRow }>) {
	const { t } = useTranslation("devices");
	const { followsLatest, eventVersion, eligible } = row.eligibility;
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
							event: pinText(row.pin.eventVersion),
							flow: pinText(row.pin.boardVersion),
						},
					}
				: followsLatest && eventVersion
					? { pinNote: appCopy(t).pinLatest(pinText(eventVersion)) }
					: {})}
			{...(row.newIn ? { newIn: row.newIn } : {})}
			{...(eligible ? { runs: eventRunsLine(t, row.eligibility) } : {})}
		/>
	);
}

function ServedCell({
	row,
	cell,
	service,
}: Readonly<{ row: MatrixRow; cell: ModelCell; service: AppServiceRow }>) {
	const { t } = useTranslation("devices");
	const { view } = useAppPage();
	const { href } = useDevicesRoute();
	const deviceName = useDeviceNames();
	const time = useAreaTime();
	const copy = appCopy(t);
	const note = copy.servedNote(cell.drift);
	const lines = cellLines(
		t,
		row,
		service.view,
		() =>
			scheduleRunNames(t, {
				deviceId: cell.deviceId,
				serviceId: service.serviceId,
				device: deviceName(cell.deviceId),
				eventId: row.eventId,
				...(row.where ? { where: row.where } : {}),
				deviceName,
				siblings: view.services
					.filter((entry) => entry.deviceId === cell.deviceId)
					.map((entry) => entry.view),
			}),
		time,
	);
	const endpoint = useEndpoint(cell.deviceId, service.serviceId);
	const staged = stagedVersionOf(service, view.versions);
	const actions = useStagedActions(service, deviceName(cell.deviceId), staged);
	const observed = observedRun(service.view.observed);
	const label = enumLabel(t, "observed", observed);
	const stagedPin = staged?.pins.find((pin) => pin.eventId === row.eventId);
	const newest = view.versions[0];
	const target =
		cell.behind && row.pin
			? cell.pin &&
				pinText(cell.pin.eventVersion) === pinText(row.pin.eventVersion)
				? t("app.event.flowTarget", "flow {{flow}}", {
						flow: pinText(row.pin.boardVersion),
					})
				: pinText(row.pin.eventVersion)
			: undefined;
	const matrixCell = (
		<MatrixCell
			state={cell.state === "staged" ? "staged" : "served"}
			serviceId={service.serviceId}
			href={href(APP_LINKS.service(cell.deviceId, service.serviceId))}
			desired={desiredRun(service.view.desired)}
			observed={observed}
			conv={service.view.conv}
			actual={
				service.lastKnown
					? t("app.event.lastKnown", "{{label}} · last known", { label })
					: label
			}
			pin={
				cell.pin
					? t("app.event.cellPins", "{{event}} · flow {{flow}}", {
							event: pinText(cell.pin.eventVersion),
							flow: pinText(cell.pin.boardVersion),
						})
					: t("app.event.pinUnknown", "version unknown")
			}
			{...(target ? { target } : {})}
			{...(note ? { note } : {})}
			{...(row.eligibility.followsLatest ? { tag: true } : {})}
			{...(lines.length ? { lines } : {})}
			{...(target && row.pin && newest
				? {
						targetTitle: copy.pinTitle({
							event: row.name,
							version: pinText(row.pin.eventVersion),
							release: versionName(newest),
						}),
					}
				: {})}
			{...(endpoint && answersRequests(row.eligibility.kind)
				? { port: `:${endpoint.address.split(":").pop()}`, tls: endpoint.tls }
				: {})}
			{...(cell.state === "staged"
				? {
						stagedVersion: stagedPin
							? pinText(stagedPin.eventVersion)
							: t("app.event.stagedUpdate", "Update"),
						onActivate: actions.activate,
						activateGate: actions.activateGate,
					}
				: {})}
			{...(cell.serviceIds.length > 1
				? { also: cell.serviceIds.slice(1) }
				: {})}
		/>
	);
	if (row.eligibility.kind !== "on_demand") return matrixCell;
	return (
		<span className="flex min-w-0 flex-col items-start gap-1.5">
			{matrixCell}
			<RunNowAction
				deviceId={cell.deviceId}
				view={service.view}
				eventId={row.eventId}
			/>
		</span>
	);
}

/** Why this device can't take the event; an agent that is too old comes with the way to update it. */
function CantHereReason({ cell }: Readonly<{ cell: ModelCell }>) {
	const { t } = useTranslation("devices");
	const link = useRouteLink();
	const device = useDeviceNames()(cell.deviceId);
	const reason = cantHereCopy(t, cell, device);
	if (cell.why !== "agent") return <>{reason}</>;
	return (
		<>
			{reason}{" "}
			<a
				{...link({
					screen: "device",
					deviceId: cell.deviceId,
					tab: "settings",
				})}
				className={cx(LINK, "underline")}
			>
				{agentTooOldCopy(t, device, cell.feature).fix}
			</a>
		</>
	);
}

function DeviceCell({
	row,
	cell,
}: Readonly<{ row: MatrixRow; cell: ModelCell }>) {
	const { view } = useAppPage();
	const { navigate } = useDevicesRoute();
	const gateText = useGateText();
	if (cell.state === "served" || cell.state === "staged") {
		const service = view.services.find(
			(entry) =>
				entry.deviceId === cell.deviceId &&
				entry.serviceId === cell.serviceIds[0],
		);
		if (service) return <ServedCell row={row} cell={cell} service={service} />;
	}
	if (cell.state === "cant_here")
		return (
			<MatrixCell state="cant_here" reason={<CantHereReason cell={cell} />} />
		);
	if (cell.state === "no_access") return <MatrixCell state="no_access" />;
	if (cell.state === "unknown" && cell.unknown) {
		const kind = cell.unknown.kind;
		const why: MatrixUnknownCell["why"] =
			kind === "never" || kind === "noaccess" ? "notloaded" : kind;
		const since = "since" in cell.unknown ? cell.unknown.since : undefined;
		return (
			<MatrixCell
				state="unknown"
				why={why}
				{...(since === undefined ? {} : { since })}
				action={
					kind === "snapshot" ? undefined : (
						<UnknownAction
							deviceId={cell.deviceId}
							unknown={cell.unknown}
							size="xs"
						/>
					)
				}
			/>
		);
	}
	return (
		<MatrixCell
			state="not_served"
			onDeploy={() =>
				navigate(
					APP_LINKS.deploy({
						eventId: row.eventId,
						deviceIds: [cell.deviceId],
					}),
				)
			}
			gate={blocksDeploy(cell.gate) ? gateText(cell.gate) : null}
		/>
	);
}

function DeviceHead({
	deviceId,
	focused,
}: Readonly<{ deviceId: string; focused: boolean }>) {
	const { view, data } = useAppPage();
	const link = useRouteLink();
	const deviceName = useDeviceNames();
	const device = data.devices.get(deviceId);
	const group = view.groups.find((entry) => entry.deviceId === deviceId);
	const name = deviceName(deviceId);
	const chip = device ? keyChipOf(device.keys, device.live) : null;
	const ref = useRef<HTMLSpanElement>(null);
	useEffect(() => {
		if (focused) ref.current?.scrollIntoView?.({ block: "nearest" });
	}, [focused]);
	return (
		<span ref={ref} className="flex min-w-0 flex-col gap-0.5 normal-case">
			<span className="flex min-w-0 items-center gap-1.5">
				{device ? <PresenceGlyph kind={device.presence.kind} /> : null}
				<a
					{...link(APP_LINKS.device(deviceId))}
					title={name}
					className={cx(
						LINK,
						"min-w-0 truncate font-mono text-xs font-semibold tracking-normal",
					)}
				>
					{name}
				</a>
				{chip ? (
					<KeyChip
						state={chip.state}
						transport={chip.transport}
						className="h-4.5 border-0 bg-transparent px-0 [&>span]:sr-only"
					/>
				) : null}
			</span>
			{group?.services.length ? (
				<span className="truncate font-mono text-[11px] font-normal tracking-normal text-muted-foreground">
					{group.services.map((service) => service.serviceId).join(", ")}
				</span>
			) : null}
		</span>
	);
}

/** An event that can't be deployed any more keeps running where it was deployed: each such service, with its link. */
function StillServed({ row }: Readonly<{ row: MatrixRow }>) {
	const { t } = useTranslation("devices");
	const link = useRouteLink();
	const deviceName = useDeviceNames();
	const served = Object.values(row.cells).filter(
		(cell) => cell.state === "served" || cell.state === "staged",
	);
	return served.map((cell) => {
		const [serviceId = ""] = cell.serviceIds;
		return (
			<CellSub
				key={cell.deviceId}
				data-still-served={cell.deviceId}
				className="text-warning"
			>
				{t(
					"events.pop.stillServed",
					"{{device}} still runs it until you stop or update {{service}}.",
					{ device: deviceName(cell.deviceId), service: serviceId },
				)}{" "}
				<a
					{...link(APP_LINKS.service(cell.deviceId, serviceId))}
					className={cx(LINK, "underline")}
				>
					{t("events.pop.openService", "Open {{service}}", {
						service: serviceId,
					})}
				</a>
			</CellSub>
		);
	});
}

/** "Can't run on devices · N": collapsed; each event with its plain reason and fix link (APP §2.10). */
export function CantRunGroup({ span }: Readonly<{ span: number }>) {
	const { t } = useTranslation("devices");
	const { view, data } = useAppPage();
	const [open, setOpen] = useState(false);
	const rows = view.events.ineligible;
	if (!rows.length) return null;
	const Chevron = open ? ChevronDown : ChevronRight;
	return (
		<>
			<tr data-cant-run="" className="hover:bg-transparent">
				<td
					colSpan={span}
					className="border-t border-hairline bg-surface-sunken px-4 py-1.5"
				>
					<button
						type="button"
						aria-expanded={open}
						onClick={() => setOpen((value) => !value)}
						className="inline-flex items-center gap-1.5 text-xs font-semibold text-ink-2 hover:text-foreground focus-visible:outline-2 focus-visible:outline-ring"
					>
						<Chevron aria-hidden className="size-3.5" />
						{t(
							"app.event.cantRun",
							"Can't run on devices · {{count, number}}",
							{
								count: rows.length,
							},
						)}
					</button>
				</td>
			</tr>
			{open
				? rows.map((row) => {
						const reason = row.eligibility.code
							? eligibilityCopy(
									t,
									eligibilityInput(
										{ ...row.eligibility, code: row.eligibility.code },
										row.eventType,
									),
								)
							: null;
						return (
							<Tr key={row.eventId} data-event={row.eventId} dim>
								<Td label={t("app.event.colEvent", "Event")} kind="name">
									<EventCell row={row} />
									{reason ? (
										<CellSub className="pl-8.5 text-ink-2">
											{reason.long}{" "}
											{reason.fix ? (
												<HostLink
													href={appEventsHref(data.appId, row.eventId)}
													className={cx(LINK, "underline")}
												>
													{eligibilityFixLabel(t, reason.fix)}
												</HostLink>
											) : null}
										</CellSub>
									) : null}
								</Td>
								{span > 1 ? (
									<Td
										label={t("app.event.colDevices", "Devices")}
										colSpan={span - 1}
										className="text-muted-foreground"
									>
										{t("app.event.notOffered", "Not offered when you deploy")}
										<StillServed row={row} />
									</Td>
								) : null}
							</Tr>
						);
					})
				: null}
		</>
	);
}

/** The foot of the By event view: what unknown means and the pins legend. */
export function EventFoot() {
	const { t } = useTranslation("devices");
	return (
		<div className="flex w-full flex-col gap-1.5">
			<p className="flex items-start gap-1.5 text-xs">
				<Info aria-hidden className="mt-0.5 size-3.5 shrink-0" />
				<span>
					{t(
						"app.event.foot",
						"An event can be served by more than one service on the same device. Unknown never means not deployed.",
					)}
				</span>
			</p>
			<p className="flex items-start gap-1.5 text-xs">
				<PairedPins
					desired="running"
					observed="running"
					conv="converged"
					title
					className="mt-0.5"
				/>
				<span>
					{t(
						"app.event.footPins",
						"Hollow pin: what you asked for. Filled pin: what the device reports.",
					)}
				</span>
			</p>
		</div>
	);
}

/** The event column alone, for the never-deployed layout (APP §2.18). */
export function EventList() {
	const { t } = useTranslation("devices");
	const { view } = useAppPage();
	return (
		<DvTable
			label={t(
				"app.event.listLabel",
				"Events of {{app}} that can run on a device",
				{
					app: view.app.name,
				},
			)}
			cols={["100%"]}
			stackAt={false}
		>
			{view.events.rows.map((row) => (
				<Tr key={row.eventId} data-event={row.eventId}>
					<Td label={t("app.event.colEvent", "Event")} kind="name">
						<EventCell row={row} />
					</Td>
				</Tr>
			))}
			<CantRunGroup span={1} />
		</DvTable>
	);
}

/** APP §2.10: events × devices; with more than four device columns one "Runs on" cell per event. */
export function WhereByEvent() {
	const { t } = useTranslation("devices");
	const { view, data, route } = useAppPage();
	const deviceName = useDeviceNames();
	const serviceRoutes = useMemo(
		() =>
			view.services.map((service) =>
				APP_LINKS.service(service.deviceId, service.serviceId),
			),
		[view.services],
	);
	const onClickCapture = useLinkCapture(serviceRoutes);
	const { cols, rows } = view.events;
	const listMode = cols.length > MATRIX_MAX_COLUMNS;
	const widths = useMemo(() => {
		if (listMode || !cols.length) return ["40%", "60%"];
		const each = `${Math.floor(68 / cols.length)}%`;
		return ["32%", ...cols.map(() => each)];
	}, [listMode, cols]);
	const target = useRef<HTMLTableRowElement>(null);
	// biome-ignore lint/correctness/useExhaustiveDependencies: scrolls once per deep link
	useEffect(() => {
		target.current?.scrollIntoView?.({ block: "nearest" });
	}, [route.eventId]);
	const span = widths.length;
	const table = (
		<DvTable
			label={t("app.event.tableLabel", "Events of {{app}} by device", {
				app: view.app.name,
			})}
			cols={widths}
			head={
				<tr>
					<Th>{t("app.event.colEvent", "Event")}</Th>
					{listMode ? (
						<Th>{t("app.event.colRunsOn", "Runs on")}</Th>
					) : (
						cols.map((deviceId) => (
							<Th
								key={deviceId}
								data-device={deviceId}
								data-focus={route.focusDeviceId === deviceId || undefined}
								className={cx(
									"align-top whitespace-normal",
									route.focusDeviceId === deviceId && "bg-row-selected",
								)}
							>
								<DeviceHead
									deviceId={deviceId}
									focused={route.focusDeviceId === deviceId}
								/>
							</Th>
						))
					)}
				</tr>
			}
		>
			{rows.map((row) => {
				const targeted = route.eventId === row.eventId;
				return (
					<Tr
						key={row.eventId}
						ref={targeted ? target : undefined}
						data-event={row.eventId}
						data-target={targeted || undefined}
						className={cx(
							"hover:bg-transparent",
							targeted &&
								"[&>td:first-child]:border-l-2 [&>td:first-child]:border-l-ring",
						)}
					>
						<Td label={t("app.event.colEvent", "Event")} kind="name">
							<EventCell row={row} />
						</Td>
						{listMode ? (
							<Td label={t("app.event.colRunsOn", "Runs on")}>
								<RunsOnCell appId={data.appId} eventId={row.eventId} />
							</Td>
						) : (
							cols.map((deviceId) => {
								const cell = row.cells[deviceId];
								return (
									<Td
										key={deviceId}
										label={deviceName(deviceId)}
										className={
											route.focusDeviceId === deviceId
												? "bg-row-selected"
												: undefined
										}
									>
										{cell ? <DeviceCell row={row} cell={cell} /> : null}
									</Td>
								);
							})
						)}
					</Tr>
				);
			})}
			<CantRunGroup span={span} />
		</DvTable>
	);
	return (
		<div
			data-by-event={listMode ? "list" : "matrix"}
			onClickCapture={onClickCapture}
		>
			{listMode ? (
				<AreaEventsDevices appId={data.appId}>{table}</AreaEventsDevices>
			) : (
				table
			)}
		</div>
	);
}
