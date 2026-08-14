"use client";

import { i18n as i18next, useTranslation } from "@flow-like/locales";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useEffect, useMemo, useState } from "react";
import { toast } from "sonner";
import type { IProfile } from "../../../../lib/schema/profile/profile";
import { useBackend } from "../../../../state/backend-state";
import {
	Button,
	Dialog,
	DialogBody,
	DialogContent,
	DialogDescription,
	DialogFooter,
	DialogHeader,
	DialogTitle,
	Input,
	Label,
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
	Switch,
} from "../../../ui";
import {
	ALERTS_QUERY_KEY,
	ALERT_COMPARATOR_OPTIONS,
	ALERT_METRIC_OPTIONS,
	ALERT_MODE_OPTIONS,
	ALERT_RULES_PATH,
	ALERT_SOURCE_OPTIONS,
	ALERT_WINDOW_OPTIONS,
	DEFAULT_ALERT_MIN_SAMPLES,
	DEFAULT_ALERT_SENSITIVITY,
	DEFAULT_ALERT_WINDOW_MINUTES,
	type ITelemetryAlertRule,
	type ITelemetryAlertRulePayload,
	MAX_ALERT_SAMPLES,
	MAX_ALERT_SENSITIVITY,
	MIN_ALERT_SAMPLES,
	MIN_ALERT_SENSITIVITY,
	alertChannelMeta,
	alertMetricMeta,
	alertRulePath,
	alertSourceHint,
	alertSourceLabel,
	alertSourceMismatch,
} from "./alerts-types";

const ANY_SOURCE = "any";

const EMAIL_CHANNEL = alertChannelMeta("email");
const PUSH_CHANNEL = alertChannelMeta("push");

interface AlertRuleForm {
	name: string;
	metric: string;
	mode: string;
	comparator: string;
	threshold: string;
	sensitivity: string;
	minSamples: string;
	windowMinutes: number;
	source: string;
	enabled: boolean;
	notifyEmail: boolean;
	notifyPush: boolean;
}

const EMPTY_FORM: AlertRuleForm = {
	name: "",
	metric: ALERT_METRIC_OPTIONS[0].value,
	mode: "threshold",
	comparator: "gt",
	threshold: "",
	sensitivity: String(DEFAULT_ALERT_SENSITIVITY),
	minSamples: String(DEFAULT_ALERT_MIN_SAMPLES),
	windowMinutes: DEFAULT_ALERT_WINDOW_MINUTES,
	source: ANY_SOURCE,
	enabled: true,
	notifyEmail: false,
	notifyPush: false,
};

function formFromRule(rule: ITelemetryAlertRule): AlertRuleForm {
	return {
		name: rule.name,
		metric: rule.metric,
		mode: rule.mode,
		comparator: rule.comparator,
		threshold:
			rule.threshold === null || rule.threshold === undefined
				? ""
				: String(rule.threshold),
		sensitivity: String(rule.sensitivity ?? DEFAULT_ALERT_SENSITIVITY),
		minSamples: String(rule.minSamples),
		windowMinutes: rule.windowMinutes,
		source: rule.source ?? ANY_SOURCE,
		enabled: rule.enabled,
		notifyEmail: rule.notifyEmail ?? false,
		notifyPush: rule.notifyPush ?? false,
	};
}

function sourceOf(form: AlertRuleForm): string | null {
	return form.source === ANY_SOURCE ? null : form.source;
}

/**
 * Client-side mirror of the server's rule validation. The metric/source pair is
 * checked here too — the server rejects a source that can never reach the
 * metric's table, so an impossible combination never round-trips into a 400.
 */
function validateForm(form: AlertRuleForm): string | null {
	if (!form.name.trim()) return i18next.t('giveTheRuleAName', 'Give the rule a name.');
	if (form.mode === "threshold") {
		const threshold = Number.parseFloat(form.threshold);
		if (!Number.isFinite(threshold)) {
			return i18next.t('thresholdModeNeedsANumericThreshold', 'Threshold mode needs a numeric threshold.');
		}
	} else {
		const sensitivity = Number.parseFloat(form.sensitivity);
		if (
			!Number.isFinite(sensitivity) ||
			sensitivity < MIN_ALERT_SENSITIVITY ||
			sensitivity > MAX_ALERT_SENSITIVITY
		) {
			return i18next.t('sensitivityMustBeBetweenMin_alert_sensitivityAndMax_alert_sensitivity', 'Sensitivity must be between {{MIN_ALERT_SENSITIVITY}} and {{MAX_ALERT_SENSITIVITY}}.', { MIN_ALERT_SENSITIVITY, MAX_ALERT_SENSITIVITY });
		}
		const minSamples = Number.parseInt(form.minSamples, 10);
		if (
			!Number.isFinite(minSamples) ||
			minSamples < MIN_ALERT_SAMPLES ||
			minSamples > MAX_ALERT_SAMPLES
		) {
			return i18next.t('baselineWindowsMustBeBetweenMin_alert_samplesAndMax_alert_samples', 'Baseline windows must be between {{MIN_ALERT_SAMPLES}} and {{MAX_ALERT_SAMPLES}}.', { MIN_ALERT_SAMPLES, MAX_ALERT_SAMPLES });
		}
	}
	return alertSourceMismatch(form.metric, sourceOf(form));
}

