"use client";

import { useTranslation } from "@flow-like/locales";
import { CircleCheck, Info, TriangleAlert } from "lucide-react";
import {
	type ComponentProps,
	type ReactNode,
	useEffect,
	useId,
	useMemo,
	useState,
} from "react";
import {
	type ApprovalDraft,
	type ApprovalIssue,
	type ApprovalIssueCode,
	type SpendingDraft,
	checkApprovalDraft,
} from "../../../../lib/device-management/model/deploy-plan";
import type {
	CopyRef,
	GateReason,
	OnlineAccess,
} from "../../../../lib/device-management/model/types";
import {
	eurosToMicros,
	formatEuroMicros,
} from "../../../../lib/device-resources";
import { enumExplain } from "../copy/enum-labels";
import { gateCopy } from "../copy/gate-copy";
import { type DevicesT, useAreaTime } from "../primitives/area-context";
import {
	CheckField,
	DvInput,
	Field,
	InputWithUnit,
} from "../primitives/form-fields";
import { GateInline } from "../primitives/gate-notice";
import { Segmented } from "../primitives/segmented";
import { cx } from "../primitives/tone";
import { daysUntil } from "./cloud-model";
import { money } from "./cloud-parts";
import {
	useAppModels,
	useAppNames,
	useBillingEligibility,
	useModelNames,
} from "./use-cloud";

const DAY_S = 86_400;
const MAX_DAYS = 365;
const MAX_INSTANCES = 100;

type IssueCopy = (
	t: DevicesT,
	params: Record<string, string | number>,
) => string;

const ISSUE_COPY: Record<ApprovalIssueCode, IssueCopy> = {
	models_invalid: (t) =>
		t("devices:cloud.issue.modelsInvalid", "Pick up to 64 different models."),
	nothing_approved: (t) =>
		t(
			"devices:cloud.issue.nothingApproved",
			"Pick at least one model, or allow project files.",
		),
	files_need_app: (t) =>
		t(
			"devices:cloud.issue.filesNeedApp",
			"Only online apps have project files in the cloud.",
		),
	files_need_owner: (t, p) =>
		t(
			"devices:cloud.issue.filesNeedOwner",
			"Only the owner of {{app}} can allow access to its project files.",
			p,
		),
	files_need_consent: (t) =>
		t(
			"devices:cloud.issue.filesNeedConsent",
			"Tick the box to allow access to the project files.",
		),
	instances_range: (t) =>
		t("devices:cloud.issue.instancesRange", "Enter 1 to 100 instances."),
	instances_below_service: (t, p) =>
		t(
			"devices:cloud.issue.instancesBelowService",
			"Must be at least {{count, number}}, this service's instances.",
			p,
		),
	expiry_past: (t) =>
		t("devices:cloud.issue.expiryRange", "Enter 1 to 365 days."),
	expiry_too_far: (t) =>
		t("devices:cloud.issue.expiryRange", "Enter 1 to 365 days."),
	spending_required: (t) =>
		t(
			"devices:cloud.issue.spendingRequired",
			"An approval with models needs a spending limit.",
		),
	spending_without_models: (t) =>
		t(
			"devices:cloud.issue.spendingWithoutModels",
			"A spending limit only applies to an approval with models.",
		),
	spending_range: (t) =>
		t(
			"devices:cloud.issue.spendingRange",
			"Enter an amount above €0 and up to €1,000,000.",
		),
	spending_expiry: (t) =>
		t(
			"devices:cloud.issue.spendingExpiry",
			"The limit can't run longer than the approval.",
		),
	spending_consent: (t) =>
		t(
			"devices:cloud.issue.spendingConsent",
			"Tick the box to accept the charges.",
		),
};

/** The sentence of one `checkApprovalDraft` issue. */
export function approvalIssueText(
	t: DevicesT,
	issue: ApprovalIssue,
	app: string,
): string {
	return ISSUE_COPY[issue.code](t, { app, ...issue.params });
}

const whole = (text: string) =>
	/^\d{1,4}$/.test(text.trim()) ? Number(text.trim()) : 0;

const HINT_TONE = { good: "text-good", warning: "text-warning" } as const;

