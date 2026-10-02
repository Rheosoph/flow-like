"use client";

import { useTranslation } from "@flow-like/locales";
import {
	Copy,
	Ellipsis,
	ExternalLink,
	Info,
	LayoutGrid,
	Link as LinkIcon,
	type LucideIcon,
	Minus,
	Play,
	Plus,
	Radio,
	Rocket,
	RotateCw,
	Server,
	Square,
	Trash2,
} from "lucide-react";
import { type ReactNode, useId, useState } from "react";
import { deviceName } from "../../../../lib/device-management/model/device-view";
import type {
	DeviceViewModel,
	DevicesRoute,
	GateResult,
	PlacementConfigFacts,
	PlacementStatusPlus,
	ServiceView,
} from "../../../../lib/device-management/model/types";
import {
	DropdownMenu,
	DropdownMenuContent,
	DropdownMenuItem,
	DropdownMenuSeparator,
	DropdownMenuTrigger,
} from "../../../ui/dropdown-menu";
import { gateCopy } from "../copy/gate-copy";
import { useAreaTime } from "../primitives/area-context";
import { DvButton } from "../primitives/dv-button";
import { type Gate, GateInline } from "../primitives/gate-notice";
import { InlineConfirm } from "../primitives/inline-confirm";
import { InlineResult } from "../primitives/inline-result";
import { cx } from "../primitives/tone";
import { copyText } from "../primitives/use-copy";
import { useDevicesRoute } from "../routing/use-devices-route";
import { MENU_CONTENT_CLASS, MENU_ITEM_CLASS } from "../shell/area-nav";
import {
	type ServiceCommand,
	serviceGateExtra,
	useFixAction,
	useGates,
	useInlineResults,
	useServiceCommands,
} from "../workspace";
import { type ServiceApp, isBehind } from "./service-header";

const WILDCARD = new Set(["0.0.0.0", "::", "[::]"]);
const LOOPBACK = new Set(["127.0.0.1", "::1", "localhost"]);

/** The service page's address as far as this computer knows it; `undefined` when the service has no web endpoint or its settings weren't read here. */
export function servicePageUrl(
	endpoint: PlacementConfigFacts | undefined,
): string | undefined {
	if (!endpoint?.host || !endpoint.port || WILDCARD.has(endpoint.host))
		return undefined;
	const host = endpoint.host.includes(":")
		? `[${endpoint.host}]`
		: endpoint.host;
	const scheme = endpoint.tlsCertificateId ? "https" : "http";
	return `${scheme}://${host}:${endpoint.port}/ui/`;
}

type Confirming = "start" | "stop" | "restart" | { scale: number };

const GROUP_LABEL =
	"text-label font-semibold uppercase tracking-[0.06em] text-muted-foreground";

function Group({
	label,
	children,
}: Readonly<{ label: string; children: ReactNode }>) {
	return (
		<div className="flex min-w-0 flex-col gap-1.5">
			<span className={GROUP_LABEL}>{label}</span>
			{children}
		</div>
	);
}

function MenuRow({
	icon: Icon,
	label,
	note,
	danger = false,
}: Readonly<{
	icon: LucideIcon;
	label: string;
	note?: string;
	danger?: boolean;
}>) {
	return (
		<>
			<Icon
				aria-hidden
				className={cx("size-4 shrink-0", danger && "text-critical")}
			/>
			<span className="flex min-w-0 flex-col">
				<span className={danger ? "text-critical" : undefined}>{label}</span>
				{note ? (
					<span className="text-xs whitespace-normal text-muted-foreground">
						{note}
					</span>
				) : null}
			</span>
		</>
	);
}

