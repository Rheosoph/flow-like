"use client";

import { useTranslation } from "@flow-like/locales";
import { ChevronRight, Layers } from "lucide-react";
import { type MouseEvent, useEffect, useId, useRef, useState } from "react";
import {
	fleetFacts,
	keysLocked,
} from "../../../../lib/device-management/model/device-view";
import type { DevicesRoute } from "../../../../lib/device-management/model/types";
import { useAreaTime } from "../primitives/area-context";
import { DvButton } from "../primitives/dv-button";
import { DvSheet } from "../primitives/dv-sheet";
import { StatusChip } from "../primitives/status-chip";
import { type ChipTone, cx } from "../primitives/tone";
import { devicesHref } from "../routing/devices-href";
import {
	PlaneActions,
	type PlaneDeviceRow,
	PlaneDot,
	type PlaneLine,
	type PlaneSegmentId,
	type PlaneState,
	planeCadence,
	planeExplanation,
	planeIcon,
	planeName,
	planeReaders,
	usePlaneLines,
	usePlaneRows,
} from "../shell/plane-popover";
import { plainClick } from "../shell/rail-row";
import { useAttentionState } from "../workspace/use-attention";
import type { OverlaySheetProps } from "./area-overlays";

export interface PlaneSheetProps extends OverlaySheetProps {
	/** The plane the sheet was opened for: its per-device table starts open. */
	plane: PlaneSegmentId;
}

const PLANE_TONE: Record<PlaneState, ChipTone> = {
	ok: "good",
	warn: "warning",
	err: "critical",
	off: "unknown",
	busy: "info",
};

/** Rows shown before "Show all": a 200-device fleet stays one screen per plane (R11). */
const ROW_CAP = 25;

/* The app's base styles give every table margins, cell borders on all sides and a head fill: reset them here. */
const TABLE = "my-0 w-full border-collapse text-xs";
const HEAD_CELL =
	"border-0 bg-transparent px-1.5 py-1.25 text-left text-label font-semibold tracking-[0.06em] text-muted-foreground uppercase";
const CELL =
	"border-0 border-t border-hairline bg-transparent px-1.5 py-1.25 align-top";

interface DeviceRowsProps extends OverlaySheetProps {
	plane: PlaneSegmentId;
}

function DeviceTableRow({
	row,
	scope,
	onNavigate,
	onClose,
}: Readonly<OverlaySheetProps & { row: PlaneDeviceRow }>) {
	const time = useAreaTime();
	const route: DevicesRoute = {
		screen: "device",
		deviceId: row.deviceId,
		tab: "overview",
	};
	const open = (event: MouseEvent) => {
		if (!plainClick(event)) return;
		event.preventDefault();
		onClose();
		onNavigate(route);
	};
	return (
		<tr>
			<td className={cx(CELL, "max-w-[22ch] truncate font-mono")}>
				<a
					href={devicesHref(route, scope)}
					onClick={open}
					title={row.name}
					className="text-foreground underline decoration-border-strong underline-offset-2 hover:decoration-current"
				>
					{row.name}
				</a>
			</td>
			<td className={CELL}>{row.state}</td>
			<td className={cx(CELL, "whitespace-nowrap tabular-nums")}>
				{row.action ?? (row.at === undefined ? "–" : time.ago(row.at))}
			</td>
		</tr>
	);
}

/** One plane's state per device. Mounted only while open, so closed planes cost nothing. */
function DeviceRows({ plane, ...sheet }: Readonly<DeviceRowsProps>) {
	const { t } = useTranslation("devices");
	const rows = usePlaneRows(plane, sheet.onClose);
	const [all, setAll] = useState(false);
	const shown = all ? rows : rows.slice(0, ROW_CAP);
	return (
		<>
			<table data-plane-rows={plane} className={TABLE}>
				<thead>
					<tr>
						<th scope="col" className={HEAD_CELL}>
							{t("overlay.plane.table.device", "Device")}
						</th>
						<th scope="col" className={HEAD_CELL}>
							{t("overlay.plane.table.state", "State")}
						</th>
						<th scope="col" className={HEAD_CELL}>
							{t("overlay.plane.table.age", "Age")}
						</th>
					</tr>
				</thead>
				<tbody>
					{rows.length === 0 ? (
						<tr>
							<td colSpan={3} className={cx(CELL, "text-muted-foreground")}>
								{t("overlay.plane.table.none", "No devices to show here.")}
							</td>
						</tr>
					) : null}
					{shown.map((row) => (
						<DeviceTableRow key={row.deviceId} row={row} {...sheet} />
					))}
				</tbody>
			</table>
			{rows.length > shown.length ? (
				<DvButton
					variant="link"
					size="xs"
					className="self-start text-xs"
					onClick={() => setAll(true)}
				>
					{t("overlay.plane.table.showAll", "Show all {{count, number}}", {
						count: rows.length,
					})}
				</DvButton>
			) : null}
		</>
	);
}

