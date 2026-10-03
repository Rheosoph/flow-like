"use client";

import { useTranslation } from "@flow-like/locales";
import { useId, useState } from "react";
import {
	type ApprovalDraft,
	type SpendingDraft,
	checkApprovalDraft,
} from "../../../../lib/device-management/model/deploy-plan";
import { useAreaTime } from "../primitives/area-context";
import { ConsequencePreview } from "../primitives/consequence-preview";
import { DvButton } from "../primitives/dv-button";
import {
	CloudApprovalFields,
	FormHint,
	SpendingLimitFields,
	approvalIssueText,
} from "./approval-fields";
import type { CloudApproval, CloudLimit } from "./cloud-model";
import { money, useMoney } from "./cloud-parts";
import { useModelNames } from "./use-cloud";

const DAY_S = 86_400;
const DEFAULT_APPROVAL_DAYS = 30;
const DEFAULT_LIMIT_MICROS = 10_000_000;

export interface ApprovalFormSubject {
	deviceId: string;
	deviceName: string;
	serviceId: string;
	/** The online app; null for a local-only app. */
	appId: string | null;
	/** The app as the viewer knows it (for a local-only app: its id on this computer). */
	projectId: string;
	appName: string;
	modelOnly: boolean;
	serviceMaxInstances: number;
	isAppOwner?: boolean;
	/** The device is offline: naming the approval in the settings has to wait. */
	offline: boolean;
}

/*
 * These three leave their return type to inference: Codacy's Lizard merges a
 * function declaration that states one with the function after it.
 */

/** Only the app's owner can allow project files, so nobody else starts with them. */
function firstFiles(subject: ApprovalFormSubject, current?: CloudApproval) {
	const none: ApprovalDraft["files"] = "none";
	if (subject.modelOnly) return none;
	if (current) return current.files;
	if (subject.appId && subject.isAppOwner !== false) return "read_only";
	return none;
}

function firstDraft(
	subject: ApprovalFormSubject,
	current: CloudApproval | undefined,
	now: number,
) {
	const details = current?.details;
	const draft: ApprovalDraft = {
		files: firstFiles(subject, current),
		ownerConsent: false,
		models: [...(details?.models ?? [])].sort(),
		maxInstances: Math.max(
			details?.maxInstances ?? 1,
			subject.serviceMaxInstances,
			1,
		),
		expiresAt: now + DEFAULT_APPROVAL_DAYS * DAY_S,
	};
	return draft;
}

/** An existing approval as a draft, so its limit is checked by the rules of a new one. */
function draftOf(approval: CloudApproval) {
	const details = approval.details;
	const draft: ApprovalDraft = {
		files: approval.files,
		ownerConsent: true,
		models: details?.models ?? [],
		maxInstances: details?.maxInstances ?? 1,
		expiresAt: approval.expiresAt,
	};
	return draft;
}