function payloadFromForm(form: AlertRuleForm): ITelemetryAlertRulePayload {
	const anomaly = form.mode === "anomaly";
	const threshold = Number.parseFloat(form.threshold);
	const sensitivity = Number.parseFloat(form.sensitivity);
	const minSamples = Number.parseInt(form.minSamples, 10);
	return {
		name: form.name.trim(),
		metric: form.metric,
		source: sourceOf(form),
		comparator: form.comparator,
		threshold: anomaly || !Number.isFinite(threshold) ? null : threshold,
		mode: form.mode,
		window_minutes: form.windowMinutes,
		sensitivity: anomaly && Number.isFinite(sensitivity) ? sensitivity : null,
		min_samples:
			anomaly && Number.isFinite(minSamples)
				? minSamples
				: DEFAULT_ALERT_MIN_SAMPLES,
		enabled: form.enabled,
		notify_email: form.notifyEmail,
		notify_push: form.notifyPush,
	};
}

function NumberField({
	id,
	label,
	value,
	onChange,
	step,
	min,
	max,
	placeholder,
	hint,
}: {
	readonly id: string;
	readonly label: string;
	readonly value: string;
	readonly onChange: (value: string) => void;
	readonly step: string;
	readonly min?: number;
	readonly max?: number;
	readonly placeholder?: string;
	readonly hint?: string;
}) {
	return (
		<div className="space-y-1.5">
			<Label htmlFor={id} className="text-xs">
				{label}
			</Label>
			<Input
				id={id}
				type="number"
				step={step}
				min={min}
				max={max}
				placeholder={placeholder}
				value={value}
				onChange={(e) => onChange(e.target.value)}
			/>
			{hint ? (
				<p className="text-[11px] text-muted-foreground">{hint}</p>
			) : null}
		</div>
	);
}

function ToggleRow({
	id,
	label,
	description,
	checked,
	onCheckedChange,
}: {
	readonly id: string;
	readonly label: string;
	readonly description: string;
	readonly checked: boolean;
	readonly onCheckedChange: (checked: boolean) => void;
}) {
	return (
		<div className="flex items-center justify-between gap-4 rounded-lg border p-3">
			<div className="space-y-0.5">
				<Label htmlFor={id} className="text-sm">
					{label}
				</Label>
				<p className="text-xs text-muted-foreground">{description}</p>
			</div>
			<Switch id={id} checked={checked} onCheckedChange={onCheckedChange} />
		</div>
	);
}

interface TelemetryAlertRuleDialogProps {
	open: boolean;
	onOpenChange: (open: boolean) => void;
	rule?: ITelemetryAlertRule | null;
	profile?: IProfile;
}

