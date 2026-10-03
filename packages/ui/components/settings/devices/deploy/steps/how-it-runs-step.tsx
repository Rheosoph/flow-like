"use client";

import { useTranslation } from "@flow-like/locales";
import {
	Archive,
	CircleArrowUp,
	CircleCheck,
	Database,
	FileBraces,
	Folder,
	ListChecks,
	type LucideIcon,
	RefreshCw,
	Send,
	Upload,
} from "lucide-react";
import { type ReactNode, useState } from "react";
import {
	type ArtifactInput,
	pendingArtifactTransfers,
} from "../../../../../lib/device-management/artifacts";
import type { AppMode } from "../../../../../lib/device-management/model/app-plan";
import type { PlanTarget } from "../../../../../lib/device-management/model/deploy-plan";
import { humanFileSize } from "../../../../../lib/utils";
import { appCopy } from "../../copy/app-copy";
import { eligibilityFixLabel } from "../../copy/eligibility-copy";
import { MODE_ICON } from "../../primitives/app-chips";
import { type DevicesT, useAreaTime } from "../../primitives/area-context";
import { Banner } from "../../primitives/banner";
import { Block } from "../../primitives/block";
import { Checklist, type ChecklistItem } from "../../primitives/checklist";
import { DvButton } from "../../primitives/dv-button";
import { FreshnessStamp } from "../../primitives/freshness-stamp";
import { GateNotice } from "../../primitives/gate-notice";
import { IdRef } from "../../primitives/id-ref";
import { KeyValueList, KvRow } from "../../primitives/key-value-list";
import { StateView } from "../../primitives/state-view";
import { WizardStepHeader } from "../../primitives/wizard";
import { appEventsHref } from "../../routing/devices-href";
import { useHostLink } from "../../routing/use-devices-route";
import { useDeviceWorkspace } from "../../workspace/device-workspace-provider";
import {
	flowLabel,
	prepareFailureText,
	preparedFlowText,
} from "../deploy-copy";
import { Disclosure, HeadChip, Note } from "../deploy-parts";
import type { PlanStepProps } from "../step-props";
import type {
	DeployPrepareState,
	ImportedCopy,
	PrepareCheck,
	PrepareCheckId,
	PrepareFailure,
} from "../use-deploy-prepare";
import { useFlowNames } from "../use-deploy-reads";

/* Step 2 · How it runs (APP §3.6): the mode as a fact, what is sent, and the preparation on this computer. */

const MAX_GROUPS = 8;

function ModeCard({ app, mode }: Readonly<{ app: string; mode: AppMode }>) {
	const { t } = useTranslation("devices");
	const copy = appCopy(t);
	const text = copy.mode(mode, app);
	const Icon = MODE_ICON[mode];
	const online = mode === "online";
	return (
		<section
			data-mode-card={mode}
			aria-label={text.chip}
			className="@container/modecard flex min-w-0 flex-col gap-2.5 rounded-lg border border-border bg-surface-sunken px-4 py-3"
		>
			<div className="flex min-w-0 items-center gap-3">
				<span className="inline-flex size-9 shrink-0 items-center justify-center rounded-lg border border-border bg-card text-ink-2">
					<Icon aria-hidden className="size-4.5" />
				</span>
				<div className="min-w-0">
					<p className="text-[15px]/5 font-semibold">{text.chip}</p>
					<p className="text-ui text-muted-foreground">
						{copy.explainerSub(mode, app)}.
					</p>
				</div>
			</div>
			<div className="flex min-w-0 flex-col gap-2 @min-[680px]/modecard:pl-12">
				<KeyValueList>
					<KvRow label={t("deploy.how.rowData", "Data")}>
						{text.rows.data}
					</KvRow>
					<KvRow label={copy.explainerRowLabel("internet")}>
						{online
							? t(
									"deploy.how.internetOnline",
									"Needed. Services get cloud access 10 minutes at a time.",
								)
							: text.rows.internet}
					</KvRow>
					<KvRow label={copy.explainerRowLabel("updates")}>
						{text.rows.updates}
					</KvRow>
					<KvRow label={copy.explainerRowLabel("cost")}>
						{online
							? t(
									"deploy.how.costOnline",
									"Model use is charged to whoever sets the spending limit. Files written count against the approver's storage.",
								)
							: t(
									"deploy.how.costOffline",
									"Model use only if you approve hosted models, up to your limit.",
								)}
					</KvRow>
					<KvRow label={t("deploy.how.outages", "During outages")}>
						{online
							? t(
									"deploy.how.outagesOnline",
									"With write buffering, services keep accepting changes and send them when the internet is back.",
								)
							: t(
									"deploy.how.outagesOffline",
									"Nothing to buffer: the data never leaves the device.",
								)}
					</KvRow>
				</KeyValueList>
				<details className="text-ui">
					<summary className="w-fit cursor-pointer font-medium focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring">
						{t("deploy.how.whyNot", "Why not the other way?")}
					</summary>
					<p className="mt-1.5 max-w-[72ch] text-ui text-ink-2">
						{text.whyNot}
					</p>
				</details>
			</div>
		</section>
	);
}

