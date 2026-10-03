"use client";

import { Trans, useTranslation } from "@flow-like/locales";
import {
	CircleAlert,
	KeyRound,
	ShieldCheck,
	TriangleAlert,
} from "lucide-react";
import { type Ref, useState } from "react";
import type { DevicesT } from "../../primitives/area-context";
import { Field, SecretInput, SwitchField } from "../../primitives/form-fields";
import { WizardStepHeader } from "../../primitives/wizard";
import { useLocalSummary } from "../../workspace";
import { useSetup } from "../setup-context";
import { FieldNote, IconList, Mono } from "../setup-parts";
import {
	PASSWORD_MAX_BYTES,
	PASSWORD_MIN_BYTES,
	type PasswordIssue,
	type RepeatIssue,
	STEP_COUNT,
	passwordIssue,
	repeatIssue,
} from "../setup-state";

export function passwordIssueText(t: DevicesT, issue: PasswordIssue): string {
	switch (issue.code) {
		case "empty":
			return t(
				"devices:setup.password.error.empty",
				"Enter a device password.",
			);
		case "too_short":
			return t(
				"devices:setup.password.error.tooShort",
				"Use at least {{min, number}} bytes. This one has {{bytes, number}}.",
				{ min: PASSWORD_MIN_BYTES, bytes: issue.bytes },
			);
		default:
			return t(
				"devices:setup.password.error.tooLong",
				"Use at most {{max, number}} bytes. This one has {{bytes, number}}.",
				{ max: PASSWORD_MAX_BYTES, bytes: issue.bytes },
			);
	}
}

export const repeatIssueText = (t: DevicesT, issue: RepeatIssue): string =>
	issue === "empty"
		? t("devices:setup.password.error.repeatEmpty", "Type the password again.")
		: t(
				"devices:setup.password.error.mismatch",
				"The two passwords don't match.",
			);

/** IA §6.6: what a device password is, before it is typed. */
function PasswordFacts() {
	const { t } = useTranslation("devices");
	const strong = { 1: <b className="font-semibold text-foreground" /> };
	return (
		<IconList
			boxed
			rows={[
				{
					id: "one",
					icon: KeyRound,
					text: (
						<Trans
							t={t}
							i18nKey="setup.password.explain.one"
							defaults="<1>One password per device on this computer.</1> It isn't your account password."
							components={strong}
						/>
					),
				},
				{
					id: "forget",
					icon: CircleAlert,
					text: (
						<Trans
							t={t}
							i18nKey="setup.password.explain.forget"
							defaults="<1>If you forget it, nobody can recover it,</1> and you set the device up again."
							components={strong}
						/>
					),
				},
				{
					id: "local",
					icon: ShieldCheck,
					text: t(
						"setup.password.explain.local",
						"The password never leaves this computer and the hub never sees it.",
					),
				},
			]}
		/>
	);
}

/** The two fields; a message shows once its field was left with something in it, or after Continue. */
function PasswordFields() {
	const { t } = useTranslation("devices");
	const { draft, tried, secrets } = useSetup();
	const [touched, setTouched] = useState({ password: false, repeat: false });
	const { password, repeat } = secrets;
	const first = passwordIssue(password);
	const second = repeatIssue(password, repeat);
	const showFirst = first && (tried || (touched.password && !!password));
	const showSecond = second && (tried || (touched.repeat && !!repeat));
	return (
		<div className="grid gap-4 @[560px]/setup:grid-cols-2">
			<Field
				id="dv-setup-password"
				label={
					<Trans
						t={t}
						i18nKey="setup.password.label"
						defaults="Device password for <1/>"
						components={{ 1: <Mono>{draft.name}</Mono> }}
					/>
				}
				error={showFirst ? passwordIssueText(t, first) : undefined}
			>
				<SecretInput
					value={password}
					onValueChange={secrets.setPassword}
					minBytes={PASSWORD_MIN_BYTES}
					maxBytes={PASSWORD_MAX_BYTES}
					autoComplete="new-password"
					autoCapitalize="off"
					onBlur={() => setTouched((state) => ({ ...state, password: true }))}
				/>
			</Field>
			<Field
				id="dv-setup-repeat"
				label={t("setup.password.repeatLabel", "Repeat the password")}
				error={showSecond ? repeatIssueText(t, second) : undefined}
			>
				<SecretInput
					value={repeat}
					onValueChange={secrets.setRepeat}
					autoComplete="new-password"
					autoCapitalize="off"
					onBlur={() => setTouched((state) => ({ ...state, repeat: true }))}
				/>
			</Field>
		</div>
	);
}

/** The account backup switch: on by default, and what turning it off means. */
function BackupChoice() {
	const { t } = useTranslation("devices");
	const { draft, update } = useSetup();
	const local = useLocalSummary();
	const risky =
		local.platform !== "desktop" && local.persistence !== "persisted";
	return (
		<div className="flex flex-col gap-1.5 rounded-lg border border-border bg-card px-3.5 py-3">
			<SwitchField
				id="dv-setup-backup"
				checked={draft.backup}
				onCheckedChange={(backup) => update({ backup })}
			>
				{t(
					"setup.password.backup",
					"Save an encrypted key backup to my account",
				)}
			</SwitchField>
			<p className="text-xs text-muted-foreground @[560px]/setup:pl-11.5">
				<Trans
					t={t}
					i18nKey="setup.password.backupHint"
					defaults="The hub stores it encrypted. You still need the device password to open it. With it you can manage <1/> from another computer, or after losing this one."
					components={{ 1: <Mono>{draft.name}</Mono> }}
				/>
			</p>
			{draft.backup ? null : (
				<FieldNote tone="warning" icon={TriangleAlert}>
					{t(
						"setup.password.backupOff",
						"Without it, the key backup file you download in the Save step is the only copy of {{name}}'s keys outside this computer.",
						{ name: draft.name },
					)}
				</FieldNote>
			)}
			{draft.backup && risky ? (
				<FieldNote tone="warning" icon={TriangleAlert}>
					{t(
						"setup.password.backupKeep",
						"Keep this on: this browser may delete keys.",
					)}
				</FieldNote>
			) : null}
		</div>
	);
}

/** Step 3: the password that locks the new device's owner keys on this computer. */
export function PasswordStep({
	headingRef,
}: Readonly<{ headingRef?: Ref<HTMLHeadingElement> }>) {
	const { t } = useTranslation("devices");
	const { draft, leaveLink } = useSetup();
	return (
		<>
			<WizardStepHeader
				headingRef={headingRef}
				step={4}
				total={STEP_COUNT}
				title={t("setup.password.title", "Set a device password")}
				lede={
					<Trans
						t={t}
						i18nKey="setup.password.lede"
						defaults="It locks <1/>'s owner keys on this computer."
						components={{ 1: <Mono>{draft.name}</Mono> }}
					/>
				}
			/>
			<PasswordFacts />
			<PasswordFields />
			<p className="text-xs text-muted-foreground">
				<Trans
					t={t}
					i18nKey="setup.password.hint"
					defaults="Using the same password as your other devices lets you open them together with “Unlock several”. Letters like ä or é count as 2 bytes. <1>What if I forget it?</1>"
					components={{
						1: (
							<a
								{...leaveLink({ screen: "keys", guide: "forgot-password" })}
								className="underline decoration-border-strong underline-offset-2 hover:decoration-current"
							/>
						),
					}}
				/>
			</p>
			<BackupChoice />
		</>
	);
}
