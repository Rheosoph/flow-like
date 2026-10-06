"use client";

import { useTranslation } from "@flow-like/locales";
import { useQueries } from "@tanstack/react-query";
import {
	Cable,
	Copy,
	Globe,
	Laptop,
	Lock,
	Minus,
	Network,
	Plus,
	Shield,
	ShieldCheck,
	ShieldOff,
} from "lucide-react";
import { useState } from "react";
import {
	type DeviceCertificate,
	readCertificates,
} from "../../../../../lib/device-management/certificates";
import {
	type PlacementConfiguration,
	eventKind,
} from "../../../../../lib/device-management/deployment";
import { ROUTE_EVENT_TYPES } from "../../../../../lib/device-management/event-route";
import {
	DEFAULT_ISOLATION,
	type DeployOverrides,
	type IsolationDraft,
	type PlanException,
	type PlanTarget,
	type TokenMode,
} from "../../../../../lib/device-management/model/deploy-plan";
import { humanFileSize } from "../../../../../lib/utils";
import { type DevicesT, useAreaTime } from "../../primitives/area-context";
import { Block } from "../../primitives/block";
import { ConsequencePreview } from "../../primitives/consequence-preview";
import { DvButton } from "../../primitives/dv-button";
import { DvSheet } from "../../primitives/dv-sheet";
import { DvTable, Td, Th, Tr } from "../../primitives/dv-table";
import {
	CheckField,
	ChoiceCards,
	DvInput,
	Field,
	InputWithUnit,
	SecretInput,
} from "../../primitives/form-fields";
import { GateInline } from "../../primitives/gate-notice";
import { Segmented } from "../../primitives/segmented";
import { StateView } from "../../primitives/state-view";
import { WizardStepHeader } from "../../primitives/wizard";
import { useDeviceWorkspace } from "../../workspace/device-workspace-provider";
import { deviceCall } from "../../workspace/use-live";
import { eventName, issueText, planNames } from "../deploy-copy";
import type { DeployDevice } from "../deploy-facts";
import {
	DeploySelect,
	Disclosure,
	LocalStamp,
	Note,
	type SelectOption,
	TargetsStamp,
} from "../deploy-parts";
import {
	type ExceptionRow,
	ExceptionsTable,
	exceptionRows,
} from "../exceptions-table";
import { serviceWhyText } from "../service-plan";
import type { PlanStepProps } from "../step-props";

/* Step 5 · Endpoint & limits (APP §3.9): shared values first, then the devices that differ. */

type State = PlanStepProps["state"];
type Bind = "local" | "all" | "ip" | "keep";

const MIB = 1024 ** 2;
const GIB = 1024 ** 3;

function bindOf(host: string | null): Bind {
	if (host === null) return "keep";
	if (host === "127.0.0.1") return "local";
	return host === "0.0.0.0" ? "all" : "ip";
}

/** 32 random bytes as URL-safe text: printable, no spaces, long enough for the device. */
function newToken(): string {
	const bytes = globalThis.crypto.getRandomValues(new Uint8Array(32));
	return btoa(String.fromCharCode(...bytes))
		.replaceAll("+", "-")
		.replaceAll("/", "_")
		.replaceAll("=", "");
}

function setOver(
	state: State,
	deviceId: string,
	change: (over: DeployOverrides) => DeployOverrides,
) {
	state.updateTarget(deviceId, (target) => ({
		...target,
		over: change(target.over),
	}));
}

const deviceOf = (state: State, deviceId: string) =>
	state.devices.find((device) => device.id === deviceId);

/** Certificates on each picked, connected device that lets this viewer choose one. */
function useCertificates(
	devices: readonly DeployDevice[],
): Record<string, DeviceCertificate[] | undefined> {
	const workspace = useDeviceWorkspace();
	const able = devices.filter(
		(device) => device.isLive && device.canManageCertificates,
	);
	const results = useQueries({
		queries: able.map((device) => ({
			queryKey: ["devices-deploy-certificates", workspace.scopeKey, device.id],
			queryFn: async () =>
				(await readCertificates(deviceCall(workspace, device.id, "poll")))
					.certificates,
			staleTime: 60_000,
			retry: false,
		})),
	});
	return Object.fromEntries(
		able.map((device, index) => [device.id, results[index]?.data]),
	);
}

/** Events reached through the service listener, including forms when hosting is enabled. */
function servedEvents(state: State, events: readonly string[]) {
	return (state.plan.app?.events ?? []).filter(
		(event) =>
			events.includes(event.id) &&
			(eventKind(event) === "served" ||
				(eventKind(event) === "on_demand" &&
					state.plan.services.some(
						(service) => service.hosted && service.events.includes(event.id),
					))),
	);
}

function hostedNames(state: State): string {
	const { plan } = state;
	const hosted = plan.services.filter((service) => service.hosted);
	const names = hosted.flatMap((service) =>
		servedEvents(state, service.events).map((event) => event.name),
	);
	return [...new Set(names)].join(", ");
}

/** An Endpoint (no Page): one route of the service's web server. */
const isEndpoint = (event: {
	event_type: string;
	default_page_id?: string | null;
}) =>
	!event.default_page_id &&
	(ROUTE_EVENT_TYPES as readonly string[]).includes(event.event_type);

/**
 * Services that put Endpoints together with Pages or chats: one access token
 * calls all of them, so whoever gets it to open a Page can call the Endpoints.
 */
function sharedTokenServices(state: State) {
	if (state.draft.endpoint.token === "none") return [];
	return state.plan.services.flatMap((service) => {
		if (
			state.draft.endpoint.token === "keep" &&
			!state.plan.targets.some((target) =>
				target.services.some(
					(row) =>
						row.key === service.key &&
						state.configurations[target.deviceId]?.find(
							(config) => config.placement_id === row.serviceId,
						)?.config.hosting?.authentication !== "none",
				),
			)
		)
			return [];
		const served = servedEvents(state, service.events);
		const endpoints = served.filter(isEndpoint);
		return endpoints.length && endpoints.length < served.length
			? [
					{
						service: service.id,
						endpoints: endpoints.map((event) => event.name),
					},
				]
			: [];
	});
}