const CHECK_SOURCE: Record<PrepareCheckId, "hub" | "local"> = {
	// An online app's flows are published on the hub; a local-only app's on this computer (see `checkSource`).
	publish_flows: "hub",
	read_hub: "hub",
	check_events: "hub",
	collect: "local",
	approved: "local",
	read_app: "local",
	no_secrets: "local",
	no_history: "local",
	no_writers: "local",
	limits: "local",
};

const checkSource = (id: PrepareCheckId, mode: AppMode | null) =>
	id === "publish_flows" && mode === "offline" ? "local" : CHECK_SOURCE[id];

function checkLabel(
	t: DevicesT,
	id: PrepareCheckId,
	prepare: DeployPrepareState,
): string {
	const descriptor = prepare.prepared?.artifact.descriptor;
	switch (id) {
		case "publish_flows":
			return t(
				"devices:deploy.prepare.publishFlows",
				"Create flow versions for current edits",
			);
		case "read_hub":
			return t(
				"devices:deploy.prepare.readHub",
				"Reading what's published on the hub",
			);
		case "check_events":
			return t(
				"devices:deploy.prepare.checkEvents",
				"Checking which events can run on devices",
			);
		case "collect":
			return t(
				"devices:deploy.prepare.collect",
				"Collecting models and packages",
			);
		case "approved":
			return t("devices:deploy.prepare.approved", "Definitions approved");
		case "read_app":
			return prepare.imported
				? t("devices:deploy.prepare.readImport", "Reading the prepared copy")
				: t(
						"devices:deploy.prepare.readApp",
						"Reading the app on this computer",
					);
		case "no_secrets":
			return t(
				"devices:deploy.prepare.noSecrets",
				"No saved secret values in flows",
			);
		case "no_history":
			return t(
				"devices:deploy.prepare.noHistory",
				"No flow needs table history or search indexes",
			);
		case "no_writers":
			return t(
				"devices:deploy.prepare.noWriters",
				"Nothing else is writing to the app",
			);
		default:
			return descriptor
				? t(
						"devices:deploy.prepare.limitsOf",
						"Under the limits: {{bytes}} of 8 GiB, {{files, number}} of 8,192 files",
						{
							bytes: humanFileSize(descriptor.total_bytes),
							files: descriptor.file_count,
						},
					)
				: t(
						"devices:deploy.prepare.limits",
						"Under the limits: 8 GiB and 8,192 files",
					);
	}
}

/** What the first check did, per flow; without a Latest event among the choices it says that nothing is created. */
function PublishedFlows({
	check,
	state,
}: Readonly<{ check: PrepareCheck; state: PlanStepProps["state"] }>) {
	const { t } = useTranslation("devices");
	const names = useFlowNames(state.plan.app?.id, !!check.flows?.length);
	if (check.flows?.length)
		return (
			<span data-flows="">
				{check.flows
					.map((flow) =>
						preparedFlowText(
							t,
							flow,
							flowLabel(t, state.plan, names, flow.boardId),
						),
					)
					.join(" · ")}
			</span>
		);
	return (
		<>
			{t(
				"deploy.prepare.publishFlows",
				"Create flow versions for current edits",
			)}
			{check.state === "skip" ? (
				<span className="text-muted-foreground">
					{t(
						"deploy.prepare.publishFlowsNone",
						" · none of the chosen events follows Latest",
					)}
				</span>
			) : null}
		</>
	);
}

