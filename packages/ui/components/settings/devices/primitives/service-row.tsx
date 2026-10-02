"use client";

import { useTranslation } from "@flow-like/locales";
import type { MouseEvent, ReactNode } from "react";
import { CellSub, Td, Th, Tr } from "./dv-table";
import { PairedPins } from "./paired-pins";
import { RequestedActual, type ServiceStateProps } from "./requested-actual";
import { cx } from "./tone";

export interface ServiceRowProps {
	serviceId: string;
	href?: string;
	app?: { name: string; href?: string };
	/** Outline chips: cloud access, offline writes, endpoint (they wrap, never clip). */
	badges?: ReactNode;
	state: Omit<ServiceStateProps, "className" | "versions">;
	/** Paired pins before the name (N1 Services view, N4); `titled` on the first row of a page. */
	pins?: boolean | "titled";
	instances?: { ready: number; requested: number; max?: number };
	version?: ReactNode;
	versionSub?: ReactNode;
	update?: ReactNode;
	updateSub?: ReactNode;
	/** Contextual actions by requested state, with gate reasons under them. */
	actions?: ReactNode;
	/** Whole-row navigation, except for controls inside the row. */
	onOpen?: () => void;
	selected?: boolean;
	dim?: boolean;
	className?: string;
}

type NameCellProps = Pick<
	ServiceRowProps,
	"serviceId" | "href" | "app" | "badges" | "state" | "pins"
>;

interface TextCellProps {
	label: string;
	value?: ReactNode;
	sub?: ReactNode;
}

/** Column plan for a services table built from `ServiceRow` (SPEC §5 N2/N1). */
export const SERVICE_ROW_COLS = [
	"26%",
	"19%",
	"10%",
	"15%",
	"14%",
	"16%",
] as const;

const NAME = "block truncate font-mono text-[12.5px] font-semibold";

const isControl = (target: EventTarget | null) =>
	target instanceof Element &&
	!!target.closest("a,button,input,select,textarea,[role=menuitem]");

export function ServiceRowHead() {
	const { t } = useTranslation("devices");
	return (
		<tr>
			<Th>{t("view.serviceRow.service", "Service")}</Th>
			<Th>{t("view.serviceRow.state", "Requested → actual")}</Th>
			<Th>{t("view.serviceRow.instances", "Instances")}</Th>
			<Th>{t("view.serviceRow.versions", "Versions")}</Th>
			<Th>{t("view.serviceRow.update", "Update")}</Th>
			<Th>{t("view.serviceRow.actions", "Actions")}</Th>
		</tr>
	);
}

function ServiceName({
	serviceId,
	href,
}: Readonly<Pick<ServiceRowProps, "serviceId" | "href">>) {
	return href ? (
		<a
			href={href}
			title={serviceId}
			className={cx(NAME, "text-foreground no-underline hover:underline")}
		>
			{serviceId}
		</a>
	) : (
		<span title={serviceId} className={NAME}>
			{serviceId}
		</span>
	);
}

function AppLink({ app }: Readonly<{ app: { name: string; href?: string } }>) {
	return (
		<CellSub>
			{app.href ? (
				<a href={app.href} className="hover:text-foreground hover:underline">
					{app.name}
				</a>
			) : (
				app.name
			)}
		</CellSub>
	);
}

function NameCell({
	serviceId,
	href,
	app,
	badges,
	state,
	pins,
}: Readonly<NameCellProps>) {
	const { t } = useTranslation("devices");
	const name = <ServiceName serviceId={serviceId} href={href} />;
	return (
		<Td label={t("view.serviceRow.service", "Service")} kind="name">
			{pins ? (
				<span className="flex min-w-0 items-center gap-2">
					<PairedPins
						desired={state.desired}
						observed={state.observed}
						conv={state.conv}
						title={pins === "titled"}
					/>
					<span className="min-w-0">{name}</span>
				</span>
			) : (
				name
			)}
			{app ? <AppLink app={app} /> : null}
			{badges ? (
				<span className="mt-1.5 flex flex-wrap gap-1">{badges}</span>
			) : null}
		</Td>
	);
}

function InstancesCell({
	instances,
}: Readonly<Pick<ServiceRowProps, "instances">>) {
	const { t } = useTranslation("devices");
	return (
		<Td label={t("view.serviceRow.instances", "Instances")} kind="name">
			{instances ? (
				<>
					{t(
						"view.serviceRow.ready",
						"{{ready, number}} of {{requested, number}} ready",
						{ ready: instances.ready, requested: instances.requested },
					)}
					{instances.max === undefined ? null : (
						<CellSub>
							{t("view.serviceRow.max", "max {{max, number}}", {
								max: instances.max,
							})}
						</CellSub>
					)}
				</>
			) : (
				"–"
			)}
		</Td>
	);
}

function TextCell({ label, value, sub }: Readonly<TextCellProps>) {
	return (
		<Td label={label}>
			{value ?? "–"}
			{sub ? <CellSub>{sub}</CellSub> : null}
		</Td>
	);
}

/** SPEC §4.14: one service in a `DvTable`; every cell carries its card label. */
export function ServiceRow(props: Readonly<ServiceRowProps>) {
	const { serviceId, state, actions, onOpen } = props;
	const { t } = useTranslation("devices");
	const onRowClick = onOpen
		? (event: MouseEvent<HTMLTableRowElement>) => {
				if (!isControl(event.target)) onOpen();
			}
		: undefined;
	return (
		<Tr
			data-service={serviceId}
			selected={props.selected}
			dim={props.dim}
			onClick={onRowClick}
			className={cx(onOpen && "cursor-pointer", props.className)}
		>
			<NameCell
				serviceId={serviceId}
				href={props.href}
				app={props.app}
				badges={props.badges}
				state={state}
				pins={props.pins}
			/>
			<Td label={t("view.serviceRow.state", "Requested → actual")}>
				<RequestedActual {...state} />
			</Td>
			<InstancesCell instances={props.instances} />
			<TextCell
				label={t("view.serviceRow.versions", "Versions")}
				value={props.version}
				sub={props.versionSub}
			/>
			<TextCell
				label={t("view.serviceRow.update", "Update")}
				value={props.update}
				sub={props.updateSub}
			/>
			<Td label={t("view.serviceRow.actions", "Actions")} kind="act">
				{actions ? (
					<div className="flex flex-wrap items-start gap-1.5">{actions}</div>
				) : null}
			</Td>
		</Tr>
	);
}