/** The shared-token hint of a service that mixes Endpoints with Pages or chats. */
function SharedTokenNote({ state }: Readonly<{ state: State }>) {
	const { t } = useTranslation("devices");
	const mixed = sharedTokenServices(state);
	if (!mixed.length) return null;
	return (
		<>
			{mixed.map((row) => (
				<Note key={row.service}>
					{t(
						"deploy.endpoint.sharedToken",
						"One access token calls everything {{service}} serves: whoever has it to open a Page or a chat can also call {{endpoints}}. Deploy an Endpoint as its own service to give it a token of its own.",
						{ service: row.service, endpoints: row.endpoints.join(", ") },
					)}
				</Note>
			))}
		</>
	);
}

function BindChoice({ state, update }: Readonly<PlanStepProps>) {
	const { t } = useTranslation("devices");
	const { draft, plan } = state;
	const { endpoint } = draft;
	const bind = bindOf(endpoint.host);
	const [ip, setIp] = useState(bind === "ip" ? (endpoint.host ?? "") : "");
	const [single] = plan.targets;
	const where =
		plan.targets.length === 1 && single
			? single.name
			: t("deploy.endpoint.eachDevice", "each device");
	const pick = (next: Bind) => {
		const host =
			next === "keep"
				? null
				: next === "local"
					? "127.0.0.1"
					: next === "all"
						? "0.0.0.0"
						: ip;
		update({
			endpoint: {
				...endpoint,
				host,
				port: next === "keep" ? endpoint.port : (endpoint.port ?? 8080),
			},
		});
	};
	const invalid = state.check.issues.find(
		(issue) => issue.step === "endpoint" && issue.code === "host_invalid",
	);
	return (
		<>
			<ChoiceCards<Bind>
				id="deploy-bind"
				legend={t("deploy.endpoint.who", "Who can reach it")}
				value={bind}
				onValueChange={pick}
				options={[
					...(draft.entry === "update"
						? [
								{
									value: "keep" as const,
									icon: Lock,
									title: t(
										"deploy.endpoint.keep",
										"Keep each service's address",
									),
									hint: t(
										"deploy.endpoint.keepHint",
										"The address and port stay as they are.",
									),
								},
							]
						: []),
					{
						value: "local",
						icon: Laptop,
						title: t("deploy.endpoint.local", "Only this device"),
						hint: t(
							"deploy.endpoint.localHint",
							"127.0.0.1 · programs on {{where}} itself",
							{ where },
						),
					},
					{
						value: "all",
						icon: Network,
						title: t("deploy.endpoint.all", "All networks"),
						hint: t(
							"deploy.endpoint.allHint",
							"0.0.0.0 · anything that can reach {{where}}",
							{ where },
						),
					},
					{
						value: "ip",
						icon: Cable,
						title: t("deploy.endpoint.ip", "One network address"),
						hint: t(
							"deploy.endpoint.ipHint",
							"One of the device's IP addresses",
						),
					},
				]}
			/>
			{bind === "ip" ? (
				<Field
					id="deploy-ip"
					label={t("deploy.endpoint.ipLabel", "IP address")}
					error={
						invalid
							? t(
									"deploy.endpoint.ipInvalid",
									"Enter an IP address, not a host name.",
								)
							: undefined
					}
					hint={t(
						"deploy.endpoint.ipFieldHint",
						"Each device needs that address; set it per device when they differ.",
					)}
				>
					<DvInput
						mono
						value={ip}
						placeholder="10.0.4.20"
						onChange={(event) => {
							setIp(event.target.value);
							update({ endpoint: { ...endpoint, host: event.target.value } });
						}}
					/>
				</Field>
			) : null}
			{bind === "all" ? (
				<Note tone="warning">
					<b className="font-semibold">
						{t(
							"deploy.endpoint.exposed",
							"Exposed on every network the device is on.",
						)}
					</b>{" "}
					{t(
						"deploy.endpoint.exposedText",
						"Clients can reach {{events}} at this address. Use a certificate to encrypt network traffic.",
						{ events: hostedNames(state) },
					)}
				</Note>
			) : null}
		</>
	);
}

function portHint(t: DevicesT, state: State): string {
	const { plan, facts } = state;
	const [single] = plan.targets;
	if (plan.targets.length !== 1 || !single)
		return t(
			"devices:deploy.endpoint.portHintMulti",
			"Checked on each device against the ports in use there.",
		);
	const used = facts.devices[single.deviceId]?.portsInUse ?? [];
	return used.length
		? t(
				"devices:deploy.endpoint.portInUse",
				"In use on {{device}}: {{ports}}",
				{
					device: single.name,
					ports: used
						.map((row) =>
							row.serviceId
								? `${row.port} (${row.serviceId})`
								: String(row.port),
						)
						.join(", "),
				},
			)
		: t(
				"devices:deploy.endpoint.portHint",
				"1 to 65535. The device checks the port again when you deploy.",
			);
}

function PortField({ state, update }: Readonly<PlanStepProps>) {
	const { t } = useTranslation("devices");
	const { endpoint } = state.draft;
	if (endpoint.port === null) return null;
	const issue = state.check.issues.find(
		(row) => row.step === "endpoint" && row.code === "port_invalid",
	);
	return (
		<Field
			id="deploy-port"
			label={t("deploy.endpoint.port", "Port")}
			className="max-w-[360px]"
			error={
				issue
					? t("deploy.endpoint.portInvalid", "Enter a port from 1 to 65535.")
					: undefined
			}
			hint={portHint(t, state)}
		>
			<DvInput
				numeric
				inputMode="numeric"
				value={Number.isFinite(endpoint.port) ? String(endpoint.port) : ""}
				onChange={(event) =>
					update({
						endpoint: {
							...endpoint,
							port:
								event.target.value === ""
									? Number.NaN
									: Number(event.target.value),
						},
					})
				}
			/>
		</Field>
	);
}

type CertificateValue = "keep" | "none" | (string & {});