/** A one-line note inside a form: a fact, or what is still missing before it can be sent. */
export const FormHint = ({
	icon: Icon = Info,
	tone,
	children,
	...props
}: Readonly<
	{
		icon?: typeof Info;
		tone?: keyof typeof HINT_TONE;
		children: ReactNode;
	} & Pick<ComponentProps<"p">, "id">
>) => (
	<p
		data-form-hint=""
		className="flex items-start gap-1.5 text-xs text-muted-foreground"
		{...props}
	>
		<Icon
			aria-hidden
			className={`mt-px size-3.5 shrink-0 ${tone ? HINT_TONE[tone] : ""}`}
		/>
		<span className="min-w-0">{children}</span>
	</p>
);

export interface CloudApprovalFieldsProps {
	deviceId: string;
	/** The online app; null for a local-only app. */
	appId: string | null;
	value: ApprovalDraft;
	onChange(value: ApprovalDraft): void;
	/** Offline copies: models only, never project files. */
	modelOnly: boolean;
	disabledReason?: CopyRef<GateReason>;
	/** The service's instances; the approval must allow at least as many. */
	serviceMaxInstances?: number;
	/** The app whose models are offered when `appId` is null (a local-only app). */
	modelsAppId?: string;
	/** False when the viewer isn't the app's owner (project files need the owner). */
	isAppOwner?: boolean;
}

/** None / Read / Read & write, with the owner's consent; only the app's owner can allow files. */
function ProjectFilesField({
	id,
	app,
	value,
	onChange,
	disabled,
	online,
	notOwner,
}: Readonly<{
	id: string;
	app: string;
	value: ApprovalDraft;
	onChange(value: ApprovalDraft): void;
	disabled: boolean;
	/** Only online apps have project files in the cloud. */
	online: boolean;
	notOwner: boolean;
}>) {
	const { t } = useTranslation("devices");
	const label = t("cloud.fields.files", "Project files");
	const locked = disabled || !online || notOwner;
	const said = { app };
	const hints = {
		none: t("cloud.fields.filesNoneHint", "Model calls only."),
		read_only: t(
			"cloud.fields.filesReadHint",
			"Reads files in {{app}}'s cloud storage.",
			said,
		),
		read_write: t(
			"cloud.fields.filesWriteHint",
			"Reads and changes files in {{app}}'s cloud storage. Needed for write buffering. {{explain}}",
			{ app, explain: enumExplain(t, "onlineAccess", "read_write") ?? "" },
		),
	};
	const setFiles = (files: "none" | OnlineAccess) =>
		onChange({
			...value,
			files,
			ownerConsent: files === "none" ? false : value.ownerConsent,
		});
	return (
		<div data-files="" className="flex min-w-0 flex-col gap-1.5">
			<span className="text-[13px]/[18px] font-medium">{label}</span>
			<Segmented
				label={label}
				value={value.files}
				onChange={setFiles}
				wrap
				className="self-start"
				options={[
					{
						value: "none",
						label: t("cloud.fields.filesNone", "None"),
						disabled,
					},
					{
						value: "read_only",
						label: t("cloud.fields.filesRead", "Read"),
						disabled: locked,
					},
					{
						value: "read_write",
						label: t("cloud.fields.filesWrite", "Read & write"),
						disabled: locked,
					},
				]}
			/>
			<p className="text-xs text-muted-foreground">{hints[value.files]}</p>
			{notOwner ? (
				<GateInline
					kind="owner"
					className={cx(
						"max-w-none",
						value.files !== "none" && "text-critical",
					)}
				>
					{ISSUE_COPY.files_need_owner(t, said)}
				</GateInline>
			) : null}
			{value.files === "none" || notOwner ? null : (
				<CheckField
					id={`${id}-consent`}
					checked={value.ownerConsent}
					disabled={disabled}
					onCheckedChange={(ownerConsent) =>
						onChange({ ...value, ownerConsent })
					}
				>
					{value.files === "read_write"
						? t(
								"cloud.fields.ownerConsentWrite",
								"I own {{app}} and allow this service to read and change its files",
								said,
							)
						: t(
								"cloud.fields.ownerConsentRead",
								"I own {{app}} and allow this service to read its files",
								said,
							)}
				</CheckField>
			)}
			{value.files !== "none" && !online ? (
				<p className="text-xs text-critical">
					{ISSUE_COPY.files_need_app(t, said)}
				</p>
			) : null}
		</div>
	);
}

