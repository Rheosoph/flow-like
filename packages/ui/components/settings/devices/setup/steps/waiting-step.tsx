"use client";

import { Trans, useTranslation } from "@flow-like/locales";
import {
	ChevronDown,
	CircleCheck,
	CircleDashed,
	Hourglass,
	KeyRound,
	ListChecks,
	LoaderCircle,
	PackageX,
	Rocket,
	Stethoscope,
	UserPlus,
} from "lucide-react";
import type { ReactNode, Ref } from "react";
import { enumLabel } from "../../copy/enum-labels";
import { useAreaTime } from "../../primitives/area-context";
import { Block } from "../../primitives/block";
import { Checklist, type ChecklistItem } from "../../primitives/checklist";
import { CommandBlock } from "../../primitives/command-block";
import { FreshnessStamp } from "../../primitives/freshness-stamp";
import { InlineResult } from "../../primitives/inline-result";
import { PresenceGlyph } from "../../primitives/presence-glyph";
import { StateView } from "../../primitives/state-view";
import { StatusChip } from "../../primitives/status-chip";
import { WizardStepHeader } from "../../primitives/wizard";
import { hubErrorCopy } from "../../workspace";
import { useSetup } from "../setup-context";
import { IconList, Mono, PendingSetupsStamp } from "../setup-parts";
import { type CreatedSetup, STEP_COUNT, startBy } from "../setup-state";
import { WAIT_POLL_MS } from "../use-setup-wait";

const LINK =
	"underline decoration-border-strong underline-offset-2 hover:decoration-current";
const POLL_S = WAIT_POLL_MS / 1000;

function Troubleshoot() {
	const { t } = useTranslation("devices");
	const { host } = useSetup();
	const rows: { id: string; label: string; text: ReactNode }[] = [
		{
			id: "pending",
			label: enumLabel(t, "connectionStatus", "pending"),
			text: (
				<Trans
					t={t}
					i18nKey="setup.wait.trouble.pending"
					defaults="The package wasn't started in this folder. Run <1>sh start.sh</1> there."
					components={{ 1: <Mono /> }}
				/>
			),
		},
		{
			id: "enrolled",
			label: enumLabel(t, "connectionStatus", "enrolled"),
			text: t(
				"setup.wait.trouble.enrolled",
				"It registered but hasn't checked in yet. Give it a minute.",
			),
		},
		{
			id: "connected",
			label: enumLabel(t, "connectionStatus", "connected"),
			text: t(
				"setup.wait.trouble.connected",
				"All good. This page updates within {{count, number}} s.",
				{ count: POLL_S },
			),
		},
		{
			id: "disconnected",
			label: enumLabel(t, "connectionStatus", "disconnected"),
			text: (
				<Trans
					t={t}
					i18nKey="setup.wait.trouble.disconnected"
					defaults="Check the device's internet access and that outbound HTTPS to <1/> is allowed."
					components={{ 1: <Mono>{host}</Mono> }}
				/>
			),
		},
		{
			id: "denied",
			label: enumLabel(t, "connectionStatus", "access_denied"),
			text: (
				<Trans
					t={t}
					i18nKey="setup.wait.trouble.denied"
					defaults="The agent stopped trying. Run <1>./flow-like-standalone recover-enrollment</1>. Often the device clock is off."
					components={{ 1: <Mono /> }}
				/>
			),
		},
	];
	return (
		<details className="group rounded-lg border border-border bg-card">
			<summary className="flex cursor-pointer list-none items-center gap-2 px-4 py-2.5 text-sm font-semibold [&::-webkit-details-marker]:hidden">
				<Stethoscope aria-hidden className="size-4 text-muted-foreground" />
				{t("setup.wait.trouble.title", "It hasn't shown up")}
				<ChevronDown
					aria-hidden
					className="ml-auto size-4 text-muted-foreground transition-transform group-open:rotate-180"
				/>
			</summary>
			<div className="flex flex-col gap-3 border-t border-hairline px-4 pt-3 pb-4 text-ui">
				<p>
					{t(
						"setup.wait.trouble.run",
						"On the device, in the package folder, run:",
					)}
				</p>
				<CommandBlock
					command="./flow-like-standalone status"
					note={t(
						"setup.wait.trouble.runNote",
						"Shows the connection state, last contact and each service's last error.",
					)}
				/>
				<ul className="m-0 flex list-none flex-col p-0">
					{rows.map((row) => (
						<li
							key={row.id}
							className="grid grid-cols-1 gap-x-3 gap-y-0.5 border-t border-hairline py-1.75 first:border-t-0 @[560px]/setup:grid-cols-[150px_minmax(0,1fr)]"
						>
							<b className="font-semibold">{row.label}</b>
							<span className="min-w-0">{row.text}</span>
						</li>
					))}
				</ul>
			</div>
		</details>
	);
}