function CertificateSelect({
	state,
	target,
	certificates,
}: Readonly<{
	state: State;
	target: PlanTarget;
	certificates: DeviceCertificate[] | undefined;
}>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const device = deviceOf(state, target.deviceId);
	const chosen = target.endpoint.certificateId;
	const value: CertificateValue =
		chosen === undefined ? "keep" : chosen === null ? "none" : chosen;
	const updates = target.services.some((service) => service.kind !== "new");
	const options: SelectOption<CertificateValue>[] = [
		...(updates
			? [
					{
						value: "keep",
						label: t(
							"deploy.endpoint.certKeep",
							"Keep the current certificate",
						),
					},
				]
			: []),
		{
			value: "none",
			label: t("deploy.endpoint.certNone", "No certificate (unencrypted)"),
		},
		...(certificates ?? []).map((certificate) => ({
			value: certificate.certificate_id,
			label: t("deploy.endpoint.certOption", "{{label}} · until {{date}}", {
				label: certificate.label,
				date: time.at(certificate.not_after),
			}),
			disabled: certificate.not_after <= time.nowS,
		})),
	];
	const hint = !device?.canManageCertificates
		? device?.isLive
			? t(
					"deploy.endpoint.certNoRight",
					"Needs Manage certificates on the whole device.",
				)
			: t(
					"deploy.endpoint.certNotLive",
					"Certificates on {{device}} load once it's connected live.",
					{ device: target.name },
				)
		: certificates === undefined
			? t("deploy.endpoint.certLoading", "Reading certificates…")
			: certificates.length
				? t("deploy.endpoint.certCount", {
						count: certificates.length,
						device: target.name,
						defaultValue_one: "{{count, number}} certificate on {{device}}.",
						defaultValue_other: "{{count, number}} certificates on {{device}}.",
					})
				: t(
						"deploy.endpoint.certEmpty",
						"{{device}} has reported no certificates.",
						{ device: target.name },
					);
	return (
		<div className="flex min-w-0 flex-col gap-1">
			<DeploySelect<CertificateValue>
				id={`deploy-certificate-${target.deviceId}`}
				label={t("deploy.endpoint.certOn", "Certificate on {{device}}", {
					device: target.name,
				})}
				value={value}
				disabled={!device?.canManageCertificates}
				options={options}
				onChange={(next) =>
					setOver(state, target.deviceId, (over) => {
						const { certificateId: _dropped, ...rest } = over;
						if (next === "keep") return rest;
						return { ...rest, certificateId: next === "none" ? null : next };
					})
				}
			/>
			<p className="text-xs text-muted-foreground">{hint}</p>
		</div>
	);
}

function Certificates({
	state,
	certificates,
}: Readonly<{
	state: State;
	certificates: Record<string, DeviceCertificate[] | undefined>;
}>) {
	const { t } = useTranslation("devices");
	const { targets } = state.plan;
	const [single] = targets;
	if (!targets.length) return null;
	if (targets.length === 1 && single)
		return (
			<div className="flex max-w-[420px] flex-col gap-1.5">
				<span className="text-[13px]/[18px] font-medium">
					{t("deploy.endpoint.cert", "Certificate")}
				</span>
				<CertificateSelect
					state={state}
					target={single}
					certificates={certificates[single.deviceId]}
				/>
			</div>
		);
	return (
		<div className="flex min-w-0 flex-col gap-1.5">
			<span className="text-[13px]/[18px] font-medium">
				{t("deploy.endpoint.certEach", "Certificate on each device")}
			</span>
			<DvTable
				label={t("deploy.endpoint.certEach", "Certificate on each device")}
				cols={["32%", "68%"]}
				stackAt={560}
				wrapperClassName="rounded-lg border border-border"
				head={
					<tr>
						<Th>{t("deploy.exceptions.device", "Device")}</Th>
						<Th>{t("deploy.endpoint.cert", "Certificate")}</Th>
					</tr>
				}
			>
				{targets.map((target) => (
					<Tr key={target.deviceId} className="hover:bg-transparent">
						<Td label={t("deploy.exceptions.device", "Device")} kind="name">
							<span className="font-mono">{target.name}</span>
						</Td>
						<Td label={t("deploy.endpoint.cert", "Certificate")}>
							<CertificateSelect
								state={state}
								target={target}
								certificates={certificates[target.deviceId]}
							/>
						</Td>
					</Tr>
				))}
			</DvTable>
			<p className="text-xs text-muted-foreground">
				{t(
					"deploy.endpoint.certEachHint",
					"Certificates live on each device, so each one picks its own.",
				)}
			</p>
		</div>
	);
}

function TokenField({ state, update }: Readonly<PlanStepProps>) {
	const { t } = useTranslation("devices");
	const { draft, plan } = state;
	const { endpoint } = draft;
	const multi = plan.targets.length > 1;
	const issue = state.check.issues.find(
		(row) => row.step === "endpoint" && row.code === "token_invalid",
	);
	const pick = (token: TokenMode) =>
		update({
			endpoint: {
				...endpoint,
				token,
				tokenValue: token === "same" ? newToken() : "",
			},
		});
	const body: Record<TokenMode, string> = {
		none: t(
			"deploy.endpoint.tokenNone",
			"Anyone who can reach the service can use its endpoints, Pages, chats and actions without a token. The token set in Events is not used on a device.",
		),
		per_device: t(
			"deploy.endpoint.tokenPerDevice",
			"Generated on this computer for each device; shown once after deploy so you can copy them.",
		),
		same: t(
			"deploy.endpoint.tokenSame",
			"One token generated on this computer and stored on every device; shown once after deploy. Anyone with it can call the service on all of them.",
		),
		own: t(
			"deploy.endpoint.tokenOwn",
			"At least 32 printable characters, no spaces.",
		),
		keep: t(
			"deploy.endpoint.tokenKeep",
			"Keeps each service's current access settings, including whether a token is required. Existing tokens can't be read back.",
		),
	};
	return (
		<div className="flex min-w-0 flex-col gap-1.5">
			<span className="text-[13px]/[18px] font-medium">
				{t("deploy.endpoint.token", "Access token")}
			</span>
			<Segmented<TokenMode>
				label={t("deploy.endpoint.token", "Access token")}
				value={endpoint.token}
				onChange={pick}
				wrap
				className="self-start"
				options={[
					{
						value: "per_device",
						label: multi
							? t("deploy.endpoint.tokenOptPerDevice", "One token per device")
							: t("deploy.endpoint.tokenOptGenerate", "Generate"),
					},
					...(multi
						? [
								{
									value: "same" as const,
									label: t(
										"deploy.endpoint.tokenOptSame",
										"Same token on every device",
									),
								},
							]
						: []),
					{
						value: "own",
						label: t("deploy.endpoint.tokenOptOwn", "Set my own"),
					},
					{
						value: "none",
						label: t("deploy.endpoint.tokenOptNone", "No token"),
					},
					...(draft.entry === "update"
						? [
								{
									value: "keep" as const,
									label: t("deploy.endpoint.tokenOptKeep", "Keep current"),
								},
							]
						: []),
				]}
			/>
			{endpoint.token === "own" ? (
				<SecretInput
					id="deploy-token"
					aria-label={t("deploy.endpoint.token", "Access token")}
					autoComplete="new-password"
					value={endpoint.tokenValue}
					onValueChange={(tokenValue) =>
						update({ endpoint: { ...endpoint, tokenValue } })
					}
					minBytes={32}
					maxBytes={4096}
				/>
			) : null}
			<p className="text-xs text-muted-foreground">{body[endpoint.token]}</p>
			{issue ? (
				<p className="text-xs text-critical">
					{issueText(t, issue, planNames(t, plan))}
				</p>
			) : null}
		</div>
	);
}