/** APP §3.10 item 2: what one service may reach in the cloud (files, models, instances, end). */
export function CloudApprovalFields({
	appId,
	value,
	onChange,
	modelOnly,
	disabledReason,
	serviceMaxInstances = 1,
	modelsAppId,
	isAppOwner,
}: Readonly<CloudApprovalFieldsProps>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const id = useId();
	const names = useAppNames();
	const sourceApp = appId ?? modelsAppId ?? null;
	const app =
		(sourceApp ? names(sourceApp) : undefined) ??
		t("cloud.fields.thisApp", "this app");
	const offered = useAppModels(sourceApp);
	const nameOf = useModelNames(value.models);
	const now = Math.floor(time.nowS);
	const disabled = !!disabledReason;

	const options = useMemo(() => {
		const known = new Set(offered.models.map((model) => model.id));
		const local = new Map(
			offered.localModels.map((model) => [model.id, model]),
		);
		return [
			...offered.models,
			...value.models
				.filter((model) => !known.has(model))
				.map(
					(model) =>
						local.get(model) ?? {
							id: model,
							name: nameOf(model),
							access: "unknown",
						},
				),
		];
	}, [offered.models, offered.localModels, value.models, nameOf]);
	const localModels = offered.localModels.filter(
		(model) => !value.models.includes(model.id),
	);

	const issues = checkApprovalDraft(value, {
		appId,
		now,
		serviceMaxInstances,
		...(isAppOwner === undefined ? {} : { isAppOwner }),
	});
	const issueOf = (field: ApprovalIssue["field"], soft: boolean) => {
		const found = issues.find(
			(issue) =>
				issue.field === field &&
				(issue.code === "nothing_approved" ||
					issue.code === "files_need_consent") === soft,
		);
		if (!found) return undefined;
		return modelOnly && found.code === "nothing_approved"
			? t("cloud.issue.nothingApprovedModel", "Pick at least one model.")
			: approvalIssueText(t, found, app);
	};
	// Someone who isn't the owner gets no consent box, so nothing asks them to tick one.
	const softHint =
		issueOf("models", true) ??
		(isAppOwner === false ? undefined : issueOf("files", true));
	const days = daysUntil(value.expiresAt, now);
	const validDays = value.expiresAt > now;

	const toggleModel = (model: string, on: boolean) =>
		onChange({
			...value,
			models: on
				? [...value.models.filter((entry) => entry !== model), model].sort()
				: value.models.filter((entry) => entry !== model),
		});

	return (
		<div
			data-approval-fields=""
			aria-disabled={disabled || undefined}
			className="flex min-w-0 flex-col gap-3"
		>
			{disabledReason ? (
				<GateInline kind="role" className="max-w-none">
					{
						gateCopy(
							t,
							{
								ok: false,
								gate: "G12",
								kind: "role",
								hide: false,
								copy: disabledReason,
							},
							time,
						).inline
					}
				</GateInline>
			) : null}
			{modelOnly ? null : (
				<ProjectFilesField
					id={id}
					app={app}
					value={value}
					onChange={onChange}
					disabled={disabled}
					online={!!appId}
					notOwner={isAppOwner === false}
				/>
			)}
			<fieldset
				data-models=""
				className="m-0 flex min-w-0 flex-col gap-1.5 border-0 p-0"
			>
				<legend className="mb-1.5 p-0 text-[13px]/[18px] font-medium">
					{t("cloud.fields.models", "Hosted model access")}
				</legend>
				<p className="text-xs text-muted-foreground">
					{t(
						"cloud.fields.modelsHint",
						"Optional cloud access for the app's declared model dependencies. Ticking a model requires a spending limit. Local execution stays preferred where the device supports it.",
					)}
				</p>
				{options.map((model) => (
					<div key={model.id} className="flex flex-col gap-1">
						<CheckField
							id={`${id}-model-${model.id}`}
							checked={value.models.includes(model.id)}
							disabled={disabled}
							onCheckedChange={(on) => toggleModel(model.id, on)}
						>
							{model.name}
						</CheckField>
						{model.access === "local_with_hosted_fallback" ? (
							<p className="pl-6 text-xs text-muted-foreground">
								{t(
									"cloud.fields.modelHostedFallback",
									"Runs on the device when supported. Tick to allow hosted fallback.",
								)}
							</p>
						) : model.access === "local" ? (
							<p className="pl-6 text-xs text-muted-foreground">
								{t(
									"cloud.fields.modelLocalApproved",
									"This model requires a compatible on-device runtime and needs no cloud approval. Untick to remove its existing approval.",
								)}
							</p>
						) : null}
					</div>
				))}
				{options.length ? null : (
					<p className="text-xs text-muted-foreground">
						{offered.loading
							? t("cloud.fields.modelsLoading", "Reading the app's models…")
							: offered.known
								? t(
										"cloud.fields.modelsNone",
										"No hosted models were found in {{app}}'s model dependencies.",
										{ app },
									)
								: t(
										"cloud.fields.modelsUnknown",
										"The models of {{app}} can't be read on this computer.",
										{ app },
									)}
					</p>
				)}
				{issueOf("models", false) ? (
					<p className="text-xs text-critical">{issueOf("models", false)}</p>
				) : null}
			</fieldset>
			{localModels.length ? (
				<div data-local-models="" className="flex flex-col gap-1.5">
					<span className="text-[13px]/[18px] font-medium">
						{t("cloud.fields.localModels", "On-device models")}
					</span>
					<p className="text-xs text-muted-foreground">
						{t(
							"cloud.fields.localModelsHint",
							"These models need a compatible runtime on the device. They need no cloud approval or spending limit.",
						)}
					</p>
					<ul className="list-disc pl-5 text-sm">
						{localModels.map((model) => (
							<li key={model.id}>{model.name}</li>
						))}
					</ul>
				</div>
			) : null}
			<div className="grid min-w-0 gap-3 @min-[520px]/devices:grid-cols-2">
				<Field
					id={`${id}-instances`}
					label={t("cloud.fields.instances", "Instances allowed")}
					error={issueOf("maxInstances", false)}
					hint={t(
						"cloud.fields.instancesHint",
						"At least {{count, number}}, this service's instances. A safe update's startup check doesn't count against it.",
						{ count: serviceMaxInstances },
					)}
				>
					<DvInput
						numeric
						inputMode="numeric"
						disabled={disabled}
						value={value.maxInstances || ""}
						min={serviceMaxInstances}
						max={MAX_INSTANCES}
						onChange={(event) =>
							onChange({ ...value, maxInstances: whole(event.target.value) })
						}
					/>
				</Field>
				<Field
					id={`${id}-days`}
					label={t("cloud.fields.endsAfter", "Ends after")}
					error={issueOf("expiresAt", false)}
					hint={
						validDays
							? t(
									"cloud.fields.endsUntil",
									"until {{date}} · at most 365 days",
									{ date: time.at(value.expiresAt) },
								)
							: t("cloud.fields.endsMax", "at most 365 days")
					}
				>
					<InputWithUnit
						unit={t("cloud.fields.days", "days")}
						numeric
						inputMode="numeric"
						disabled={disabled}
						value={validDays ? days : ""}
						min={1}
						max={MAX_DAYS}
						onChange={(event) =>
							onChange({
								...value,
								expiresAt: now + whole(event.target.value) * DAY_S,
							})
						}
					/>
				</Field>
			</div>
			{softHint ? <FormHint>{softHint}</FormHint> : null}
			{modelOnly ? (
				<FormHint>
					{t(
						"cloud.fields.localOnly",
						"Local-only apps can't get access to cloud files.",
					)}
				</FormHint>
			) : null}
		</div>
	);
}