function Expired({ created }: Readonly<{ created: CreatedSetup }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const { draft } = useSetup();
	return (
		<Block
			icon={PackageX}
			title={
				<Trans
					t={t}
					i18nKey="setup.wait.expired.block"
					defaults="Package for <1/>"
					components={{ 1: <Mono>{draft.name}</Mono> }}
				/>
			}
			summary={
				<StatusChip tone="unknown" icon={CircleDashed}>
					{enumLabel(t, "enrollment", "expired")}
				</StatusChip>
			}
			stamp={<PendingSetupsStamp />}
		>
			<StateView
				kind="never"
				icon={CircleDashed}
				title={t("setup.wait.expired.title", "The package expired unused")}
				text={
					<Trans
						t={t}
						i18nKey="setup.wait.expired.text"
						defaults="The setup package for <1/> expired on {{when}} and no longer works. Nothing was installed."
						values={{ when: time.at(startBy(created)) }}
						components={{ 1: <Mono>{draft.name}</Mono> }}
					/>
				}
			/>
		</Block>
	);
}

/** Step 7: the hub is asked every 5 s until the device registered and checked in once. */
export function WaitingStep({
	headingRef,
}: Readonly<{ headingRef?: Ref<HTMLHeadingElement> }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const { draft, wait, expired, leaveLink } = useSetup();
	const created = draft.created;
	if (!created) return null;
	const registeredAt = draft.registeredAt;
	const checkedInAt = draft.checkedInAt;
	const done = checkedInAt !== undefined;
	const registered = registeredAt !== undefined;
	const name = <Mono>{draft.name}</Mono>;

	const header = (
		<WizardStepHeader
			headingRef={headingRef}
			step={8}
			total={STEP_COUNT}
			title={
				done
					? t("setup.wait.titleDone", "Set up")
					: t("setup.wait.title", "Waiting for the first check-in")
			}
			className="sr-only"
		/>
	);
	if (expired)
		return (
			<>
				{header}
				<Expired created={created} />
			</>
		);

	const items: ChecklistItem[] = [
		{
			id: "created",
			state: "pass",
			source: "local",
			label: (
				<b className="font-medium">
					{t("setup.wait.created", "Package created")}
				</b>
			),
			note: t("setup.wait.createdNote", "{{when}} · works until {{until}}", {
				when: time.at(created.createdAt),
				until: time.at(startBy(created)),
			}),
		},
		{
			id: "started",
			state: registered ? "pass" : "active",
			label: (
				<b className="font-medium">
					{registered
						? t("setup.wait.started", "Device started the package")
						: t("setup.wait.waiting", "Waiting for the device…")}
				</b>
			),
			note:
				registeredAt === undefined
					? t(
							"setup.wait.waitingNote",
							"The package expires {{left}}. The app checks the hub every {{count, number}} s.",
							{ left: time.ago(startBy(created)), count: POLL_S },
						)
					: t(
							"setup.wait.startedNote",
							"The agent used the one-time setup secret at {{time}} and deleted it.",
							{ time: time.clock(registeredAt) },
						),
		},
		{
			id: "registered",
			state: registered ? "pass" : "pending",
			label: (
				<b className="font-medium">
					{t("setup.wait.registered", "Registered")}
				</b>
			),
			note:
				registeredAt === undefined
					? t("setup.wait.registeredTodo", "The hub accepts the device's keys.")
					: t(
							"setup.wait.registeredNote",
							"The hub accepted the device's keys at {{time}}.",
							{ time: time.clock(registeredAt) },
						),
		},
		{
			id: "checkin",
			state: done ? "pass" : registered ? "active" : "pending",
			label: (
				<b className="font-medium">
					{t("setup.wait.checkin", "First check-in")}
				</b>
			),
			note:
				checkedInAt === undefined ? (
					t(
						"setup.wait.checkinTodo",
						"A check-in is a signed “I'm here” the device sends about once a minute.",
					)
				) : (
					<span className="inline-flex items-center gap-1.5">
						{t("setup.wait.checkinNote", "Checked in at {{time}}", {
							time: time.clock(checkedInAt),
						})}
						<PresenceGlyph kind="online" decorative />
						{enumLabel(t, "presence", "online")}
					</span>
				),
		},
	];

	const chip = done ? (
		<StatusChip tone="good" icon={CircleCheck}>
			{t("setup.wait.chip.done", "Checked in")}
		</StatusChip>
	) : registered ? (
		<StatusChip tone="info" icon={LoaderCircle} spin>
			{t("setup.wait.registered", "Registered")}
		</StatusChip>
	) : (
		<StatusChip tone="info" icon={Hourglass}>
			{t("setup.wait.chip.waiting", "Not seen yet")}
		</StatusChip>
	);
	const checkedAtS =
		wait.checkedAt === undefined
			? undefined
			: Math.floor(wait.checkedAt / 1000);
	const stamp =
		checkedInAt === undefined ? (
			<FreshnessStamp
				source="hub"
				age={checkedAtS === undefined ? "notloaded" : "current"}
				observedAt={checkedAtS}
				cadenceSec={POLL_S}
				error={
					wait.error
						? {
								dataFrom: checkedAtS,
								reason: hubErrorCopy(t, wait.error.code),
							}
						: undefined
				}
			/>
		) : (
			<FreshnessStamp
				source="hub"
				age="current"
				observedAt={checkedInAt}
				text={t("setup.wait.confirmed", "confirmed {{ago}}", {
					ago: time.ago(checkedInAt),
				})}
			/>
		);

	return (
		<>
			{header}
			<Block
				icon={done ? CircleCheck : Hourglass}
				title={
					done ? (
						<Trans
							t={t}
							i18nKey="setup.wait.blockDone"
							defaults="<1/> is online"
							components={{ 1: name }}
						/>
					) : (
						<Trans
							t={t}
							i18nKey="setup.wait.block"
							defaults="Waiting for <1/>"
							components={{ 1: name }}
						/>
					)
				}
				summary={chip}
				stamp={stamp}
				foot={
					done ? (
						t(
							"setup.wait.footDone",
							"A check-in doesn't prove a live connection works. Unlock the device to read its services.",
						)
					) : (
						<span>
							{t(
								"setup.wait.foot",
								"You can close this; the setup stays in Pending setups.",
							)}{" "}
							<a
								{...leaveLink({ screen: "fleet", view: "devices" })}
								className={LINK}
							>
								{t("setup.pending.title", "Pending setups")}
							</a>
						</span>
					)
				}
			>
				<Checklist
					items={items}
					label={t("setup.wait.title", "Waiting for the first check-in")}
				/>
				{registeredAt !== undefined && checkedInAt !== undefined ? (
					<InlineResult tone="good">
						<Trans
							t={t}
							i18nKey="setup.wait.result"
							defaults="<1/> is set up. It registered at {{registered}} and checked in at {{checked}}."
							values={{
								registered: time.clock(registeredAt),
								checked: time.clock(checkedInAt),
							}}
							components={{ 1: name }}
						/>
					</InlineResult>
				) : null}
			</Block>
			{done ? (
				<Block
					icon={ListChecks}
					title={t("setup.wait.next.title", "Next")}
					stamp={
						<FreshnessStamp
							source="local"
							age="current"
							text={t("setup.wait.next.stamp", "suggestions")}
						/>
					}
				>
					<IconList
						rows={[
							{
								id: "deploy",
								icon: Rocket,
								text: (
									<Trans
										t={t}
										i18nKey="setup.wait.next.deploy"
										defaults="Deploy an app to <1/> from its device page, or from the app's Devices settings."
										components={{ 1: name }}
									/>
								),
							},
							{
								id: "share",
								icon: UserPlus,
								text: (
									<Trans
										t={t}
										i18nKey="setup.wait.next.share"
										defaults="Share it with someone: <1>Add people…</1>"
										components={{
											1: (
												<a
													{...leaveLink({
														screen: "access",
														tab: "people",
														action: "add-people",
													})}
													className={LINK}
												/>
											),
										}}
									/>
								),
							},
							{
								id: "keys",
								icon: KeyRound,
								text: (
									<Trans
										t={t}
										i18nKey="setup.wait.next.keys"
										defaults="Check its key backup in <1>Keys & recovery</1>."
										components={{
											1: (
												<a
													{...leaveLink({ screen: "keys" })}
													className={LINK}
												/>
											),
										}}
									/>
								),
							},
						]}
					/>
				</Block>
			) : (
				<Troubleshoot />
			)}
		</>
	);
}