/** "Change for this device": the device's own port and address (`targets[].over`). */
function DeviceSheet({
	state,
	deviceId,
	onClose,
}: Readonly<{ state: State; deviceId: string | null; onClose(): void }>) {
	const { t } = useTranslation("devices");
	const target = state.plan.targets.find((row) => row.deviceId === deviceId);
	const own = state.draft.targets.find((row) => row.deviceId === deviceId);
	if (!target || !own || !deviceId) return null;
	const used = state.facts.devices[deviceId]?.portsInUse ?? [];
	return (
		<DvSheet
			open
			onOpenChange={(open) => {
				if (!open) onClose();
			}}
			title={t("deploy.endpoint.sheetTitle", "Endpoint on {{device}}", {
				device: target.name,
			})}
			sub={t(
				"deploy.endpoint.sheetSub",
				"Applies to this device only; the others keep the shared values.",
			)}
			foot={
				<DvButton variant="primary" onClick={onClose}>
					{t("deploy.endpoint.sheetDone", "Done")}
				</DvButton>
			}
		>
			<Field
				id="deploy-own-port"
				label={t("deploy.endpoint.ownPort", "Port on this device")}
				hint={
					used.length
						? t(
								"deploy.endpoint.portInUse",
								"In use on {{device}}: {{ports}}",
								{
									device: target.name,
									ports: used
										.map((row) =>
											row.serviceId
												? `${row.port} (${row.serviceId})`
												: String(row.port),
										)
										.join(", "),
								},
							)
						: t(
								"deploy.endpoint.ownPortHint",
								"Leave empty to use the shared port.",
							)
				}
			>
				<DvInput
					numeric
					inputMode="numeric"
					value={own.over.port === undefined ? "" : String(own.over.port)}
					onChange={(event) =>
						setOver(state, deviceId, (over) => {
							const { port: _dropped, ...rest } = over;
							return event.target.value === ""
								? rest
								: { ...rest, port: Number(event.target.value) };
						})
					}
				/>
			</Field>
			<Field
				id="deploy-own-host"
				label={t("deploy.endpoint.ownHost", "Address on this device")}
				hint={t(
					"deploy.endpoint.ownHostHint",
					"An IP address of this device. Leave empty to use the shared address.",
				)}
			>
				<DvInput
					mono
					value={own.over.host ?? ""}
					onChange={(event) =>
						setOver(state, deviceId, (over) => {
							const { host: _dropped, ...rest } = over;
							return event.target.value === ""
								? rest
								: { ...rest, host: event.target.value };
						})
					}
				/>
			</Field>
		</DvSheet>
	);
}

function UnencryptedCheck({
	state,
	target,
}: Readonly<{ state: State; target: PlanTarget }>) {
	const { t } = useTranslation("devices");
	const own = state.draft.targets.find(
		(row) => row.deviceId === target.deviceId,
	);
	return (
		<CheckField
			id={`deploy-unencrypted-${target.deviceId}`}
			checked={own?.over.allowUnencrypted === true}
			onCheckedChange={(allowUnencrypted) =>
				setOver(state, target.deviceId, (over) => ({
					...over,
					allowUnencrypted,
				}))
			}
		>
			{t(
				"deploy.endpoint.serveUnencrypted",
				"Serve unencrypted on {{device}}",
				{
					device: target.name,
				},
			)}
		</CheckField>
	);
}

function endpointRows(
	t: DevicesT,
	state: State,
	edit: (deviceId: string) => void,
): ExceptionRow[] {
	const { plan, check, draft } = state;
	const names = planNames(t, plan);
	const fromPlan = exceptionRows(
		t,
		plan,
		check.exceptions.filter(
			(exception) =>
				exception.step === "endpoint" && exception.code !== "runs_as_agent",
		),
		(exception: PlanException) => {
			const target = plan.targets.find(
				(row) => row.deviceId === exception.deviceId,
			);
			return exception.code === "no_certificate" && target
				? { check: <UnencryptedCheck state={state} target={target} /> }
				: { onChange: () => edit(exception.deviceId) };
		},
		state.facts,
	);
	const taken = check.issues
		.filter(
			(issue) => issue.step === "endpoint" && issue.code === "port_in_use",
		)
		.map((issue) => ({
			id: `${issue.deviceId}:port_in_use:${issue.params?.port}`,
			device: names.device(issue.deviceId),
			tone: "warning" as const,
			differs: t("devices:deploy.endpoint.portTaken", "Port {{port}}", {
				port: String(issue.params?.port ?? ""),
			}),
			why: issueText(t, issue, names),
			onChange: () => edit(issue.deviceId ?? ""),
		}));
	const own = draft.targets
		.filter((target) => target.over.port !== undefined)
		.filter(
			(target) =>
				!taken.some((row) => row.id.startsWith(`${target.deviceId}:`)),
		)
		.map((target) => ({
			id: `${target.deviceId}:own_port`,
			device: names.device(target.deviceId),
			tone: "info" as const,
			differs: t("devices:deploy.endpoint.portTaken", "Port {{port}}", {
				port: String(target.over.port),
			}),
			why: t(
				"devices:deploy.endpoint.ownPortWhy",
				"You set this port for this device.",
			),
			onChange: () => edit(target.deviceId),
		}));
	return [...fromPlan, ...taken, ...own];
}

