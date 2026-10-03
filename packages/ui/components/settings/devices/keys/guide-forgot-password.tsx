"use client";

import { Trans, useTranslation } from "@flow-like/locales";
import {
	BookOpen,
	ChevronDown,
	ChevronUp,
	CircleCheck,
	CircleX,
	Laptop,
	type LucideIcon,
	Plus,
	Power,
	Rocket,
	RotateCcw,
	Share2,
	Trash2,
	User,
	UserPlus,
} from "lucide-react";
import type { ReactNode } from "react";
import {
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "../../../ui/select";
import { Block } from "../primitives/block";
import { DvButton } from "../primitives/dv-button";
import { Field } from "../primitives/form-fields";
import { FreshnessStamp } from "../primitives/freshness-stamp";
import { GatedAction } from "../primitives/gate-notice";
import { cx } from "../primitives/tone";
import { useRouteLink } from "../routing/use-devices-route";
import type { KeyFlowsApi } from "./key-flows";
import { type GuideStep, GuideSteps, Mono } from "./key-parts";
import type { KeyRow } from "./keys-model";
import { type KeysRead, stampOf } from "./use-keys-model";

const ANY = "any";
const SELECT_ITEM =
	"text-[13px]/[18px] focus:bg-row-hover focus:text-foreground";

function WorksList({
	title,
	ok,
	items,
}: Readonly<{ title: string; ok: boolean; items: readonly string[] }>) {
	const Icon = ok ? CircleCheck : CircleX;
	return (
		<div className="min-w-0 rounded-lg border border-hairline bg-surface-sunken px-3 py-2.5">
			<h3 className="text-label font-semibold uppercase tracking-[0.06em] text-muted-foreground">
				{title}
			</h3>
			<ul className="mt-1.5 flex list-none flex-col gap-1 p-0 text-ui">
				{items.map((item) => (
					<li
						key={item}
						className="grid grid-cols-[16px_minmax(0,1fr)] gap-1.5"
					>
						<Icon
							aria-hidden
							className={cx(
								"mt-0.5 size-3.5",
								ok ? "text-good" : "text-critical",
							)}
						/>
						<span>{item}</span>
					</li>
				))}
			</ul>
		</div>
	);
}

/** What the hub still answers without the password, and what stays closed. */
function StillWorks() {
	const { t } = useTranslation("devices");
	return (
		<div className="@container/works">
			<div className="grid grid-cols-2 gap-3 @max-[560px]/works:grid-cols-1">
				<WorksList
					ok
					title={t("keys.guide.forgot.works", "Still works without it")}
					items={[
						t("keys.guide.forgot.works1", "Check-ins and online status"),
						t("keys.guide.forgot.works2", "Certificate expiry dates"),
						t("keys.guide.forgot.works3", "Revoking the device"),
						t(
							"keys.guide.forgot.works4",
							"Cloud approvals and spending limits",
						),
					]}
				/>
				<WorksList
					ok={false}
					title={t("keys.guide.forgot.broken", "Doesn't work")}
					items={[
						t(
							"keys.guide.forgot.broken1",
							"Anything encrypted: services, logs, metrics and settings",
						),
						t("keys.guide.forgot.broken2", "Changing who has access"),
						t("keys.guide.forgot.broken3", "Backups made with that password"),
					]}
				/>
			</div>
		</div>
	);
}

function DevicePicker({
	candidates,
	chosen,
	onDevice,
}: Readonly<{
	candidates: readonly KeyRow[];
	chosen: KeyRow | undefined;
	onDevice(deviceId: string | undefined): void;
}>) {
	const { t } = useTranslation("devices");
	const pick = (value: string) => onDevice(value === ANY ? undefined : value);
	return (
		<div className="flex flex-wrap items-end gap-x-3 gap-y-2 pt-1">
			<Field
				id="keys-forgot-device"
				label={t("keys.guide.forgot.which", "Which device?")}
				className="w-full max-w-70"
			>
				<Select value={chosen ? chosen.deviceId : ANY} onValueChange={pick}>
					<SelectTrigger
						id="keys-forgot-device"
						className="h-8.5 w-full rounded-lg border-input bg-card text-[13px]/[18px] shadow-none"
					>
						<SelectValue />
					</SelectTrigger>
					<SelectContent className="border-border-strong bg-popover shadow-none backdrop-blur-none">
						<SelectItem value={ANY} className={SELECT_ITEM}>
							{t("keys.guide.forgot.any", "Any device")}
						</SelectItem>
						{candidates.map((row) => (
							<SelectItem
								key={row.deviceId}
								value={row.deviceId}
								className={SELECT_ITEM}
							>
								{row.relationship === "owner"
									? t("keys.guide.forgot.optionYours", "{{device}} · yours", {
											device: row.name,
										})
									: t(
											"keys.guide.forgot.optionShared",
											"{{device}} · shared with you",
											{ device: row.name },
										)}
							</SelectItem>
						))}
					</SelectContent>
				</Select>
			</Field>
			{chosen ? (
				<span className="pb-2 text-xs text-muted-foreground">
					{chosen.relationship === "owner"
						? t(
								"keys.guide.forgot.youOwn",
								"You own it, so you can set it up again.",
							)
						: t(
								"keys.guide.forgot.theyOwn",
								"Someone else owns it, so you ask again.",
							)}
				</span>
			) : null}
		</div>
	);
}

function Option({
	icon: Icon,
	title,
	steps,
}: Readonly<{
	icon: LucideIcon;
	title: ReactNode;
	steps: readonly GuideStep[];
}>) {
	return (
		<section className="flex min-w-0 flex-col gap-2.5 border-t border-hairline pt-3">
			<h3 className="flex items-start gap-2 text-sm font-semibold tracking-normal">
				<Icon
					aria-hidden
					className="mt-0.5 size-4 shrink-0 text-muted-foreground"
				/>
				<span className="min-w-0">{title}</span>
			</h3>
			<GuideSteps dense steps={steps} />
		</section>
	);
}

interface OptionProps {
	chosen: KeyRow | undefined;
	/** The chosen device's name, or "the device". */
	name: ReactNode;
}

/** Delete the keys that only the forgotten password opens, then restore with the new one. */
function RestoreActions({
	chosen,
	flows,
}: Readonly<{ chosen: KeyRow | undefined; flows: KeyFlowsApi }>) {
	const { t } = useTranslation("devices");
	const restore = () =>
		flows.open(
			chosen
				? { kind: "restore", deviceId: chosen.deviceId }
				: { kind: "restore" },
		);
	return (
		<>
			{chosen?.vault ? (
				<DvButton
					size="sm"
					variant="danger-ghost"
					icon={Trash2}
					onClick={() => flows.deleteKeys(chosen)}
				>
					{t("keys.guide.forgot.delete", "Delete keys for {{device}}…", {
						device: chosen.name,
					})}
				</DvButton>
			) : null}
			<DvButton size="sm" icon={RotateCcw} onClick={restore}>
				{t("keys.header.restore", "Restore keys…")}
			</DvButton>
		</>
	);
}

function OtherComputerOption({
	chosen,
	name,
	flows,
}: Readonly<OptionProps & { flows: KeyFlowsApi }>) {
	const { t } = useTranslation("devices");
	const steps: GuideStep[] = [
		{
			id: "open",
			title: t(
				"keys.guide.forgot.a1",
				"Open Flow-Like there, signed in to the same account and hub.",
			),
			hint: t(
				"keys.guide.forgot.a1Hint",
				"Each computer and profile keeps its own device password, so the one you use there may still work.",
			),
		},
		{
			id: "change",
			title: (
				<Trans
					t={t}
					i18nKey="keys.guide.forgot.a2"
					defaults="Unlock <1/> there, choose <2>Change device password…</2>, then <2>Update account backup</2>."
					components={{
						1: <span>{name}</span>,
						2: <b className="font-semibold" />,
					}}
				/>
			),
		},
		{
			id: "restore",
			title: chosen?.vault
				? t(
						"keys.guide.forgot.a3Held",
						"Here, delete the keys that open only with the forgotten password, then restore from your account backup with the new password.",
					)
				: t(
						"keys.guide.forgot.a3",
						"Here, restore from your account backup with the new password.",
					),
			hint: chosen
				? undefined
				: t(
						"keys.guide.forgot.a3Hint",
						"If this computer still holds the device's keys, delete them first. Restoring only offers devices without keys here.",
					),
			actions: <RestoreActions chosen={chosen} flows={flows} />,
		},
	];
	return (
		<Option
			icon={Laptop}
			title={t(
				"keys.guide.forgot.aTitle",
				"Another computer or profile may still have its keys",
			)}
			steps={steps}
		/>
	);
}

function RevokeAction({ chosen }: Readonly<Pick<OptionProps, "chosen">>) {
	const { t } = useTranslation("devices");
	const link = useRouteLink();
	if (!chosen)
		return (
			<GatedAction
				gate={{
					kind: "policy",
					reason: t(
						"keys.guide.forgot.chooseFirst",
						"Choose the device above first.",
					),
				}}
			>
				<DvButton size="sm" variant="danger-ghost" icon={Power}>
					{t("keys.guide.forgot.revokeAny", "Revoke…")}
				</DvButton>
			</GatedAction>
		);
	return (
		<DvButton size="sm" variant="danger-ghost" icon={Power} asChild>
			<a
				{...link({
					screen: "device",
					deviceId: chosen.deviceId,
					tab: "settings",
					action: "revoke",
				})}
			>
				{t("keys.guide.forgot.revoke", "Revoke {{device}}…", {
					device: chosen.name,
				})}
			</a>
		</DvButton>
	);
}

function OwnerOption({ chosen, name }: Readonly<OptionProps>) {
	const { t } = useTranslation("devices");
	const link = useRouteLink();
	const steps: GuideStep[] = [
		{
			id: "revoke",
			title: (
				<Trans
					t={t}
					i18nKey="keys.guide.forgot.b1"
					defaults="Revoke <1/>."
					components={{ 1: <span>{name}</span> }}
				/>
			),
			hint: t(
				"keys.guide.forgot.b1Hint",
				"Revoking happens at the hub, so it doesn't need your device password and works even if the device is offline. It's permanent.",
			),
			actions: <RevokeAction chosen={chosen} />,
		},
		{
			id: "setup",
			title: t(
				"keys.guide.forgot.b2",
				"Set the device up again, with a new device password.",
			),
			actions: (
				<DvButton size="sm" icon={Plus} asChild>
					<a {...link({ screen: "setup" })}>
						{t("keys.guide.forgot.setup", "Set up a device")}
					</a>
				</DvButton>
			),
		},
		{
			id: "deploy",
			title: t("keys.guide.forgot.b3", "Deploy your apps to it again."),
			hint: t(
				"keys.guide.forgot.b3Hint",
				"Deploy from the new device's page once it has checked in.",
			),
			actions: (
				<DvButton size="sm" icon={Rocket} asChild>
					<a {...link({ screen: "fleet", view: "devices" })}>
						{t("keys.guide.forgot.fleet", "Open your devices")}
					</a>
				</DvButton>
			),
		},
	];
	return (
		<Option
			icon={User}
			title={
				<Trans
					t={t}
					i18nKey="keys.guide.forgot.bTitle"
					defaults="You own <1/>: set it up again"
					components={{ 1: <span>{name}</span> }}
				/>
			}
			steps={steps}
		/>
	);
}

function SharedOption() {
	const { t } = useTranslation("devices");
	const link = useRouteLink();
	const steps: GuideStep[] = [
		{
			id: "request",
			title: t(
				"keys.guide.forgot.c1",
				"Request access again. The request creates new keys with a new password.",
			),
			actions: (
				<DvButton size="sm" icon={UserPlus} asChild>
					<a {...link({ screen: "access", tab: "shared", action: "request" })}>
						{t("keys.guide.forgot.request", "Request access")}
					</a>
				</DvButton>
			),
		},
		{
			id: "approve",
			title: t("keys.guide.forgot.c2", "The owner approves the new request."),
		},
		{
			id: "unlock",
			title: t("keys.guide.forgot.c3", "Unlock with the new password."),
		},
	];
	return (
		<Option
			icon={Share2}
			title={t("keys.guide.forgot.cTitle", "It was shared with you: ask again")}
			steps={steps}
		/>
	);
}

/** The three ways back in, narrowed to the ones that fit the chosen device. */
function WaysBack({
	chosen,
	flows,
}: Readonly<{ chosen: KeyRow | undefined; flows: KeyFlowsApi }>) {
	const { t } = useTranslation("devices");
	const name = chosen ? (
		<Mono>{chosen.name}</Mono>
	) : (
		t("keys.guide.forgot.theDevice", "the device")
	);
	const owner = chosen?.relationship === "owner";
	return (
		<div className="grid grid-cols-[repeat(auto-fit,minmax(280px,1fr))] gap-x-6 gap-y-3">
			<OtherComputerOption chosen={chosen} name={name} flows={flows} />
			{!chosen || owner ? <OwnerOption chosen={chosen} name={name} /> : null}
			{!chosen || !owner ? <SharedOption /> : null}
		</div>
	);
}

function GuideToggle({
	open,
	onToggle,
}: Readonly<{ open: boolean; onToggle(): void }>) {
	const { t } = useTranslation("devices");
	return (
		<DvButton
			size="sm"
			icon={open ? ChevronUp : ChevronDown}
			aria-expanded={open}
			aria-controls={open ? "guide-forgot-steps" : undefined}
			onClick={onToggle}
		>
			{open
				? t("keys.guide.hide", "Hide steps")
				: t("keys.guide.forgot.show", "Show what to do")}
		</DvButton>
	);
}

/**
 * SPEC §5.9 guide "Forgot a device password": what still works without it,
 * what doesn't, and the three ways back in, narrowed by the chosen device.
 */
export function GuideForgotPassword({
	read,
	open,
	onToggle,
	deviceId,
	onDevice,
	flows,
}: Readonly<{
	read: KeysRead;
	open: boolean;
	onToggle(): void;
	/** The device the guide is narrowed to; undefined = any device. */
	deviceId?: string;
	onDevice(deviceId: string | undefined): void;
	flows: KeyFlowsApi;
}>) {
	const { t } = useTranslation("devices");
	const candidates = read.model.rows.filter(
		(row) => row.relationship !== "cloud_approval",
	);
	const chosen = candidates.find((row) => row.deviceId === deviceId);
	return (
		<Block
			id="guide-forgot-password"
			icon={BookOpen}
			className="scroll-mt-16"
			title={t("keys.guide.forgot.title", "Forgot a device password")}
			stamp={<FreshnessStamp {...stampOf(read.devices.freshness)} />}
		>
			<p className="max-w-[72ch] text-ui text-ink-2">
				{t(
					"keys.guide.forgot.lead",
					"Nobody can recover a device password, not even the hub. Services on the device keep running meanwhile.",
				)}
			</p>
			{open ? (
				<div id="guide-forgot-steps" className="flex flex-col gap-2">
					<StillWorks />
					<DevicePicker
						candidates={candidates}
						chosen={chosen}
						onDevice={onDevice}
					/>
					<WaysBack chosen={chosen} flows={flows} />
					<div className="pt-1">
						<GuideToggle open onToggle={onToggle} />
					</div>
				</div>
			) : (
				<div className="flex flex-wrap items-center gap-x-3 gap-y-2">
					<span className="text-xs text-muted-foreground">
						{t(
							"keys.guide.forgot.summary",
							"What still works, and your three ways back in.",
						)}
					</span>
					<GuideToggle open={false} onToggle={onToggle} />
				</div>
			)}
		</Block>
	);
}