/** SPEC §5.3 action bar: Start / Restart… / Stop…, instances, the service page and the overflow menu. */
export function ServiceActions({
	device,
	service,
	app,
	placement,
	endpoint,
}: Readonly<{
	device: DeviceViewModel;
	service: ServiceView;
	app: ServiceApp;
	placement: PlacementStatusPlus | undefined;
	/** Web endpoint and buffering settings, once the service's settings were read here. */
	endpoint: PlacementConfigFacts | undefined;
}>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const { navigate } = useDevicesRoute();
	const runFix = useFixAction();
	const deviceId = device.row.device_id;
	const { serviceId, projectId } = service;
	const name = deviceName(device.row);
	const commands = useServiceCommands(deviceId, serviceId);
	const results = useInlineResults(commands.resultKey);
	const gates = useGates(["update_service"], deviceId, {
		placementId: serviceId,
		projectId,
		labels: { service: serviceId },
		extra: serviceGateExtra(service),
	});
	const [confirming, setConfirming] = useState<Confirming | null>(null);
	const reasonId = useId();

	const staged = service.rollout?.state === "staged";
	const inline = (gate: GateResult, menu = false) => {
		if (gate.ok) return undefined;
		// A staged update blocks like a running one, but nothing is in progress yet.
		if (!staged || gate.copy.code !== "rollout_in_progress")
			return gateCopy(t, gate, time).inline;
		return menu
			? t(
					"service.menu.stagedFirst",
					"An update is staged. Activate or discard it under Status first.",
				)
			: t(
					"service.actions.stagedFirst",
					"An update is staged. Activate or discard it under Status first. Only Stop is allowed.",
				);
	};
	const gateOf = (gate: GateResult): Gate | null =>
		gate.ok ? null : { kind: gate.kind, reason: inline(gate) };

	const crashed =
		service.desired === "stopped" ||
		service.conv === "crash_looping" ||
		service.conv === "failed_stopped";
	const { requested, ready, max } = service.instances;

	const shown = crashed
		? [commands.start.gate]
		: [commands.restart.gate, commands.stop.gate];
	const reasons = [
		...new Map(
			shown.flatMap((gate) =>
				gate.ok ? [] : [[inline(gate) ?? "", gate] as const],
			),
		).values(),
	];
	const fix = reasons.find((gate) => gate.fix?.kind === "connect")?.fix;
	/** The id of the visible reason line of a gated command, for `aria-describedby`. */
	const reasonOf = (gate: GateResult) => {
		const index = gate.ok
			? -1
			: reasons.findIndex((reason) => inline(reason) === inline(gate));
		return index < 0 ? undefined : `${reasonId}-${index}`;
	};

	const commandButton = (
		command: ServiceCommand,
		kind: "start" | "stop" | "restart",
		label: string,
		icon: LucideIcon,
	) => (
		<DvButton
			icon={icon}
			busy={command.pending}
			aria-disabled={command.gate.ok ? undefined : true}
			aria-describedby={reasonOf(command.gate)}
			data-command={kind}
			onClick={() => setConfirming(kind)}
		>
			{label}
		</DvButton>
	);
	const hint =
		crashed && commands.start.gate.ok
			? service.conv === "crash_looping"
				? t(
						"service.actions.startClears",
						"Start also clears the crash-loop limit.",
					)
				: service.conv === "failed_stopped"
					? t(
							"service.actions.startRuns",
							"Start runs settings v{{settings}} again.",
							{ settings: service.settings.latest },
						)
					: ""
			: "";

	const scaleTo = (replicas: number) => {
		const command = commands.scale(replicas);
		if (!command.gate.ok) return;
		if (replicas < requested) {
			setConfirming({ scale: replicas });
			return;
		}
		setConfirming(null);
		void command.run({ confirmed: true });
	};
	const scaleGate: Gate | null =
		gateOf(commands.scale(requested).gate) ??
		(service.desired === "stopped"
			? {
					kind: "busy",
					reason: t(
						"service.actions.scaleStopped",
						"Start it first. A stopped service runs no instances.",
					),
				}
			: null);
	const scaleShown =
		scaleGate && !reasons.some((gate) => inline(gate) === scaleGate.reason);

	const oneInstance =
		endpoint?.offlineWrites || (placement?.offline_writes?.scopes ?? 0) > 0
			? t(
					"service.actions.oneBuffering",
					"This service runs one instance because write buffering is on.",
				)
			: endpoint && !endpoint.host
				? t(
						"service.actions.oneBackground",
						"This service runs one instance: its background event runs once per device.",
					)
				: t(
						"service.actions.oneMax",
						"This service runs one instance. Raise Max instances in Configuration to run more.",
					);

	const pageUrl = servicePageUrl(endpoint);
	const hosted = !!endpoint?.host && !!endpoint.port;
	const endpointRoute: DevicesRoute = {
		screen: "service",
		deviceId,
		serviceId,
		tab: "endpoint",
	};
	const deployRoute: DevicesRoute = {
		screen: "deploy",
		deviceIds: [deviceId],
		serviceId,
	};
	const behind = isBehind(app.drift);
	const updateLabel =
		behind && app.newestLabel
			? t("service.actions.updateTo", "Update to {{version}}…", {
					version: app.newestLabel,
				})
			: t("service.actions.update", "Update…");
	const updateGate = gates.update_service;
	const served = new Set((app.events ?? []).map((event) => event.id));
	const addable = app.view?.events.rows.some(
		(row) => row.eligibility.eligible && !served.has(row.eventId),
	);
	const addNote = !updateGate.ok
		? inline(updateGate, true)
		: app.view && app.events && !addable
			? t(
					"service.actions.addNone",
					"Every event of {{app}} that can run on a device is already served here.",
					{ app: app.name },
				)
			: undefined;
	const removeDenied =
		!commands.remove.gate.ok && commands.remove.gate.kind === "noaccess";

	const confirmOf = (): ServiceCommand | undefined => {
		if (!confirming) return undefined;
		if (typeof confirming === "string") return commands[confirming];
		return commands.scale(confirming.scale);
	};
	const pendingConfirm = confirmOf();

	return (
		<div className="flex min-w-0 flex-col gap-2">
			<section
				aria-label={t("service.actions.label", "Service actions")}
				data-service-actions=""
				className="flex min-w-0 flex-wrap items-start gap-x-8 gap-y-3 rounded-lg border border-border bg-card px-3 py-2.5"
			>
				<Group label={t("service.actions.group", "Service")}>
					<div className="flex flex-wrap items-center gap-2">
						{crashed ? (
							commandButton(
								commands.start,
								"start",
								t("service.actions.start", "Start"),
								Play,
							)
						) : (
							<>
								{commandButton(
									commands.restart,
									"restart",
									t("service.actions.restart", "Restart…"),
									RotateCw,
								)}
								{commandButton(
									commands.stop,
									"stop",
									t("service.actions.stop", "Stop…"),
									Square,
								)}
							</>
						)}
						{fix ? (
							<DvButton
								size="sm"
								variant="ghost"
								icon={Radio}
								onClick={() => {
									const outcome = runFix(fix);
									if (outcome.kind === "navigate") navigate(outcome.route);
								}}
							>
								{t("service.actions.connect", "Connect live")}
							</DvButton>
						) : null}
					</div>
					{reasons.map((gate, index) => (
						<GateInline
							key={inline(gate)}
							kind={gate.kind}
							id={`${reasonId}-${index}`}
						>
							{inline(gate)}
						</GateInline>
					))}
					{hint ? (
						<span className="inline-flex max-w-[42ch] items-start gap-1 text-xs text-muted-foreground">
							<Info aria-hidden className="mt-0.5 size-3 shrink-0" />
							{hint}
						</span>
					) : null}
				</Group>
				<Group label={t("service.actions.instances", "Instances")}>
					{max <= 1 ? (
						<p className="inline-flex max-w-[42ch] items-start gap-1.5 text-ui text-ink-2">
							<Info
								aria-hidden
								className="mt-0.75 size-3.25 shrink-0 text-muted-foreground"
							/>
							<span>{oneInstance}</span>
						</p>
					) : (
						<>
							<div className="flex flex-wrap items-center gap-2.5">
								<fieldset
									aria-label={t(
										"service.actions.instancesOf",
										"Instances of {{service}}",
										{ service: serviceId },
									)}
									className="inline-flex items-center rounded-lg border border-border"
								>
									<DvButton
										variant="ghost"
										iconOnly
										icon={Minus}
										aria-label={t(
											"service.actions.fewer",
											"One instance fewer",
										)}
										aria-disabled={
											scaleGate || requested <= 1 ? true : undefined
										}
										onClick={() => scaleTo(requested - 1)}
									/>
									<output
										aria-live="polite"
										className="min-w-8 text-center font-mono text-ui tabular-nums"
									>
										{requested}
									</output>
									<DvButton
										variant="ghost"
										iconOnly
										icon={Plus}
										aria-label={t("service.actions.more", "One instance more")}
										aria-disabled={
											scaleGate || requested >= max ? true : undefined
										}
										onClick={() => scaleTo(requested + 1)}
									/>
								</fieldset>
								<span className="text-xs text-muted-foreground tabular-nums">
									{t(
										"service.actions.ofMax",
										"of max {{max}} · {{ready}} ready",
										{ max, ready },
									)}
								</span>
							</div>
							{scaleGate && scaleShown ? (
								<GateInline kind={scaleGate.kind}>
									{scaleGate.reason}
								</GateInline>
							) : null}
						</>
					)}
				</Group>
				<div className="ml-auto flex items-center gap-2">
					{pageUrl ? (
						<DvButton asChild icon={ExternalLink}>
							<a
								href={pageUrl}
								target="_blank"
								rel="noopener noreferrer"
								title={
									LOOPBACK.has(endpoint?.host ?? "")
										? t(
												"service.actions.openLoopback",
												"Opens {{address}} in your browser. Only {{device}} itself can reach this address.",
												{
													address: `${endpoint?.host}:${endpoint?.port}`,
													device: name,
												},
											)
										: pageUrl
								}
							>
								{t("service.actions.openPage", "Open service page")}
							</a>
						</DvButton>
					) : hosted ? (
						<DvButton
							icon={ExternalLink}
							title={t(
								"service.actions.openEndpoint",
								"This service listens on every network of {{device}}. Endpoint has the address people use.",
								{ device: name },
							)}
							onClick={() => navigate(endpointRoute)}
						>
							{t("service.actions.openPageStep", "Open service page…")}
						</DvButton>
					) : null}
					<DropdownMenu>
						<DropdownMenuTrigger asChild>
							<DvButton
								iconOnly
								icon={Ellipsis}
								aria-label={t(
									"service.actions.moreFor",
									"More actions for {{service}}",
									{ service: serviceId },
								)}
							/>
						</DropdownMenuTrigger>
						<DropdownMenuContent
							align="end"
							className={cx(MENU_CONTENT_CLASS, "w-72")}
						>
							<DropdownMenuItem
								className={MENU_ITEM_CLASS}
								onSelect={() => void copyText(serviceId)}
							>
								<MenuRow
									icon={Copy}
									label={t("service.menu.copyId", "Copy service ID")}
								/>
							</DropdownMenuItem>
							<DropdownMenuItem
								className={MENU_ITEM_CLASS}
								onSelect={() => void copyText(service.deploymentId)}
							>
								<MenuRow
									icon={Copy}
									label={t("service.menu.copyDeployment", "Copy deployment ID")}
								/>
							</DropdownMenuItem>
							{pageUrl ? (
								<>
									<DropdownMenuItem asChild className={MENU_ITEM_CLASS}>
										<a href={pageUrl} target="_blank" rel="noopener noreferrer">
											<MenuRow
												icon={ExternalLink}
												label={t(
													"service.actions.openPage",
													"Open service page",
												)}
											/>
										</a>
									</DropdownMenuItem>
									<DropdownMenuItem
										className={MENU_ITEM_CLASS}
										onSelect={() => void copyText(pageUrl)}
									>
										<MenuRow
											icon={LinkIcon}
											label={t(
												"service.menu.copyPageLink",
												"Copy service page link",
											)}
										/>
									</DropdownMenuItem>
								</>
							) : hosted ? (
								<DropdownMenuItem
									className={MENU_ITEM_CLASS}
									onSelect={() => navigate(endpointRoute)}
								>
									<MenuRow
										icon={ExternalLink}
										label={t(
											"service.actions.openPageStep",
											"Open service page…",
										)}
										note={t(
											"service.menu.openPageNote",
											"Its address is on Endpoint",
										)}
									/>
								</DropdownMenuItem>
							) : null}
							<DropdownMenuItem
								className={MENU_ITEM_CLASS}
								onSelect={() =>
									navigate({ screen: "device", deviceId, tab: "services" })
								}
							>
								<MenuRow
									icon={Server}
									label={t("service.menu.openDevice", "Open {{device}}", {
										device: name,
									})}
								/>
							</DropdownMenuItem>
							<DropdownMenuItem
								className={MENU_ITEM_CLASS}
								onSelect={() =>
									navigate(
										{
											screen: "app-devices",
											by: "device",
											focusDeviceId: deviceId,
										},
										{ scope: { kind: "app", appId: projectId } },
									)
								}
							>
								<MenuRow
									icon={LayoutGrid}
									label={t("service.menu.whereRuns", "Where {{app}} runs", {
										app: app.name,
									})}
									note={t(
										"service.menu.whereRunsNote",
										"In the app's settings",
									)}
								/>
							</DropdownMenuItem>
							<DropdownMenuSeparator />
							<DropdownMenuItem
								className={MENU_ITEM_CLASS}
								disabled={!updateGate.ok}
								data-menu="update"
								onSelect={() => navigate(deployRoute)}
							>
								<MenuRow
									icon={Rocket}
									label={updateLabel}
									note={
										inline(updateGate, true) ??
										t("service.menu.inWizard", "In the deploy wizard")
									}
								/>
							</DropdownMenuItem>
							<DropdownMenuItem
								className={MENU_ITEM_CLASS}
								disabled={!!addNote}
								data-menu="add-event"
								onSelect={() => navigate({ ...deployRoute, step: "what" })}
							>
								<MenuRow
									icon={Plus}
									label={t("service.menu.addEvent", "Add an event…")}
									note={addNote}
								/>
							</DropdownMenuItem>
							<DropdownMenuSeparator />
							<DropdownMenuItem
								className={MENU_ITEM_CLASS}
								disabled={removeDenied}
								data-menu="remove"
								onSelect={() =>
									navigate({
										screen: "service",
										deviceId,
										serviceId,
										tab: "configuration",
									})
								}
							>
								<MenuRow
									danger
									icon={Trash2}
									label={t("service.menu.remove", "Remove service…")}
									note={
										removeDenied
											? inline(commands.remove.gate)
											: t(
													"service.menu.removeNote",
													"In Configuration › Danger zone",
												)
									}
								/>
							</DropdownMenuItem>
						</DropdownMenuContent>
					</DropdownMenu>
				</div>
			</section>
			{pendingConfirm && confirming ? (
				<InlineConfirm
					label={t("service.actions.confirm", "Confirm {{action}}", {
						action: pendingConfirm.label,
					})}
					title={pendingConfirm.title}
					sub={pendingConfirm.sub}
					rows={pendingConfirm.rows}
					confirmLabel={pendingConfirm.label}
					tone={pendingConfirm.tone}
					onConfirm={async () => {
						await pendingConfirm.run({ confirmed: true });
						setConfirming(null);
					}}
					onCancel={() => setConfirming(null)}
				/>
			) : null}
			{results.map((result) => (
				<InlineResult
					key={result.id}
					tone={result.tone}
					onDismiss={result.dismiss}
				>
					{result.text}
				</InlineResult>
			))}
		</div>
	);
}