function sharedEndpoint(t: DevicesT, state: State): string {
	const { draft, plan } = state;
	const { host, port, token } = draft.endpoint;
	const tokens: Record<TokenMode, string> = {
		none: t("devices:deploy.endpoint.sharedNone", "no token required"),
		per_device: t(
			"devices:deploy.endpoint.sharedPerDevice",
			"one token per device",
		),
		same: t(
			"devices:deploy.endpoint.sharedSame",
			"the same token on every device",
		),
		own: t(
			"devices:deploy.endpoint.sharedOwn",
			"your own token on every device",
		),
		keep: t(
			"devices:deploy.endpoint.sharedKeep",
			"each service keeps its access settings",
		),
	};
	const address =
		host === null || port === null
			? t(
					"devices:deploy.endpoint.sharedKeepAddress",
					"each service keeps its address",
				)
			: `${host}:${port}`;
	return t(
		"devices:deploy.endpoint.shared",
		"Same on every device: {{address}} · {{token}}",
		{ address, token: tokens[token] },
	).concat(
		plan.targets.length
			? ""
			: ` ${t("devices:deploy.endpoint.sharedNoDevices", "Pick devices in Where to see what differs.")}`,
	);
}

function hostingOf(
	configurations: State["configurations"],
): PlacementConfiguration["config"]["hosting"] {
	for (const list of Object.values(configurations))
		for (const row of list) if (row.config.hosting) return row.config.hosting;
	return undefined;
}

/** Hosting values the plan doesn't carry: shown, changed after deploy in Configuration. */
function Advanced({ state }: Readonly<{ state: State }>) {
	const { t } = useTranslation("devices");
	const [open, setOpen] = useState(false);
	const hosting =
		state.draft.entry === "update"
			? hostingOf(state.configurations)
			: undefined;
	return (
		<Disclosure
			label={t("deploy.endpoint.advanced", "Advanced")}
			summary={t(
				"deploy.endpoint.advancedSummary",
				"{{parallel, number}} parallel requests · {{timeout, number}} s timeout",
				{
					parallel: hosting?.max_in_flight ?? 64,
					timeout: hosting?.request_timeout_secs ?? 300,
				},
			)}
			open={open}
			onToggle={() => setOpen(!open)}
		>
			<p className="text-xs text-muted-foreground">
				{t(
					"deploy.endpoint.advancedText",
					"Parallel requests, the request timeout, allowed web origins and the restart policy keep the service's current values, or the defaults for a new service. Change these after deploy in Configuration › Edit as JSON…",
				)}
			</p>
		</Disclosure>
	);
}

function EndpointBlock(props: Readonly<PlanStepProps>) {
	const { t } = useTranslation("devices");
	const { state } = props;
	const { plan, devices, draft } = state;
	const [editing, setEditing] = useState<string | null>(null);
	const picked = devices.filter((device) =>
		plan.targets.some((target) => target.deviceId === device.id),
	);
	const certificates = useCertificates(picked);
	const multi = plan.targets.length > 1;
	const [single] = plan.targets;
	const rows = endpointRows(t, state, setEditing);
	const keeps = draft.endpoint.host === null;
	return (
		<Block
			title={t("deploy.endpoint.title", "Web endpoint for {{events}}", {
				events: hostedNames(state),
			})}
			icon={Globe}
			stamp={<TargetsStamp targets={plan.targets} devices={devices} />}
			foot={
				picked.length && picked.every((device) => device.isLive)
					? t(
							"deploy.endpoint.footLive",
							"Ports and certificates come from each device's last live read.",
						)
					: t(
							"deploy.endpoint.footNotLive",
							"Ports and certificates of devices that aren't connected live aren't loaded. The device checks the port again when you deploy.",
						)
			}
		>
			<BindChoice {...props} />
			{keeps ? null : <PortField {...props} />}
			<Certificates state={state} certificates={certificates} />
			<TokenField {...props} />
			<SharedTokenNote state={state} />
			{multi ? (
				<div className="flex min-w-0 flex-col gap-1.5">
					<span className="text-[13px]/[18px] font-medium">
						{t("deploy.endpoint.onEach", "On each device")}
					</span>
					<ExceptionsTable
						label={t(
							"deploy.endpoint.exceptions",
							"Endpoint differences by device",
						)}
						shared={sharedEndpoint(t, state)}
						rows={rows}
					/>
				</div>
			) : (
				rows.map((row) => (
					<Note key={row.id} tone={row.tone === "warning" ? "warning" : "info"}>
						<b className="font-semibold">{row.differs}.</b> {row.why}
						{row.check ? (
							<span className="mt-1.5 block">{row.check}</span>
						) : null}
						{row.onChange && single ? (
							<>
								{" "}
								<DvButton variant="link" size="xs" onClick={row.onChange}>
									{t("deploy.exceptions.change", "Change for this device")}
								</DvButton>
							</>
						) : null}
					</Note>
				))
			)}
			<Advanced state={state} />
			<DeviceSheet
				state={state}
				deviceId={editing}
				onClose={() => setEditing(null)}
			/>
		</Block>
	);
}

/** The plan's events that a person starts (forms and quick actions). */
function personStarted(state: State): string[] {
	const { plan } = state;
	const ids = new Set(plan.services.flatMap((service) => service.events));
	return (plan.app?.events ?? [])
		.filter((event) => ids.has(event.id) && eventKind(event) === "on_demand")
		.map((event) => event.name);
}

function NoEndpoint({ state }: Readonly<{ state: State }>) {
	const { t } = useTranslation("devices");
	const { plan } = state;
	const started = personStarted(state);
	const names = [
		...new Set(
			plan.services.flatMap((service) =>
				service.events.map((eventId) => eventName(plan, eventId)),
			),
		),
	]
		.filter((name) => !started.includes(name))
		.join(", ");
	return (
		<Block
			title={t("deploy.endpoint.none", "Web endpoint")}
			icon={Globe}
			stamp={
				<LocalStamp text={t("deploy.stamp.fromEvents", "from your events")} />
			}
		>
			<StateView
				kind="empty"
				icon={Globe}
				title={t("deploy.endpoint.noneTitle", "No web endpoint needed")}
				text={
					<>
						{names
							? t(
									"deploy.endpoint.noneText",
									"{{events}} run on their own, so the service doesn't listen on a port.",
									{ events: names },
								)
							: null}
						{started.length ? (
							<span data-person-started="" className="block">
								{t("deploy.endpoint.personStarted", {
									events: started.join(", "),
									count: started.length,
									defaultValue_one:
										"{{events}} starts when a person runs it from Devices.",
									defaultValue_other:
										"{{events}} start when a person runs them from Devices.",
								})}
							</span>
						) : null}
					</>
				}
			/>
		</Block>
	);
}