/** SPEC §5.3: approve (or replace) one service's cloud access inline, with what it will cause. */
export function ApprovalForm({
	subject,
	current,
	busy,
	onSubmit,
	onCancel,
}: Readonly<{
	subject: ApprovalFormSubject;
	/** The approval being replaced. */
	current?: CloudApproval | undefined;
	busy: boolean;
	onSubmit(draft: ApprovalDraft, spending: SpendingDraft | null): void;
	onCancel(): void;
}>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const hintId = useId();
	const now = Math.floor(time.nowS);
	const { serviceId, deviceName, appName, modelOnly } = subject;
	const [draft, setDraft] = useState(() => firstDraft(subject, current, now));
	const [spending, setSpending] = useState<SpendingDraft>(() => ({
		limitMicros: DEFAULT_LIMIT_MICROS,
		expiresAt: draft.expiresAt,
		consent: false,
	}));
	const nameOf = useModelNames(draft.models);
	const replace = !!current?.revocable;
	const withModels = draft.models.length > 0;
	const issues = checkApprovalDraft(draft, {
		appId: subject.appId,
		now,
		serviceMaxInstances: subject.serviceMaxInstances,
		...(subject.isAppOwner === undefined
			? {}
			: { isAppOwner: subject.isAppOwner }),
		spending: withModels ? spending : undefined,
	});
	const spendingIssue = issues.find((issue) => issue.field === "spending");
	const title = modelOnly
		? replace
			? t("cloud.form.replaceModel", "Replace model access")
			: t("cloud.form.approveModel", "Approve model access")
		: replace
			? t("cloud.form.replaceApproval", "Replace approval")
			: t("cloud.form.approve", "Approve cloud access");
	const said = {
		service: serviceId,
		app: appName,
		models: draft.models.map(nameOf).join(", "),
	};
	const what = withModels
		? draft.files === "read_write"
			? t(
					"cloud.form.whatModelsWrite",
					"{{service}} may call {{models}}, and read and change {{app}}'s project files.",
					said,
				)
			: draft.files === "read_only"
				? t(
						"cloud.form.whatModelsRead",
						"{{service}} may call {{models}} and read {{app}}'s project files.",
						said,
					)
				: t("cloud.form.whatModels", "{{service}} may call {{models}}.", said)
		: draft.files === "read_write"
			? t(
					"cloud.form.whatWrite",
					"{{service}} may read and change {{app}}'s project files. It may call no models.",
					said,
				)
			: draft.files === "read_only"
				? t(
						"cloud.form.whatRead",
						"{{service}} may read {{app}}'s project files. It may call no models.",
						said,
					)
				: modelOnly
					? t("cloud.form.whatNothingModel", "Nothing yet: pick a model.")
					: t(
							"cloud.form.whatNothing",
							"Nothing yet: pick a model or project files.",
						);
	return (
		<fieldset
			data-approval-form=""
			aria-label={title}
			className="m-0 flex min-w-0 flex-col gap-3 rounded-lg border border-border bg-surface-sunken p-3"
		>
			<p className="text-xs text-muted-foreground">
				{replace
					? t(
							"cloud.form.replaceIntro",
							"The new approval replaces the current one. Buffered changes keep the old one until you move them.",
						)
					: t(
							"cloud.form.intro",
							"Approved by you, for {{service}} on {{device}}. Its IDs come from this service; no file is needed.",
							{ service: serviceId, device: deviceName },
						)}
			</p>
			<CloudApprovalFields
				deviceId={subject.deviceId}
				appId={subject.appId}
				value={draft}
				onChange={setDraft}
				modelOnly={modelOnly}
				serviceMaxInstances={subject.serviceMaxInstances}
				modelsAppId={subject.projectId}
				{...(subject.isAppOwner === undefined
					? {}
					: { isAppOwner: subject.isAppOwner })}
			/>
			{withModels ? (
				<div
					data-form-spending=""
					className="flex min-w-0 flex-col gap-2 border-t border-hairline pt-3"
				>
					<p className="text-xs text-muted-foreground">
						{t(
							"cloud.form.spendingWhy",
							"An approval with models needs a spending limit. The service can call the models only up to it.",
						)}
					</p>
					<SpendingLimitFields
						deviceId={subject.deviceId}
						value={spending}
						approval={draft}
						onChange={setSpending}
					/>
				</div>
			) : null}
			<ConsequencePreview
				compact
				rows={{
					what,
					who: withModels
						? t(
								"cloud.form.who",
								"You pay for its model calls, up to {{amount}}.",
								{ amount: money(spending.limitMicros) },
							)
						: t("cloud.form.whoNothing", "Nothing is charged: no models."),
					when: subject.offline
						? t(
								"cloud.form.whenOffline",
								"Once the service's settings name this approval; that waits until {{device}} is back online.",
								{ device: deviceName },
							)
						: t(
								"cloud.form.when",
								"Once the service's settings name this approval.",
							),
					undo: {
						reversible: true,
						text: t("cloud.form.undo", "Revoke the approval at any time."),
					},
				}}
			/>
			<div className="flex flex-wrap items-center gap-2">
				<DvButton
					variant="primary"
					busy={busy}
					data-act="approve-go"
					aria-disabled={issues.length > 0 || undefined}
					aria-describedby={spendingIssue ? hintId : undefined}
					onClick={() => onSubmit(draft, withModels ? spending : null)}
				>
					{title}
				</DvButton>
				<DvButton data-act="approve-cancel" onClick={onCancel}>
					{t("cloud.form.cancel", "Cancel")}
				</DvButton>
				{spendingIssue ? (
					<FormHint id={hintId}>
						{approvalIssueText(t, spendingIssue, appName)}
					</FormHint>
				) : null}
			</div>
		</fieldset>
	);
}

/** SPEC §5.3: add or replace the spending limit of an existing approval. */
export function LimitForm({
	deviceId,
	approval,
	replace,
	busy,
	onSubmit,
	onCancel,
}: Readonly<{
	deviceId: string;
	approval: CloudApproval;
	/** The limit that closes when the new one starts. */
	replace?: CloudLimit | undefined;
	busy: boolean;
	onSubmit(spending: SpendingDraft): void;
	onCancel(): void;
}>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const amount = useMoney();
	const hintId = useId();
	const now = Math.floor(time.nowS);
	const [spending, setSpending] = useState<SpendingDraft>(() => ({
		limitMicros: replace?.limit ?? DEFAULT_LIMIT_MICROS,
		expiresAt: approval.expiresAt,
		consent: false,
	}));
	const draft = draftOf(approval);
	const issue = checkApprovalDraft(draft, {
		appId: approval.appId,
		now,
		serviceMaxInstances: 1,
		spending,
	}).find((entry) => entry.field === "spending");
	const title = replace
		? t("cloud.limitForm.replaceLimit", "Replace spending limit")
		: t("cloud.limitForm.add", "Add spending limit");
	return (
		<fieldset
			data-limit-form=""
			aria-label={title}
			className="m-0 flex min-w-0 flex-col gap-3 rounded-lg border border-border bg-surface-sunken p-3"
		>
			{replace ? (
				<p className="text-xs text-muted-foreground">
					{t(
						"cloud.limitForm.replaceIntro",
						"The current limit ({{used}} of {{limit}} used) closes; the new one starts from zero used.",
						{ used: amount(replace.used), limit: amount(replace.limit) },
					)}
				</p>
			) : null}
			<SpendingLimitFields
				deviceId={deviceId}
				value={spending}
				approval={draft}
				onChange={setSpending}
				grantId={approval.grantId}
			/>
			<div className="flex flex-wrap items-center gap-2">
				<DvButton
					variant="primary"
					busy={busy}
					data-act="limit-go"
					aria-disabled={!!issue || undefined}
					aria-describedby={issue ? hintId : undefined}
					onClick={() => onSubmit(spending)}
				>
					{t("cloud.limitForm.review", "Review…")}
				</DvButton>
				<DvButton data-act="limit-cancel" onClick={onCancel}>
					{t("cloud.form.cancel", "Cancel")}
				</DvButton>
				{issue ? (
					<FormHint id={hintId}>{approvalIssueText(t, issue, "")}</FormHint>
				) : null}
			</div>
		</fieldset>
	);
}