/** A Latest event whose Page or start node is gone is fixed in Events; preparing again would stop at the same place. */
function PrepareFix({
	failure,
	appId,
	again,
}: Readonly<{ failure: PrepareFailure; appId: string; again(): void }>) {
	const { t } = useTranslation("devices");
	const hostLink = useHostLink();
	if (failure.kind === "flow" && failure.flow === "target" && failure.eventId)
		return (
			<DvButton size="xs" asChild>
				<a {...hostLink(appEventsHref(appId, failure.eventId))}>
					{eligibilityFixLabel(t, "open_events")}
				</a>
			</DvButton>
		);
	return (
		<DvButton size="xs" icon={RefreshCw} onClick={again}>
			{t("deploy.prepare.again", "Prepare again")}
		</DvButton>
	);
}

function PreparingBlock({ state, prepare, prepared }: Readonly<PlanStepProps>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const { plan, mode } = state;
	const hash = prepared?.approved?.sha256;
	const item = (check: PrepareCheck): ChecklistItem => {
		const failed = check.state === "fail" && prepare.failure;
		return {
			id: check.id,
			state: check.state,
			source: checkSource(check.id, mode),
			label: failed ? (
				prepareFailureText(t, failed, plan, state.facts.hub?.hubTypes)
			) : check.id === "publish_flows" ? (
				<PublishedFlows check={check} state={state} />
			) : (
				<>
					{checkLabel(t, check.id, prepare)}
					{check.id === "approved" && check.state === "pass" && hash ? (
						<>
							{": "}
							<IdRef
								id={hash}
								copyLabel={t(
									"deploy.prepare.copyHash",
									"Copy definitions hash",
								)}
							/>
						</>
					) : null}
					{check.state === "skip" ? (
						<span className="text-muted-foreground">
							{t("deploy.prepare.notChecked", " · not checked")}
						</span>
					) : null}
				</>
			),
			...(failed
				? {
						fix: (
							<PrepareFix
								failure={failed}
								appId={plan.app?.id ?? ""}
								again={prepare.again}
							/>
						),
					}
				: {}),
		};
	};
	return (
		<Block
			title={t("deploy.prepare.title", "Preparing")}
			icon={ListChecks}
			stamp={
				<FreshnessStamp
					source="local"
					age={prepared ? "current" : "notloaded"}
					text={
						prepared
							? t("deploy.prepare.stampDone", "prepared {{ago}}", {
									ago: time.ago(prepared.preparedAt / 1000),
								})
							: prepare.state === "blocked"
								? t("deploy.prepare.stampBlocked", "not prepared")
								: t("deploy.prepare.stampRunning", "preparing…")
					}
				/>
			}
			foot={
				mode === "online"
					? t(
							"deploy.prepare.footOnline",
							"Prepared once, then sent to every device. Each device checks the definitions' hash before it uses them.",
						)
					: t(
							"deploy.prepare.footOffline",
							"Prepared once on this computer, then sent to every device. Every device gets the same files, checked by their hashes.",
						)
			}
		>
			<Checklist
				label={t("deploy.prepare.title", "Preparing")}
				items={prepare.checks.map(item)}
			/>
		</Block>
	);
}

/** Blocks whose facts come from the prepared bundle say whether it exists yet (R5). */
function PreparedStamp({
	prepared,
}: Readonly<Pick<PlanStepProps, "prepared">>) {
	const { t } = useTranslation("devices");
	return (
		<FreshnessStamp
			source="local"
			age={prepared ? "current" : "notloaded"}
			text={
				prepared
					? t("deploy.how.shipPrepared", "prepared on this computer")
					: t("deploy.how.shipPending", "not prepared yet")
			}
		/>
	);
}

const Unknown = () => {
	const { t } = useTranslation("devices");
	return (
		<span className="text-muted-foreground">
			{t("deploy.how.knownOncePrepared", "Known once it's prepared")}
		</span>
	);
};