export interface SpendingLimitFieldsProps {
	deviceId: string;
	value: SpendingDraft;
	approval: ApprovalDraft;
	onChange(value: SpendingDraft): void;
	/** An existing approval: the hub then says whether the viewer's plan covers its models (BG34). */
	grantId?: string;
	/** How many services get a limit like this one (the deploy wizard's devices); the consent then names all of them. */
	services?: number;
}

const amountText = (micros: number) =>
	formatEuroMicros(micros).replace("€", "");

function parseAmount(text: string): number | undefined {
	try {
		return eurosToMicros(text.trim().replace(",", "."));
	} catch {
		return undefined;
	}
}

/** What was typed in micro-euros; 0, which is never a valid limit, when it isn't an amount. */
const microsOf = (text: string) => parseAmount(text) ?? 0;

/** BG34: whether the payer's plan covers the models; an older hub (or no approval yet) can't say. */
function Eligibility({
	deviceId,
	grantId,
}: Readonly<{ deviceId: string; grantId?: string }>) {
	const { t } = useTranslation("devices");
	const read = useBillingEligibility(deviceId, grantId);
	const refused = useMemo(
		() =>
			(read.data?.models ?? [])
				.filter((model) => !model.allowed)
				.map((model) => model.model_id),
		[read.data],
	);
	const nameOf = useModelNames(refused);
	if (!read.data)
		return (
			<FormHint>
				{grantId && read.loading
					? t("cloud.spendFields.planChecking", "Checking your plan…")
					: t(
							"cloud.spendFields.planInterim",
							"Your plan's model prices apply. Whether your plan covers each model is checked when the service first calls it.",
						)}
			</FormHint>
		);
	const plan = read.data.plan ?? t("cloud.spendFields.planUnnamed", "current");
	if (read.data.eligible && !refused.length)
		return (
			<FormHint icon={CircleCheck} tone="good">
				{t(
					"cloud.spendFields.planCovers",
					"Your {{plan}} plan covers these models.",
					{ plan },
				)}
			</FormHint>
		);
	return (
		<FormHint icon={TriangleAlert} tone="warning">
			{refused.length
				? t(
						"cloud.spendFields.planRefuses",
						"Your {{plan}} plan doesn't cover {{models}}. Calls to them are refused even with a limit.",
						{ plan, models: refused.map(nameOf).join(", ") },
					)
				: t(
						"cloud.spendFields.planNoModels",
						"This approval has no models, so there's nothing to pay for.",
					)}
		</FormHint>
	);
}