function InstancesBlock({ state, update }: Readonly<PlanStepProps>) {
	const { t } = useTranslation("devices");
	const { plan, draft } = state;
	const limited = plan.services.find((service) => service.why.length > 0);
	const max = limited ? 1 : 32;
	const value = Math.min(max, Math.max(1, Math.trunc(draft.maxInstances) || 1));
	const why = limited?.why.find((row) => row.code !== "split_variables");
	const reason = why ? serviceWhyText(t, plan, why) : null;
	const kept = plan.targets.some((target) =>
		target.services.some((service) => service.kind !== "new"),
	);
	return (
		<Block
			title={t("deploy.instances.title", "Instances")}
			icon={Copy}
			stamp={<LocalStamp text={t("deploy.stamp.yourChoice", "your choice")} />}
		>
			<div className="flex flex-col gap-1.5">
				<span className="text-[13px]/[18px] font-medium">
					{t("deploy.instances.max", "Max instances")}
				</span>
				<fieldset
					aria-label={t("deploy.instances.title", "Instances")}
					className="m-0 inline-flex w-fit items-center gap-1 rounded-lg border border-border p-0.5"
				>
					<DvButton
						size="sm"
						variant="ghost"
						iconOnly
						icon={Minus}
						aria-label={t("deploy.instances.fewer", "Fewer")}
						disabled={value <= 1}
						onClick={() => update({ maxInstances: value - 1 })}
					/>
					<output className="min-w-8 text-center font-mono text-ui tabular-nums">
						{value}
					</output>
					<DvButton
						size="sm"
						variant="ghost"
						iconOnly
						icon={Plus}
						aria-label={t("deploy.instances.more", "More")}
						disabled={value >= max}
						onClick={() => update({ maxInstances: value + 1 })}
					/>
				</fieldset>
				{reason ? (
					<GateInline kind="policy" className="max-w-none">
						{reason}
					</GateInline>
				) : null}
				{kept ? (
					<p className="text-xs text-muted-foreground">
						{t(
							"deploy.instances.kept",
							"A service that is updated keeps the number it runs now.",
						)}
					</p>
				) : null}
			</div>
		</Block>
	);
}

function policyLine(t: DevicesT, target: PlanTarget, device?: DeployDevice) {
	const name = target.name;
	if (!device?.isolation)
		return {
			icon: Lock,
			text: target.locked
				? t(
						"devices:deploy.isolation.policyLocked",
						"{{device}}'s policy loads when you unlock.",
						{ device: name },
					)
				: t(
						"devices:deploy.isolation.policyUnknown",
						"{{device}}'s isolation policy isn't known yet.",
						{ device: name },
					),
		};
	if (device.isolation === "required")
		return {
			icon: ShieldCheck,
			text: t(
				"devices:deploy.isolation.policyRequired",
				"{{device}} requires sandboxed services.",
				{ device: name },
			),
		};
	return device.isolation === "none"
		? {
				icon: ShieldOff,
				text: t(
					"devices:deploy.isolation.policyNone",
					"{{device}} can't sandbox services.",
					{ device: name },
				),
			}
		: {
				icon: Shield,
				text: t(
					"devices:deploy.isolation.policyOptional",
					"{{device}} · sandbox available.",
					{ device: name },
				),
			};
}

function PolicyBlock({ state }: Readonly<{ state: State }>) {
	const { t } = useTranslation("devices");
	const { plan, devices } = state;
	const [single] = plan.targets;
	if (!plan.targets.length) return null;
	return (
		<Block
			title={
				plan.targets.length > 1 || !single
					? t("deploy.isolation.policyEach", "Each device's policy")
					: t("deploy.isolation.policyOne", "{{device}}'s policy", {
							device: single.name,
						})
			}
			icon={ShieldCheck}
			flush
			stamp={<TargetsStamp targets={plan.targets} devices={devices} />}
			foot={t(
				"deploy.isolation.policyFoot",
				"The device's own policy comes first. Macs can't sandbox services; they run them as the agent.",
			)}
		>
			<ul className="flex flex-col">
				{plan.targets.map((target) => {
					const line = policyLine(t, target, deviceOf(state, target.deviceId));
					return (
						<li
							key={target.deviceId}
							className="flex items-start gap-2.5 border-t border-hairline px-4 py-2.5 text-ui first:border-t-0"
						>
							<line.icon
								aria-hidden
								className="mt-0.5 size-4 shrink-0 text-muted-foreground"
							/>
							{line.text}
						</li>
					);
				})}
			</ul>
		</Block>
	);
}

function LimitFields({
	isolation,
	onChange,
}: Readonly<{
	isolation: IsolationDraft;
	onChange(next: IsolationDraft): void;
}>) {
	const { t } = useTranslation("devices");
	const number = (text: string) => (text === "" ? 0 : Number(text));
	return (
		<>
			<div className="grid grid-cols-4 gap-3 @max-[640px]/endpointstep:grid-cols-2">
				<Field
					id="deploy-cpu"
					label={t("deploy.isolation.cpu", "CPU")}
					hint={t("deploy.isolation.cpuHint", "1.0 = one CPU")}
				>
					<InputWithUnit
						unit={t("deploy.isolation.cores", "cores")}
						numeric
						inputMode="decimal"
						value={String(isolation.cpuMillis / 1000)}
						onChange={(event) =>
							onChange({
								...isolation,
								cpuMillis: Math.round(number(event.target.value) * 1000),
							})
						}
					/>
				</Field>
				<Field
					id="deploy-memory"
					label={t("deploy.isolation.memory", "Memory")}
					hint={t("deploy.isolation.memoryHint", "At least 64 MiB")}
				>
					<InputWithUnit
						unit="MiB"
						numeric
						inputMode="numeric"
						value={String(Math.round(isolation.memoryBytes / MIB))}
						onChange={(event) =>
							onChange({
								...isolation,
								memoryBytes: number(event.target.value) * MIB,
							})
						}
					/>
				</Field>
				<Field
					id="deploy-processes"
					label={t("deploy.isolation.processes", "Processes & threads")}
					hint={t("deploy.isolation.processesHint", "16 to 65,536")}
				>
					<DvInput
						numeric
						inputMode="numeric"
						value={String(isolation.maxProcesses)}
						onChange={(event) =>
							onChange({
								...isolation,
								maxProcesses: number(event.target.value),
							})
						}
					/>
				</Field>
				<Field
					id="deploy-disk"
					label={t("deploy.isolation.disk", "Disk")}
					hint={t("deploy.isolation.diskHint", "Shared by its instances")}
				>
					<InputWithUnit
						unit="GiB"
						numeric
						inputMode="decimal"
						value={String(isolation.diskBytes / GIB)}
						onChange={(event) =>
							onChange({
								...isolation,
								diskBytes: Math.round(number(event.target.value) * GIB),
							})
						}
					/>
				</Field>
			</div>
			<p className="text-xs text-muted-foreground">
				{t(
					"deploy.isolation.limitsHint",
					"Limits apply per instance, except disk. When an instance hits its memory limit, the agent restarts it.",
				)}
			</p>
			<Note tone="warning" icon={Network}>
				<b className="font-semibold">
					{t("deploy.isolation.networkTitle", "Network boundary.")}
				</b>{" "}
				{t(
					"deploy.isolation.network",
					"Sandboxed services still share the device's network and can reach local and cloud metadata addresses.",
				)}
			</Note>
		</>
	);
}