function OnlineShip({ prepared }: Readonly<PlanStepProps>) {
	const { t } = useTranslation("devices");
	const approved = prepared?.approved;
	const descriptor = prepared?.artifact.descriptor;
	const packages = Object.keys(approved?.app.packages ?? {});
	return (
		<KeyValueList>
			<KvRow label={t("deploy.how.definitions", "Approved definitions")}>
				{approved ? (
					<>
						{t("deploy.how.definitionsValue", {
							count: approved.catalog.events.length,
							size: humanFileSize(approved.file.file.size),
							defaultValue_one: "{{count, number}} event · {{size}} · hash",
							defaultValue_other: "{{count, number}} events · {{size}} · hash",
						})}{" "}
						<IdRef
							id={approved.sha256}
							copyLabel={t("deploy.prepare.copyHash", "Copy definitions hash")}
						/>
					</>
				) : (
					<Unknown />
				)}
			</KvRow>
			<KvRow label={t("deploy.how.models", "Models")}>
				{approved ? (
					approved.app.bits.length ? (
						t("deploy.how.modelsValue", {
							count: approved.app.bits.length,
							defaultValue_one: "{{count, number}} model",
							defaultValue_other: "{{count, number}} models",
						})
					) : (
						t("deploy.how.none", "None")
					)
				) : (
					<Unknown />
				)}
			</KvRow>
			<KvRow label={t("deploy.how.packages", "Packages")}>
				{approved ? (
					packages.length ? (
						<span className="font-mono">{packages.join(", ")}</span>
					) : (
						t("deploy.how.none", "None")
					)
				) : (
					<Unknown />
				)}
			</KvRow>
			<KvRow label={t("deploy.how.data", "Data")}>
				{t(
					"deploy.how.dataOnline",
					"Stays in the cloud. Each device keeps a read cache of up to 512 MiB.",
				)}
			</KvRow>
			<KvRow label={t("deploy.how.total", "Total")}>
				{descriptor ? (
					<b className="font-semibold">
						{t("deploy.how.totalValue", {
							count: descriptor.file_count,
							size: humanFileSize(descriptor.total_bytes),
							defaultValue_one: "{{count, number}} file · {{size}} per device",
							defaultValue_other:
								"{{count, number}} files · {{size}} per device",
						})}
					</b>
				) : (
					<Unknown />
				)}
			</KvRow>
		</KeyValueList>
	);
}

interface FileGroup {
	name: string;
	icon: LucideIcon;
	files: number;
	bytes: number;
}

/** Where a file of the copy sits: models and packages are shared folders, the rest is the app's own folder or file. */
function fileGroup(path: string): Pick<FileGroup, "name" | "icon"> {
	const parts = path.split("/");
	if (parts[0] === "bits" || parts[0] === "packages")
		return { name: `${parts[0]}/`, icon: Archive };
	const name = parts[2] ?? parts[0];
	return parts.length > 3
		? { name: `${name}/`, icon: Folder }
		: { name, icon: FileBraces };
}

/** The copy's files by where they sit, the largest group first. */
function groupFiles(files: readonly ArtifactInput[]): FileGroup[] {
	const groups = new Map<string, FileGroup>();
	for (const { path, file } of files) {
		const { name, icon } = fileGroup(path);
		const group = groups.get(name) ?? { name, icon, files: 0, bytes: 0 };
		group.files += 1;
		group.bytes += file.size;
		groups.set(name, group);
	}
	return [...groups.values()].sort((a, b) => b.bytes - a.bytes);
}

function OfflineShip({ prepared }: Readonly<PlanStepProps>) {
	const { t } = useTranslation("devices");
	if (!prepared)
		return (
			<p className="px-4 py-3 text-ui text-muted-foreground">
				{t(
					"deploy.how.copyUnknown",
					"The copy's files and size show once it's prepared.",
				)}
			</p>
		);
	const groups = groupFiles(prepared.artifact.files);
	const shown = groups.slice(0, MAX_GROUPS);
	const rest = groups.slice(MAX_GROUPS);
	const { file_count, total_bytes } = prepared.artifact.descriptor;
	const filesText = (count: number) =>
		t("deploy.how.files", {
			count,
			defaultValue_one: "{{count, number}} file",
			defaultValue_other: "{{count, number}} files",
		});
	const row =
		"grid grid-cols-[18px_minmax(0,1fr)_auto_auto] items-center gap-x-3 border-t border-hairline px-4 py-2 text-ui first:border-t-0";
	return (
		<ul data-ship-list="" className="flex flex-col">
			{shown.map((group) => (
				<li key={group.name} className={row}>
					<group.icon aria-hidden className="size-4 text-muted-foreground" />
					<b className="truncate font-mono font-medium">{group.name}</b>
					<span className="text-xs text-muted-foreground tabular-nums">
						{filesText(group.files)}
					</span>
					<span className="min-w-[8ch] text-right font-mono text-xs tabular-nums">
						{humanFileSize(group.bytes)}
					</span>
				</li>
			))}
			{rest.length ? (
				<li className={row}>
					<span />
					<span className="text-muted-foreground">
						{t("deploy.how.moreGroups", {
							count: rest.length,
							defaultValue_one: "{{count, number}} more folder",
							defaultValue_other: "{{count, number}} more folders",
						})}
					</span>
					<span className="text-xs text-muted-foreground tabular-nums">
						{filesText(rest.reduce((sum, group) => sum + group.files, 0))}
					</span>
					<span className="min-w-[8ch] text-right font-mono text-xs tabular-nums">
						{humanFileSize(rest.reduce((sum, group) => sum + group.bytes, 0))}
					</span>
				</li>
			) : null}
			<li className={`${row} bg-surface-sunken font-semibold`}>
				<span />
				<span>{t("deploy.how.toEach", "To each device")}</span>
				<span className="text-xs tabular-nums">{filesText(file_count)}</span>
				<span className="min-w-[8ch] text-right font-mono text-xs tabular-nums">
					{humanFileSize(total_bytes)}
				</span>
			</li>
		</ul>
	);
}