/** APP §3.10 item 3: the amount the viewer pays at most for one service's model calls. */
export function SpendingLimitFields({
	deviceId,
	value,
	approval,
	onChange,
	grantId,
	services = 1,
}: Readonly<SpendingLimitFieldsProps>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const id = useId();
	const [text, setText] = useState(() => amountText(value.limitMicros));
	const valid = parseAmount(text) !== undefined;

	// The limit ends with the approval.
	useEffect(() => {
		if (value.expiresAt !== approval.expiresAt)
			onChange({ ...value, expiresAt: approval.expiresAt });
	}, [value, approval.expiresAt, onChange]);
	// A value set from outside (same on each device, reset) replaces what was typed.
	useEffect(() => {
		setText((current) =>
			microsOf(current) === value.limitMicros
				? current
				: amountText(value.limitMicros),
		);
	}, [value.limitMicros]);

	const typed = (next: string) => {
		setText(next);
		const micros = microsOf(next);
		if (micros !== value.limitMicros)
			onChange({ ...value, limitMicros: micros });
	};

	return (
		<div data-spending-fields="" className="flex min-w-0 flex-col gap-3">
			<Field
				id={`${id}-amount`}
				label={t("cloud.spendFields.amount", "Spending limit")}
				className="max-w-60"
				error={valid ? undefined : ISSUE_COPY.spending_range(t, {})}
			>
				<InputWithUnit
					unit="EUR"
					numeric
					inputMode="decimal"
					value={text}
					onChange={(event) => typed(event.target.value)}
				/>
			</Field>
			<p data-spend-terms="" className="text-xs text-muted-foreground">
				{valid
					? t(
							"cloud.spendFields.terms",
							"{{amount}} at most · paid by you · doesn't renew · ends with the approval ({{date}})",
							{
								amount: money(value.limitMicros),
								date: time.at(approval.expiresAt),
							},
						)
					: t(
							"cloud.spendFields.termsNoAmount",
							"Paid by you · doesn't renew · ends with the approval ({{date}})",
							{ date: time.at(approval.expiresAt) },
						)}
			</p>
			<CheckField
				id={`${id}-consent`}
				checked={value.consent}
				onCheckedChange={(consent) => onChange({ ...value, consent })}
			>
				{services > 1
					? t(
							"cloud.spendFields.consentMany",
							"I pay for model use by these services up to these limits.",
						)
					: t(
							"cloud.spendFields.consent",
							"I pay for model use by this service up to this limit.",
						)}
			</CheckField>
			<Eligibility deviceId={deviceId} {...(grantId ? { grantId } : {})} />
		</div>
	);
}