function sharedIsolation(
	t: DevicesT,
	isolation: IsolationDraft,
	anySandbox: boolean,
): string {
	return anySandbox
		? t("devices:deploy.isolation.sharedSandbox", {
				count: isolation.cpuMillis / 1000,
				memory: humanFileSize(isolation.memoryBytes),
				processes: isolation.maxProcesses,
				disk: humanFileSize(isolation.diskBytes),
				defaultValue_one:
					"Same on every device: sandboxed · {{count, number}} core · {{memory}} · {{processes, number}} processes · {{disk}} disk",
				defaultValue_other:
					"Same on every device: sandboxed · {{count, number}} cores · {{memory}} · {{processes, number}} processes · {{disk}} disk",
			})
		: t(
				"devices:deploy.isolation.sharedAgent",
				"Same on every device: runs as the agent",
			);
}

/** "On each device": the shared way to run first, then the devices that run it as the agent. */
function IsolationByDevice({
	state,
	isolation,
	anySandbox,
}: Readonly<{ state: State; isolation: IsolationDraft; anySandbox: boolean }>) {
	const { t } = useTranslation("devices");
	const { plan, check, facts } = state;
	const rows = anySandbox
		? exceptionRows(
				t,
				plan,
				check.exceptions.filter(
					(exception) => exception.code === "runs_as_agent",
				),
				undefined,
				facts,
			)
		: [];
	return (
		<div className="flex min-w-0 flex-col gap-1.5">
			<span className="text-[13px]/[18px] font-medium">
				{t("deploy.endpoint.onEach", "On each device")}
			</span>
			<ExceptionsTable
				label={t(
					"deploy.isolation.exceptions",
					"Isolation differences by device",
				)}
				shared={sharedIsolation(t, isolation, anySandbox)}
				rows={rows}
			/>
		</div>
	);
}

/** One acknowledgement per device that runs the service as the agent. */
function AgentTrust({
	state,
	target,
}: Readonly<{ state: State; target: PlanTarget }>) {
	const { t } = useTranslation("devices");
	const app = state.plan.app?.name ?? "";
	const own = state.draft.targets.find(
		(row) => row.deviceId === target.deviceId,
	);
	const canSandbox = deviceOf(state, target.deviceId)?.isolation !== "none";
	return (
		<section
			aria-label={t(
				"deploy.isolation.trustLabel",
				"Full access on {{device}}",
				{
					device: target.name,
				},
			)}
			data-trust={target.deviceId}
			className="flex flex-col gap-2.5 rounded-lg border border-border-strong bg-card p-3"
		>
			<ConsequencePreview
				compact
				rows={{
					what: t(
						"deploy.isolation.trustWhat",
						"The service runs as the agent's own account on {{device}}.",
						{ device: target.name },
					),
					who: t(
						"deploy.isolation.trustWho",
						"It can read and change everything the agent can, including other services' data and files on {{device}}.",
						{ device: target.name },
					),
					stays: t(
						"deploy.isolation.trustStays",
						"Nothing limits its CPU, memory or disk; a runaway flow can slow down other services.",
					),
					when: t("deploy.isolation.trustWhen", "From the first start."),
					undo: canSandbox
						? {
								reversible: true,
								text: t(
									"deploy.isolation.trustUndo",
									"Deploy again with Sandboxed.",
								),
							}
						: {
								reversible: null,
								text: t(
									"deploy.isolation.trustUndoNone",
									"Not on {{device}}: it can't sandbox. Remove the service, or deploy to a Linux device instead.",
									{ device: target.name },
								),
							},
				}}
			/>
			<CheckField
				id={`deploy-trust-${target.deviceId}`}
				checked={own?.over.trustAgent === true}
				onCheckedChange={(trustAgent) =>
					setOver(state, target.deviceId, (over) => ({ ...over, trustAgent }))
				}
			>
				{t(
					"deploy.isolation.trustCheck",
					"{{device}} runs {{app}} with the agent's full access",
					{ device: target.name, app },
				)}
			</CheckField>
		</section>
	);
}