export function TelemetryAlertRuleDialog({
	open,
	onOpenChange,
	rule,
	profile,
}: Readonly<TelemetryAlertRuleDialogProps>) {
	const { t } = useTranslation("admin");
	const backend = useBackend();
	const queryClient = useQueryClient();
	const [form, setForm] = useState<AlertRuleForm>(EMPTY_FORM);

	useEffect(() => {
		if (!open) return;
		setForm(rule ? formFromRule(rule) : { ...EMPTY_FORM });
	}, [open, rule]);

	const metricMeta = useMemo(() => alertMetricMeta(form.metric), [form.metric]);
	const validationError = useMemo(() => validateForm(form), [form]);
	const isAnomaly = form.mode === "anomaly";

	const save = useMutation({
		mutationFn: async () => {
			if (!profile) throw new Error("Profile not loaded");
			const payload = payloadFromForm(form);
			return rule
				? backend.apiState.patch(profile, alertRulePath(rule.id), payload)
				: backend.apiState.post(profile, ALERT_RULES_PATH, payload);
		},
		onSuccess: async () => {
			await queryClient.invalidateQueries({ queryKey: ALERTS_QUERY_KEY });
			toast.success(rule ? t('alertRuleUpdated', 'Alert rule updated') : "Alert rule created");
			onOpenChange(false);
		},
		onError: (error: Error) =>
			toast.error(error.message ?? "Failed to save the alert rule"),
	});

	return (
		<Dialog open={open} onOpenChange={onOpenChange}>
			<DialogContent className="max-w-2xl">
				<DialogHeader>
					<DialogTitle>
						{rule ? t('editAlertRule', 'Edit alert rule') : t('newAlertRule', 'New alert rule')}
					</DialogTitle>
					<DialogDescription>
						{t('alertsAreEvaluatedOverAnonymousTelemetryAggregatesAndAlwaysLandInTheInappInboxDeliveryBelowReachesThePlatformOperatorsAsWellNoThirdpartyAlertingServiceIsInvolved', "Alerts are evaluated over anonymous telemetry aggregates and always land in the in-app inbox. Delivery below reaches the platform operators as well — no third-party alerting service is involved.")}
					</DialogDescription>
				</DialogHeader>

				<DialogBody className="max-h-[60vh] space-y-4 overflow-y-auto pr-1">
					<div className="space-y-1.5">
						<Label htmlFor="alert-rule-name" className="text-xs">
							Name
						</Label>
						<Input
							id="alert-rule-name"
							value={form.name}
							onChange={(e) =>
								setForm((prev) => ({ ...prev, name: e.target.value }))
							}
							placeholder={t('desktopErrorRateSpike', 'Desktop error rate spike')}
						/>
					</div>

					<div className="grid gap-3 sm:grid-cols-2">
						<div className="space-y-1.5">
							<Label className="text-xs">{t('metric', 'Metric')}</Label>
							<Select
								value={form.metric}
								onValueChange={(metric) =>
									setForm((prev) => ({ ...prev, metric }))
								}
							>
								<SelectTrigger>
									<SelectValue />
								</SelectTrigger>
								<SelectContent>
									{ALERT_METRIC_OPTIONS.map((option) => (
										<SelectItem key={option.value} value={option.value}>
											{option.label}
										</SelectItem>
									))}
								</SelectContent>
							</Select>
							<p className="text-[11px] text-muted-foreground">
								{metricMeta.hint}
							</p>
						</div>
						<div className="space-y-1.5">
							<Label className="text-xs">{t('mode', 'Mode')}</Label>
							<Select
								value={form.mode}
								onValueChange={(mode) => setForm((prev) => ({ ...prev, mode }))}
							>
								<SelectTrigger>
									<SelectValue />
								</SelectTrigger>
								<SelectContent>
									{ALERT_MODE_OPTIONS.map((option) => (
										<SelectItem key={option.value} value={option.value}>
											{option.label}
										</SelectItem>
									))}
								</SelectContent>
							</Select>
							<p className="text-[11px] text-muted-foreground">
								{ALERT_MODE_OPTIONS.find((o) => o.value === form.mode)?.hint}
							</p>
						</div>
					</div>

					<div className="grid gap-3 sm:grid-cols-2">
						<div className="space-y-1.5">
							<Label className="text-xs">{t('comparator', 'Comparator')}</Label>
							<Select
								value={form.comparator}
								onValueChange={(comparator) =>
									setForm((prev) => ({ ...prev, comparator }))
								}
							>
								<SelectTrigger>
									<SelectValue />
								</SelectTrigger>
								<SelectContent>
									{ALERT_COMPARATOR_OPTIONS.map((option) => (
										<SelectItem key={option.value} value={option.value}>
											{option.label}
										</SelectItem>
									))}
								</SelectContent>
							</Select>
						</div>
						<div className="space-y-1.5">
							<Label className="text-xs">{t('window', 'Window')}</Label>
							<Select
								value={String(form.windowMinutes)}
								onValueChange={(value) =>
									setForm((prev) => ({
										...prev,
										windowMinutes: Number.parseInt(value, 10),
									}))
								}
							>
								<SelectTrigger>
									<SelectValue />
								</SelectTrigger>
								<SelectContent>
									{ALERT_WINDOW_OPTIONS.map((option) => (
										<SelectItem key={option.value} value={String(option.value)}>
											{option.label}
										</SelectItem>
									))}
								</SelectContent>
							</Select>
						</div>
					</div>

					{isAnomaly ? (
						<div className="grid gap-3 sm:grid-cols-2">
							<NumberField
								id="alert-rule-sensitivity"
								label={t('sensitivity', 'Sensitivity (σ)')}
								value={form.sensitivity}
								onChange={(sensitivity) =>
									setForm((prev) => ({ ...prev, sensitivity }))
								}
								step="0.1"
								min={MIN_ALERT_SENSITIVITY}
								max={MAX_ALERT_SENSITIVITY}
								hint="Standard deviations away from the baseline mean before the rule fires."
							/>
							<NumberField
								id="alert-rule-min-samples"
								label={t('baselineWindows', 'Baseline windows')}
								value={form.minSamples}
								onChange={(minSamples) =>
									setForm((prev) => ({ ...prev, minSamples }))
								}
								step="1"
								min={MIN_ALERT_SAMPLES}
								max={MAX_ALERT_SAMPLES}
								hint="Consecutive previous windows required before an anomaly can fire."
							/>
						</div>
					) : (
						<NumberField
							id="alert-rule-threshold"
							label="Threshold"
							value={form.threshold}
							onChange={(threshold) =>
								setForm((prev) => ({ ...prev, threshold }))
							}
							step="any"
							placeholder={metricMeta.unit === "ratio" ? "0.05" : "500"}
							hint={metricMeta.hint}
						/>
					)}

					<div className="space-y-1.5">
						<Label className="text-xs">{t('source', 'Source')}</Label>
						<Select
							value={form.source}
							onValueChange={(source) =>
								setForm((prev) => ({ ...prev, source }))
							}
						>
							<SelectTrigger>
								<SelectValue />
							</SelectTrigger>
							<SelectContent>
								<SelectItem value={ANY_SOURCE}>{t('allSources', 'All sources')}</SelectItem>
								{ALERT_SOURCE_OPTIONS.map((source) => (
									<SelectItem key={source} value={source}>
										{alertSourceLabel(source)}
									</SelectItem>
								))}
							</SelectContent>
						</Select>
						{sourceOf(form) ? (
							<p className="text-[11px] text-muted-foreground">
								{alertSourceHint(form.source)}
							</p>
						) : null}
					</div>

					<ToggleRow
						id="alert-rule-enabled"
						label="Enabled"
						description={t('disabledRulesStayConfiguredButAreSkippedByTheEvaluator', 'Disabled rules stay configured but are skipped by the evaluator.')}
						checked={form.enabled}
						onCheckedChange={(enabled) =>
							setForm((prev) => ({ ...prev, enabled }))
						}
					/>

					<div className="space-y-2">
						<div className="space-y-0.5">
							<Label className="text-xs">{t('delivery', 'Delivery')}</Label>
							<p className="text-[11px] text-muted-foreground">
								{t('everyAlertIsWrittenToTheInappInboxTheseChannelsSendItOutOfBandAsWellOnBothTheFiringAndTheRecoveryTransition', "Every alert is written to the in-app inbox. These channels send it out of band as well, on both the firing and the recovery transition.")}
							</p>
						</div>
						<ToggleRow
							id="alert-rule-notify-email"
							label={EMAIL_CHANNEL.title}
							description={EMAIL_CHANNEL.description}
							checked={form.notifyEmail}
							onCheckedChange={(notifyEmail) =>
								setForm((prev) => ({ ...prev, notifyEmail }))
							}
						/>
						{form.notifyEmail ? (
							<p className="text-[11px] text-muted-foreground">
								{t('nothingIsSentWhenThisPlatformHasNoAlertingMailboxOrMailProviderConfiguredTheAlertStillLandsInTheInbox', "Nothing is sent when this platform has no alerting mailbox or mail provider configured — the alert still lands in the inbox.")}
							</p>
						) : null}
						<ToggleRow
							id="alert-rule-notify-push"
							label={PUSH_CHANNEL.title}
							description={PUSH_CHANNEL.description}
							checked={form.notifyPush}
							onCheckedChange={(notifyPush) =>
								setForm((prev) => ({ ...prev, notifyPush }))
							}
						/>
					</div>

					{validationError ? (
						<p className="text-xs text-destructive">{validationError}</p>
					) : null}
				</DialogBody>

				<DialogFooter>
					<Button
						variant="outline"
						onClick={() => onOpenChange(false)}
						disabled={save.isPending}
					>
						{t('cancel', 'Cancel')}
					</Button>
					<Button
						onClick={() => save.mutate()}
						disabled={save.isPending || !profile || Boolean(validationError)}
					>
						{save.isPending ? "Saving…" : rule ? t('saveChanges', 'Save changes') : t('createRule', 'Create rule')}
					</Button>
				</DialogFooter>
			</DialogContent>
		</Dialog>
	);
}