function ShipBlock(props: Readonly<PlanStepProps>) {
	const { t } = useTranslation("devices");
	const { mode } = props.state;
	const online = mode === "online";
	const files = online ? undefined : props.prepared?.artifact.descriptor;
	return (
		<Block
			title={t("deploy.how.ship", "What goes to each device")}
			icon={Send}
			flush={!online}
			summary={
				files ? (
					<HeadChip>
						{t("deploy.how.files", {
							count: files.file_count,
							defaultValue_one: "{{count, number}} file",
							defaultValue_other: "{{count, number}} files",
						})}
					</HeadChip>
				) : undefined
			}
			stamp={<PreparedStamp prepared={props.prepared} />}
			foot={
				online
					? undefined
					: t(
							"deploy.how.shipFoot",
							"Tables are copied with their current rows only: no history and no search indexes. Your files in this app go too; they become the device's local user.",
						)
			}
		>
			{online ? <OnlineShip {...props} /> : <OfflineShip {...props} />}
		</Block>
	);
}

const updateTargets = (targets: readonly PlanTarget[]) =>
	targets.flatMap((target) =>
		target.services
			.filter((service) => service.kind !== "new")
			.map((service) => ({ target, service })),
	);

/** Updates: what the newest version means for each service (APP §3.14). */
function UpdateBlock({ state, prepared }: Readonly<PlanStepProps>) {
	const { t } = useTranslation("devices");
	const { plan, draft, mode, devices } = state;
	const rows = updateTargets(plan.targets);
	if (!rows.length || !plan.app) return null;
	if (draft.version === "keep")
		return (
			<StateView
				kind="empty"
				icon={CircleCheck}
				title={t("deploy.how.nothingUploaded", "Nothing is uploaded")}
				text={t(
					"deploy.how.nothingUploadedText",
					"Each device already has the version its service runs. Only settings or events change.",
				)}
			/>
		);
	const online = mode === "online";
	const size = prepared
		? humanFileSize(prepared.artifact.descriptor.total_bytes)
		: null;
	const running = rows.some(({ target, service }) =>
		devices
			.find((device) => device.id === target.deviceId)
			?.services?.some(
				(row) =>
					row.serviceId === service.serviceId && row.desired === "running",
			),
	);
	const line = (target: PlanTarget, service: string): ReactNode =>
		online
			? t(
					"deploy.how.repin",
					"Re-pins {{service}} on {{device}} to the event and flow versions published now. Data stays in the cloud and isn't touched.",
					{ service, device: target.name },
				)
			: t(
					"deploy.how.reupload",
					"Sends a new copy of {{app}} from this computer to {{device}}. It replaces the app that {{service}} runs.",
					{ app: plan.app?.name ?? "", device: target.name, service },
				);
	return (
		<Block
			title={
				online
					? t("deploy.how.repinTitle", "Re-pin to the newest version")
					: t("deploy.how.reuploadTitle", "Re-upload the copy")
			}
			icon={online ? CircleArrowUp : Upload}
			stamp={<PreparedStamp prepared={prepared} />}
			foot={
				running
					? online
						? t(
								"deploy.how.footRunningOnline",
								"The service keeps running its installed version until the switch.",
							)
						: t(
								"deploy.how.footRunningOffline",
								"The copy goes to the device before the switch; the running version keeps going until then.",
							)
					: t(
							"deploy.how.footStopped",
							"Nothing is running there now, so the update interrupts nothing.",
						)
			}
		>
			<ul className="flex list-disc flex-col gap-1 pl-5 text-ui">
				{rows.map(({ target, service }) => (
					<li key={`${target.deviceId}/${service.serviceId}`}>
						{line(target, service.serviceId)}
					</li>
				))}
			</ul>
			{size ? (
				<p className="text-xs text-muted-foreground">
					{t("deploy.how.sentSize", "About {{size}} is sent to each device.", {
						size,
					})}
				</p>
			) : null}
			{online ? null : (
				<Banner
					tone="info"
					icon={Database}
					title={t("deploy.how.dataStays", "The data on the device stays.")}
				>
					{t(
						"deploy.how.dataStaysText",
						"Updates replace the app, never the data on the device: the tables in the new copy aren't used. To start over with fresh data, deploy a new service.",
					)}
				</Banner>
			)}
		</Block>
	);
}