function IsolationBlock(props: Readonly<PlanStepProps>) {
	const { t } = useTranslation("devices");
	const { state, update, check } = props;
	const { plan, draft } = state;
	const { isolation } = draft;
	const stamp = (
		<LocalStamp text={t("deploy.stamp.yourChoice", "your choice")} />
	);
	if (!isolation)
		return (
			<Block
				title={t("deploy.isolation.title", "Isolation & limits")}
				icon={Shield}
				stamp={stamp}
			>
				<p className="text-ui text-ink-2">
					{t(
						"deploy.isolation.kept",
						"Each service keeps how it runs and its limits.",
					)}
				</p>
				<DvButton
					size="sm"
					className="w-fit"
					onClick={() => update({ isolation: DEFAULT_ISOLATION })}
				>
					{t("deploy.isolation.change", "Change how it runs…")}
				</DvButton>
			</Block>
		);
	const multi = plan.targets.length > 1;
	const [single] = plan.targets;
	// One device: its policy decides when it leaves only one way to run.
	const policy =
		!multi && single ? deviceOf(state, single.deviceId)?.isolation : undefined;
	const anySandbox = plan.targets.length
		? plan.targets.some((target) => !target.runsAsAgent)
		: isolation.profile !== "trusted_process";
	const issues = check.issues.filter(
		(issue) =>
			issue.step === "endpoint" &&
			(issue.code === "isolation_required" ||
				issue.code === "isolation_unavailable"),
	);
	const agents = plan.targets.filter(
		(target) => target.runsAsAgent && !target.locked,
	);
	return (
		<Block
			title={t("deploy.isolation.title", "Isolation & limits")}
			icon={Shield}
			stamp={stamp}
		>
			<ChoiceCards<"sandbox" | "agent">
				id="deploy-profile"
				legend={t("deploy.isolation.how", "How it runs")}
				value={
					isolation.profile === "trusted_process" || policy === "none"
						? "agent"
						: "sandbox"
				}
				onValueChange={(next) =>
					update({
						isolation: {
							...isolation,
							profile: next === "agent" ? "trusted_process" : "auto",
						},
					})
				}
				className="[&_[role=radiogroup]]:grid-cols-2 @max-[560px]/endpointstep:[&_[role=radiogroup]]:grid-cols-1"
				options={[
					{
						value: "sandbox",
						icon: Shield,
						disabled: policy === "none",
						title: multi
							? t(
									"deploy.isolation.sandboxWhere",
									"Sandboxed where the device can",
								)
							: t("deploy.isolation.sandbox", "Sandboxed with limits"),
						hint: multi
							? t(
									"deploy.isolation.sandboxWhereHint",
									"Sandboxed with the limits below; devices that can't sandbox run it as the agent.",
								)
							: t(
									"deploy.isolation.sandboxHint",
									"Its own CPU, memory, process and disk limits. It can't read other services' data.",
								),
					},
					{
						value: "agent",
						icon: ShieldOff,
						disabled: policy === "required",
						title: t("deploy.isolation.agent", "Runs as the agent"),
						hint: t(
							"deploy.isolation.agentHint",
							"Full device access: everything the agent can read and change.",
						),
					},
				]}
			/>
			{policy === "required" && single ? (
				<GateInline kind="policy" className="max-w-none">
					{t(
						"deploy.isolation.lockedSandbox",
						"{{device}} requires sandboxed services, so Sandboxed is the only choice.",
						{ device: single.name },
					)}
				</GateInline>
			) : null}
			{policy === "none" && single ? (
				<GateInline kind="unsupported" className="max-w-none">
					{t(
						"deploy.isolation.lockedAgent",
						"{{device}} can't sandbox services, so it runs as the agent.",
						{ device: single.name },
					)}
				</GateInline>
			) : null}
			{issues.map((issue) => (
				<p
					key={`${issue.code}:${issue.deviceId}`}
					className="text-xs text-critical"
				>
					{issueText(t, issue, planNames(t, plan))}
				</p>
			))}
			{anySandbox ? (
				<LimitFields
					isolation={isolation}
					onChange={(next) => update({ isolation: next })}
				/>
			) : null}
			{multi ? (
				<IsolationByDevice
					state={state}
					isolation={isolation}
					anySandbox={anySandbox}
				/>
			) : null}
			{agents.map((target) => (
				<AgentTrust key={target.deviceId} state={state} target={target} />
			))}
		</Block>
	);
}

export function EndpointLimitsStep(props: Readonly<PlanStepProps>) {
	const { t } = useTranslation("devices");
	const { state, goTo } = props;
	const { plan, app } = state;
	const hosted = plan.services.some((service) => service.hosted);
	const onDemandHostingChoice = plan.services.some((service) => {
		const events = (app?.events ?? []).filter((event) =>
			service.events.includes(event.id),
		);
		if (
			events.some((event) => eventKind(event) === "served") ||
			!events.some((event) => eventKind(event) === "on_demand")
		)
			return false;
		return (
			plan.draft.hostOnDemand === true ||
			!plan.targets.length ||
			plan.targets.some((target) =>
				target.services.some(
					(row) =>
						row.key === service.key &&
						row.events.some((id) => events.some((event) => event.id === id)) &&
						(row.kind === "new" ||
							!state.configurations[target.deviceId]?.find(
								(config) => config.placement_id === row.serviceId,
							)?.config.hosting),
				),
			)
		);
	});
	const limits = t(
		"deploy.endpoint.ledeLimits",
		"No event you picked is served by the device's web server, so only limits apply.",
	);
	const header = (
		<WizardStepHeader
			step={5}
			total={8}
			title={
				hosted
					? t("deploy.step.endpoint", "Endpoint & limits")
					: t("deploy.step.limits", "Limits")
			}
			lede={
				hosted
					? t(
							"deploy.endpoint.lede",
							"How {{events}} is reached, how many instances run, and what they may do on the device.",
							{ events: hostedNames(state) },
						)
					: personStarted(state).length
						? `${limits} ${t(
								"deploy.endpoint.ledeOnDemand",
								"Run its forms and quick actions from Devices: this service has no service page.",
							)}`
						: limits
			}
		/>
	);
	if (!app || !plan.services.length)
		return (
			<div className="flex min-w-0 flex-col gap-4">
				{header}
				<StateView
					kind="notloaded"
					title={t("deploy.settings.pickEvents", "Pick events first")}
					text={t(
						"deploy.endpoint.pickEventsText",
						"The endpoint and the limits depend on the events you deploy.",
					)}
					actions={
						<DvButton size="sm" onClick={() => goTo("what")}>
							{t("deploy.goToWhat", "Go to What")}
						</DvButton>
					}
				/>
			</div>
		);
	return (
		<div className="@container/endpointstep flex min-w-0 flex-col gap-4">
			{header}
			{onDemandHostingChoice ? (
				<Block
					title={t(
						"deploy.endpoint.onDemandTitle",
						"Forms and quick actions in Studio",
					)}
				>
					<CheckField
						id="deploy-host-on-demand"
						checked={plan.draft.hostOnDemand === true}
						onCheckedChange={(hostOnDemand) =>
							state.update({
								hostOnDemand,
								...(hostOnDemand && plan.draft.endpoint.token === "keep"
									? {
											endpoint: { ...plan.draft.endpoint, token: "per_device" },
										}
									: {}),
							})
						}
					>
						{t(
							"deploy.endpoint.onDemandEnable",
							"Open these forms and quick actions as a deployed app",
						)}
					</CheckField>
					<p className="text-sm text-muted-foreground">
						{t(
							"deploy.endpoint.onDemandHint",
							"Creates a device listener with the access settings below. Studio connects through the encrypted tunnel. Services without a listener can still use Run now when this is off.",
						)}
					</p>
				</Block>
			) : null}
			{hosted ? <EndpointBlock {...props} /> : <NoEndpoint state={state} />}
			<InstancesBlock {...props} />
			<PolicyBlock state={state} />
			<IsolationBlock {...props} />
		</div>
	);
}