interface PlaneItemProps extends OverlaySheetProps {
	line: PlaneLine;
	focused: boolean;
	locked: number;
}

function PlaneItem({
	line,
	focused,
	locked,
	...sheet
}: Readonly<PlaneItemProps>) {
	const { t } = useTranslation("devices");
	const [open, setOpen] = useState(focused);
	const item = useRef<HTMLLIElement | null>(null);
	const tableId = useId();
	const Icon = planeIcon(line.id);

	useEffect(() => {
		if (focused) item.current?.scrollIntoView?.({ block: "nearest" });
	}, [focused]);

	return (
		<li
			ref={item}
			data-plane={line.id}
			data-state={line.state}
			className="flex flex-col gap-1.5 border-t border-hairline py-3.5 first:border-t-0 first:pt-0 last:pb-0"
		>
			<h3 className="flex items-center gap-2 text-ui font-semibold tracking-normal">
				<PlaneDot state={line.state} />
				<Icon aria-hidden className="size-4 text-muted-foreground" />
				<span className="min-w-0 flex-1 truncate">{planeName(t, line.id)}</span>
				<StatusChip tone={PLANE_TONE[line.state]}>{line.word}</StatusChip>
			</h3>
			<p className="text-ui font-medium">{line.text}</p>
			<p className="max-w-[80ch] text-xs text-muted-foreground">
				{t(
					"overlay.plane.hint",
					"{{explanation}} Refreshed {{cadence}}. Who can read it: {{who}}",
					{
						explanation: planeExplanation(t, line.id),
						cadence: planeCadence(t, line.id),
						who: planeReaders(t, line.id),
					},
				)}
			</p>
			<div className="flex flex-wrap items-center gap-2 empty:hidden">
				<PlaneActions
					plane={line.id}
					state={line.state}
					locked={locked}
					onNavigate={sheet.onNavigate}
					onClose={sheet.onClose}
				/>
			</div>
			<button
				type="button"
				aria-expanded={open}
				aria-controls={tableId}
				onClick={() => setOpen((current) => !current)}
				className="inline-flex items-center gap-1 self-start rounded-sm py-0.5 text-xs font-medium text-ink-2 hover:text-foreground focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring"
			>
				<ChevronRight
					aria-hidden
					className={cx("size-3.5 transition-transform", open && "rotate-90")}
				/>
				{t("overlay.plane.perDevice", "Per device")}
			</button>
			<div id={tableId} className="flex flex-col gap-1.5" hidden={!open}>
				{open ? <DeviceRows plane={line.id} {...sheet} /> : null}
			</div>
		</li>
	);
}

/**
 * SPEC §3.5: all six data sources in one sheet (the compact status bar opens
 * it): what each is, how fresh it is, who can read it and its state per device.
 */
export function PlaneSheet({ plane, ...sheet }: Readonly<PlaneSheetProps>) {
	const { t, i18n } = useTranslation("devices");
	const lines = usePlaneLines();
	const { input } = useAttentionState();
	const locked = fleetFacts(input).devices.filter(
		(device) => device.active && keysLocked(device.keys),
	).length;
	const zone = new Intl.DateTimeFormat(i18n?.language).resolvedOptions()
		.timeZone;
	return (
		<DvSheet
			open
			onOpenChange={(open) => {
				if (!open) sheet.onClose();
			}}
			icon={Layers}
			title={t("overlay.plane.title", "Data sources")}
			sub={t(
				"overlay.plane.subtitle",
				"Where each part of this page comes from and how fresh it is. Times are in this computer's time zone ({{zone}}).",
				{ zone },
			)}
			foot={
				<DvButton onClick={sheet.onClose}>
					{t("overlay.plane.close", "Close")}
				</DvButton>
			}
		>
			<ul
				aria-label={t("overlay.plane.listLabel", "Data sources")}
				className="flex flex-col"
			>
				{lines.map((line) => (
					<PlaneItem
						key={line.id}
						line={line}
						focused={line.id === plane}
						locked={locked}
						{...sheet}
					/>
				))}
			</ul>
		</DvSheet>
	);
}