function FolderPick({
	id,
	icon: Icon,
	title,
	hint,
	folder,
	chosen,
	onFiles,
}: Readonly<{
	id: string;
	icon: LucideIcon;
	title: string;
	hint: string;
	folder: boolean;
	chosen?: string;
	onFiles(files: File[]): void;
}>) {
	return (
		<label
			htmlFor={id}
			className="flex cursor-pointer flex-col items-center gap-1 rounded-lg border border-dashed border-border-strong bg-surface-sunken px-4 py-4 text-center text-ink-2 hover:border-foreground hover:bg-row-hover focus-within:outline-2 focus-within:outline-offset-2 focus-within:outline-ring"
		>
			<Icon aria-hidden className="size-5 text-muted-foreground" />
			<span className="text-ui font-medium">{title}</span>
			<span className="text-xs text-muted-foreground">{chosen ?? hint}</span>
			<input
				id={id}
				type="file"
				multiple={folder}
				accept={folder ? undefined : ".json,application/json"}
				className="sr-only"
				{...(folder ? { webkitdirectory: "" } : {})}
				onChange={(event) => {
					const files = Array.from(event.target.files ?? []);
					if (files.length) onFiles(files);
					event.target.value = "";
				}}
			/>
		</label>
	);
}

/** Advanced (offline, new service, desktop): a copy prepared elsewhere, checked the same way. */
function ImportCopy({ prepare }: Readonly<PlanStepProps>) {
	const { t } = useTranslation("devices");
	const [open, setOpen] = useState(false);
	const [copy, setCopy] = useState<ImportedCopy>({
		files: [],
		pins: "",
		assets: [],
	});
	const apply = (next: ImportedCopy) => {
		setCopy(next);
		if (next.files.length) prepare.importCopy(next);
	};
	const count = (files: File[]) =>
		files.length
			? t("deploy.import.chosen", {
					count: files.length,
					defaultValue_one: "{{count, number}} file chosen",
					defaultValue_other: "{{count, number}} files chosen",
				})
			: undefined;
	return (
		<Disclosure
			label={t("deploy.import.title", "Import a prepared copy instead")}
			summary={t("deploy.import.advanced", "advanced")}
			open={open}
			onToggle={() => setOpen(!open)}
		>
			<div className="grid gap-2">
				<FolderPick
					id="deploy-import-folder"
					icon={Folder}
					folder
					title={t("deploy.import.folder", "Choose an offline app folder")}
					hint={t(
						"deploy.import.folderHint",
						"1 to 8,192 files with manifest.app at the top · up to 4 GiB per file, 8 GiB in total",
					)}
					chosen={count(copy.files)}
					onFiles={(files) => apply({ ...copy, files })}
				/>
				<FolderPick
					id="deploy-import-pins"
					icon={FileBraces}
					folder={false}
					title={t("deploy.import.pins", "Choose a public asset pins file")}
					hint={t(
						"deploy.import.pinsHint",
						".json · the models and packages the copy needs",
					)}
					chosen={
						copy.pins
							? t("deploy.import.pinsChosen", "Pins file chosen")
							: undefined
					}
					onFiles={([file]) =>
						void file.text().then((pins) => apply({ ...copy, pins }))
					}
				/>
				<FolderPick
					id="deploy-import-assets"
					icon={Archive}
					folder
					title={t("deploy.import.assets", "Choose an offline assets folder")}
					hint={t(
						"deploy.import.assetsHint",
						"The folder with the files the pins point to",
					)}
					chosen={count(copy.assets)}
					onFiles={(assets) => apply({ ...copy, assets })}
				/>
				{prepare.imported ? (
					<DvButton
						size="sm"
						variant="ghost"
						className="w-fit"
						onClick={() => {
							setCopy({ files: [], pins: "", assets: [] });
							prepare.importCopy(null);
						}}
					>
						{t("deploy.import.useApp", "Use the app on this computer again")}
					</DvButton>
				) : null}
			</div>
		</Disclosure>
	);
}

