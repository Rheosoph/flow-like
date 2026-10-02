"use client";

import { Trans, useTranslation } from "@flow-like/locales";
import {
	ArrowUpRight,
	Clock,
	Container,
	Download,
	Globe,
	KeyRound,
	PackageOpen,
	ShieldCheck,
	SlidersHorizontal,
	Terminal,
} from "lucide-react";
import type { ReactNode, Ref } from "react";
import { enumLabel } from "../../copy/enum-labels";
import { useAreaTime } from "../../primitives/area-context";
import { Block } from "../../primitives/block";
import { CommandBlock } from "../../primitives/command-block";
import { FreshnessStamp } from "../../primitives/freshness-stamp";
import { KeyValueList, KvRow } from "../../primitives/key-value-list";
import { Segmented } from "../../primitives/segmented";
import { StatusChip } from "../../primitives/status-chip";
import { WizardStepHeader } from "../../primitives/wizard";
import { useSetup } from "../setup-context";
import { IconList, Mono } from "../setup-parts";
import {
	STEP_COUNT,
	packageFile,
	packageFolder,
	packsAgent,
	safeName,
} from "../setup-state";
import { megabytes } from "./platform-step";

function GuideItem({
	n,
	title,
	note,
	children,
}: Readonly<{
	n: number;
	title: ReactNode;
	note?: ReactNode;
	children: ReactNode;
}>) {
	return (
		<li className="grid grid-cols-[26px_minmax(0,1fr)] gap-2.5">
			<span
				aria-hidden
				className="inline-flex size-6 items-center justify-center rounded-full border border-border-strong font-mono text-xs font-medium text-ink-2"
			>
				{n}
			</span>
			<div className="flex min-w-0 flex-col gap-1.5">
				<p className="text-sm font-semibold">
					{title}
					{note ? (
						<span className="font-normal text-muted-foreground"> · {note}</span>
					) : null}
				</p>
				{children}
			</div>
		</li>
	);
}

const hint = "max-w-[64ch] text-xs text-muted-foreground";

interface FileRow {
	id: string;
	files: readonly string[];
	text: ReactNode;
}

function hostOf(url: string | undefined): string {
	if (!url) return "";
	try {
		return new URL(url).host;
	} catch {
		return "";
	}
}