/** An upload of this app that this computer began on a picked device and that hasn't finished. */
function UnfinishedUploads({ state, goTo }: Readonly<PlanStepProps>) {
	const { t } = useTranslation("devices");
	const workspace = useDeviceWorkspace();
	const { plan, draft } = state;
	const waiting = plan.targets.filter(
		(target) =>
			pendingArtifactTransfers(
				target.deviceId,
				draft.appId ?? undefined,
				workspace.deps.scope,
			).length > 0,
	);
	if (!waiting.length) return null;
	return (
		<Note icon={Upload}>
			{t(
				"deploy.how.unfinishedUpload",
				"An upload of this app to {{devices}} was started and hasn't finished. Resume it in step 6, Copy & upload.",
				{ devices: waiting.map((target) => target.name).join(", ") },
			)}{" "}
			<DvButton variant="link" size="xs" onClick={() => goTo("copy_upload")}>
				{t("deploy.how.goToCopy", "Go to Copy & upload")}
			</DvButton>
		</Note>
	);
}

function LocalWebGate({ app }: Readonly<{ app: string }>) {
	const { t } = useTranslation("devices");
	return (
		<GateNotice
			kind="platform"
			title={t(
				"deploy.how.localWeb",
				"Offline copies are prepared by the desktop app.",
			)}
			text={t(
				"deploy.how.localWebText",
				"The browser can't read {{app}}'s files: it's a local-only app that exists only on the computer that created it. Open this deploy in the desktop app on that computer.",
				{ app },
			)}
		/>
	);
}

export function HowItRunsStep(props: Readonly<PlanStepProps>) {
	const { t } = useTranslation("devices");
	const { state, check } = props;
	const { app, mode, draft, facts } = state;
	const header = (lede?: string) => (
		<WizardStepHeader
			step={2}
			total={8}
			title={t("deploy.step.how", "How it runs")}
			lede={lede}
		/>
	);
	if (!app || !mode)
		return (
			<div className="flex min-w-0 flex-col gap-4">
				{header()}
				<StateView
					kind="notloaded"
					title={t("deploy.needApp", "Choose an app first")}
					text={t(
						"deploy.needAppText",
						"This step depends on the app you deploy. Go back to What.",
					)}
					actions={
						<DvButton size="sm" onClick={() => props.goTo("what")}>
							{t("deploy.goToWhat", "Go to What")}
						</DvButton>
					}
				/>
			</div>
		);
	const lede = `${appCopy(t).explainerSub(mode, app.name)}.`;
	if (check.issues.some((issue) => issue.code === "local_only_web"))
		return (
			<div className="flex min-w-0 flex-col gap-4">
				{header(lede)}
				<LocalWebGate app={app.name} />
				<ModeCard app={app.name} mode={mode} />
			</div>
		);
	const updating = draft.entry === "update";
	const prepares = draft.version !== "keep";
	const offlineNew = mode === "offline" && !updating;
	return (
		<div className="flex min-w-0 flex-col gap-4">
			{header(lede)}
			{mode === "offline" ? <UnfinishedUploads {...props} /> : null}
			<ModeCard app={app.name} mode={mode} />
			{updating ? <UpdateBlock {...props} /> : null}
			{prepares && updating ? <PreparingBlock {...props} /> : null}
			{prepares ? <ShipBlock {...props} /> : null}
			{prepares && !updating ? <PreparingBlock {...props} /> : null}
			{offlineNew ? (
				<Banner
					tone="info"
					icon={Database}
					title={t("deploy.how.dataOnDevice", "Data on the device")}
				>
					{t(
						"deploy.how.dataOnDeviceText",
						"Each new service starts with this copy of the data. Updates later replace the app, never the data on the device. To start over with fresh data, deploy a new service.",
					)}
				</Banner>
			) : null}
			{offlineNew && facts.platform === "desktop" ? (
				<ImportCopy {...props} />
			) : null}
		</div>
	);
}