/** Step 6: what to run on the device, what the package holds, and what the device needs. */
export function StartStep({
	headingRef,
}: Readonly<{ headingRef?: Ref<HTMLHeadingElement> }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const { draft, update, option, release, host } = useSetup();
	const { name, mode } = draft;
	const file = packageFile(name);
	const folder = packageFolder(name);
	const choose = mode === "both" || mode === undefined;
	const shown = choose ? (draft.startMode ?? "binary") : mode;
	const mac = option?.mac === true;
	const deferred =
		!!option?.deferredDownload && mode !== "docker" && shown === "binary";
	const artifact = release?.manifest.artifacts.find(
		(item) => item.target === draft.target,
	);
	const releaseHost = hostOf(artifact?.url);
	const sequence = release?.manifest.sequence;
	const registry = release?.manifest.container?.image.split("/")[0];
	const device = <Mono>{name}</Mono>;

	const files: FileRow[] = [];
	if (mode !== "docker") {
		if (packsAgent(option, mode))
			files.push({
				id: "agent",
				files: ["flow-like-standalone"],
				text: t(
					"setup.start.files.agent",
					"The agent, checked against the signed release.",
				),
			});
		else if (option?.deferredDownload)
			files.push({
				id: "download",
				files: ["download-runtime.sh"],
				text: t(
					"setup.start.files.download",
					"Downloads the agent on first start and checks it.",
				),
			});
		files.push({
			id: "start",
			files: ["start.sh"],
			text: t(
				"setup.start.files.start",
				"Registers the device on first start, then runs the agent.",
			),
		});
	}
	if (mode !== "binary")
		files.push({
			id: "docker",
			files: ["compose.yaml", "start-docker.sh", "container-start.sh"],
			text: t(
				"setup.start.files.docker",
				"Runs the pinned image on 127.0.0.1:8080, restarting unless stopped.",
			),
		});
	files.push(
		{
			id: "onboarding",
			files: ["onboarding.json"],
			text: (
				<>
					<StatusChip tone="warning" icon={KeyRound} className="mr-1.5">
						{t("setup.start.files.secret", "One-time secret")}
					</StatusChip>
					{t(
						"setup.start.files.onboarding",
						"Proves the device is the one this package was made for. The agent deletes it after setup. Don't share the package until then.",
					)}
				</>
			),
		},
		{
			id: "agent-env",
			files: ["state/agent.env"],
			text: t(
				"setup.start.files.agentEnv",
				"Agent settings on the device: isolation and storage limits.",
			),
		},
		{
			id: "env",
			files: [".env"],
			text: t("setup.start.files.env", "Service address and port."),
		},
		{
			id: "release",
			files: ["release.jws", "release-trust.json", "platform.json"],
			text:
				sequence === undefined
					? t(
							"setup.start.files.releasePlain",
							"The signed release and the keys it must be signed with.",
						)
					: t(
							"setup.start.files.release",
							"The signed release and the keys it must be signed with. The device refuses releases older than #{{sequence, number}}.",
							{ sequence },
						),
		},
	);

	return (
		<>
			<WizardStepHeader
				headingRef={headingRef}
				step={7}
				total={STEP_COUNT}
				title={t("setup.start.title", "Start it on the device")}
				lede={
					<Trans
						t={t}
						i18nKey="setup.start.lede"
						defaults="Run these in a terminal on <1/>, in the folder where you copied the package."
						components={{ 1: device }}
					/>
				}
			/>
			<Block
				icon={Terminal}
				title={
					<Trans
						t={t}
						i18nKey="setup.start.on"
						defaults="On <1/>"
						components={{ 1: device }}
					/>
				}
				stamp={
					<FreshnessStamp
						source="device"
						age="current"
						text={t("setup.start.stamp", "you run these there")}
					/>
				}
			>
				{choose ? (
					<div className="flex flex-wrap items-center gap-2">
						<span className={hint}>
							{mode === "both"
								? t(
										"setup.start.both",
										"Your package has both. Show commands for",
									)
								: t(
										"setup.start.unknownMode",
										"This window doesn't know how the package was built. Show commands for",
									)}
						</span>
						<Segmented
							size="sm"
							label={t("setup.start.showFor", "Show commands for")}
							value={shown ?? "binary"}
							onChange={(startMode) => update({ startMode })}
							options={[
								{
									value: "binary",
									label: enumLabel(t, "packageMode", "binary"),
									icon: Terminal,
								},
								{
									value: "docker",
									label: enumLabel(t, "packageMode", "docker"),
									icon: Container,
								},
							]}
						/>
					</div>
				) : null}
				<ol className="m-0 flex list-none flex-col gap-3.5 p-0">
					<GuideItem
						n={1}
						title={t(
							"setup.start.copy.title",
							"Copy the package to the device",
						)}
					>
						<p className={hint}>
							{t(
								"setup.start.copy.hint",
								"Any way works: a USB stick, a file share or scp. For example:",
							)}
						</p>
						<CommandBlock
							command={`scp ${file} user@${safeName(name)}.local:~`}
							note={t(
								"setup.start.copy.note",
								"Replace user and the address with your device's.",
							)}
						/>
					</GuideItem>
					<GuideItem
						n={2}
						title={t(
							"setup.start.unpack.title",
							"Unpack it and open the folder",
						)}
					>
						<CommandBlock command={`unzip ${file} -d ${folder}`} />
						<CommandBlock command={`cd ${folder}`} />
					</GuideItem>
					<GuideItem
						n={3}
						title={t("setup.start.run.title", "Start the agent")}
					>
						{shown === "docker" ? (
							<CommandBlock
								command="sh start-docker.sh"
								note={t(
									"setup.start.run.docker",
									"Starts the agent in the background with Docker Compose. It registers the device on first start and restarts on its own unless you stop it.",
								)}
							/>
						) : (
							<CommandBlock
								command="sh start.sh"
								note={
									<>
										{t(
											"setup.start.run.binary",
											"Runs in the foreground: it registers the device on first start, then keeps running. Stop it with Ctrl C.",
										)}
										{deferred && option?.size
											? t(
													"setup.start.run.deferred",
													" On first start it downloads the agent ({{size}}) from {{host}} and checks it against the signed release.",
													{ size: megabytes(option.size), host: releaseHost },
												)
											: null}
									</>
								}
							/>
						)}
					</GuideItem>
					<GuideItem
						n={4}
						title={t("setup.start.boot.title", "Start at boot")}
						note={t("setup.start.boot.recommended", "recommended")}
					>
						{shown === "docker" ? (
							<p className={hint}>
								{t(
									"setup.start.boot.docker",
									"Nothing to do: Docker restarts the container after a reboot, as long as Docker itself starts at boot.",
								)}
							</p>
						) : (
							<CommandBlock
								command="./flow-like-standalone install-service"
								note={
									draft.target === undefined
										? t(
												"setup.start.boot.any",
												"Installs a system service so the agent starts after every reboot. Run it once the first start has finished.",
											)
										: mac
											? t(
													"setup.start.boot.mac",
													"Installs a system service so the agent starts after every reboot (launchd on a Mac). Run it once the first start has finished.",
												)
											: t(
													"setup.start.boot.linux",
													"Installs a system service so the agent starts after every reboot (systemd on Linux). Run it once the first start has finished.",
												)
								}
							/>
						)}
					</GuideItem>
				</ol>
			</Block>
			<Block
				flush
				icon={PackageOpen}
				title={t("setup.start.files.title", "What's in the package")}
				stamp={
					draft.created ? (
						<FreshnessStamp
							source="local"
							age="current"
							observedAt={draft.created.createdAt}
							text={t("setup.start.files.built", "built {{ago}}", {
								ago: time.ago(draft.created.createdAt),
							})}
						/>
					) : undefined
				}
			>
				<ul className="m-0 flex list-none flex-col p-0">
					{files.map((row) => (
						<li
							key={row.id}
							className="grid grid-cols-1 gap-x-4 gap-y-0.5 border-t border-hairline px-4 py-2.25 text-ui first:border-t-0 @[560px]/setup:grid-cols-[minmax(150px,220px)_minmax(0,1fr)]"
						>
							<span className="flex min-w-0 flex-col gap-0.5 font-mono text-[12.5px]/[18px] font-medium">
								{row.files.map((item, index) => (
									<span
										key={item}
										title={item}
										className={
											index ? "truncate font-normal text-ink-2" : "truncate"
										}
									>
										{item}
									</span>
								))}
							</span>
							<span className="min-w-0 text-ink-2">{row.text}</span>
						</li>
					))}
					<li className="grid grid-cols-1 gap-x-4 gap-y-0.5 border-t border-hairline bg-surface-sunken px-4 py-2.25 text-ui @[560px]/setup:grid-cols-[minmax(150px,220px)_minmax(0,1fr)]">
						<span className="font-medium text-muted-foreground">
							{t("setup.start.files.not", "Not in the package")}
						</span>
						<span className="min-w-0 text-ink-2">
							{t(
								"setup.start.files.notText",
								"Your device password and the owner keys. They stay on this computer.",
							)}
						</span>
					</li>
				</ul>
			</Block>
			<Block
				icon={SlidersHorizontal}
				title={t(
					"setup.start.defaults.title",
					"Defaults you can change on the device",
				)}
				stamp={
					<FreshnessStamp
						source="local"
						age="current"
						text={t("setup.start.defaults.stamp", "package defaults")}
					/>
				}
			>
				<KeyValueList>
					<KvRow label={t("setup.start.defaults.address", "Service address")}>
						<Trans
							t={t}
							i18nKey="setup.start.defaults.addressText"
							defaults="<1>127.0.0.1:8080</1> · only this device can reach its services"
							components={{ 1: <Mono /> }}
						/>
						<span className="block text-xs text-muted-foreground">
							<Trans
								t={t}
								i18nKey="setup.start.defaults.addressHint"
								defaults="Change <1>FLOW_LIKE_PUBLISH_HOST</1> and <1>FLOW_LIKE_SERVICE_PORT</1> in <1>.env</1>."
								components={{ 1: <Mono /> }}
							/>
						</span>
					</KvRow>
					<KvRow label={t("setup.start.defaults.isolation", "Isolation")}>
						{t(
							"setup.start.defaults.isolationText",
							"Compatible · services run with the agent's full access unless a service is deployed with a sandbox",
						)}
						{mac ? (
							<span className="block text-xs text-muted-foreground">
								{t(
									"setup.start.defaults.isolationMac",
									"Sandboxing needs Linux; on a Mac every service runs as the agent.",
								)}
							</span>
						) : (
							<>
								<span className="mb-1.5 block text-xs text-muted-foreground">
									<Trans
										t={t}
										i18nKey="setup.start.defaults.isolationHint"
										defaults="To require a sandbox for every service, set this line in <1>state/agent.env</1> before the first start. It needs Linux with cgroups v2; deploys then refuse to run a service as the agent."
										components={{ 1: <Mono /> }}
									/>
								</span>
								<CommandBlock command="FLOW_LIKE_DEVICE_ISOLATION_POLICY=required" />
							</>
						)}
					</KvRow>
					<KvRow label={t("setup.start.defaults.storage", "Storage")}>
						{t(
							"setup.start.defaults.storageText",
							"64 GiB and 262,144 files for the device · 16 GiB per app",
						)}
					</KvRow>
				</KeyValueList>
			</Block>
			<Block
				icon={Globe}
				title={t("setup.start.network.title", "Network and clock")}
				stamp={
					<FreshnessStamp
						source="local"
						age="current"
						text={t("setup.start.network.stamp", "requirements")}
					/>
				}
			>
				<IconList
					rows={[
						{
							id: "out",
							icon: ArrowUpRight,
							text: (
								<Trans
									t={t}
									i18nKey="setup.start.network.out"
									defaults="Outbound HTTPS (port 443) to <1/>."
									components={{ 1: <Mono>{host}</Mono> }}
								/>
							),
						},
						...(deferred && releaseHost
							? [
									{
										id: "release",
										icon: Download,
										text: (
											<Trans
												t={t}
												i18nKey="setup.start.network.release"
												defaults="The first start also downloads the agent from <1/>; it needs curl and shasum on the device."
												components={{ 1: <Mono>{releaseHost}</Mono> }}
											/>
										),
									},
								]
							: []),
						...(mode !== "binary" && registry
							? [
									{
										id: "image",
										icon: Container,
										text: (
											<Trans
												t={t}
												i18nKey="setup.start.network.image"
												defaults="Docker pulls the pinned image from <1/>."
												components={{ 1: <Mono>{registry}</Mono> }}
											/>
										),
									},
								]
							: []),
						{
							id: "in",
							icon: ShieldCheck,
							text: t(
								"setup.start.network.in",
								"No inbound ports. Live connections go through the hub or directly; both are end-to-end encrypted.",
							),
						},
						{
							id: "clock",
							icon: Clock,
							text: t(
								"setup.start.network.clock",
								"The device clock must be within a few minutes of the real time. The hub refuses device proofs outside their validity window.",
							),
						},
					]}
				/>
			</Block>
		</>
	);
}
