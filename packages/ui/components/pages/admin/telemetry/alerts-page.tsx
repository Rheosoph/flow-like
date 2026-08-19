"use client";

import { Trans, i18n as i18next, useTranslation } from "@flow-like/locales";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
	ArrowLeft,
	BellRing,
	Gauge,
	Lock,
	Mail,
	Pencil,
	Play,
	Plus,
	RefreshCw,
	Trash2,
	TriangleAlert,
} from "lucide-react";
import Link from "next/link";
import { useCallback, useMemo, useState } from "react";
import { useAuth } from "react-oidc-context";
import { toast } from "sonner";
import { useInvoke } from "../../../../hooks/use-invoke";
import { GlobalPermission } from "../../../../lib/permission/global-permission";
import { useBackend } from "../../../../state/backend-state";
import {
	AlertDialog,
	AlertDialogAction,
	AlertDialogCancel,
	AlertDialogContent,
	AlertDialogDescription,
	AlertDialogFooter,
	AlertDialogHeader,
	AlertDialogTitle,
	Badge,
	Button,
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
	RelativeTime,
	Skeleton,
	Switch,
} from "../../../ui";
import { TelemetryAlertRuleDialog } from "./alert-rule-dialog";
import { TelemetryAlertsInbox } from "./alerts-inbox";
import {
	ALERTS_QUERY_KEY,
	ALERT_EVALUATE_PATH,
	ALERT_RULES_PATH,
	type AlertChannelMeta,
	type ITelemetryAlertEvaluationResponse,
	type ITelemetryAlertRule,
	type ITelemetryAlertRuleDeleteResponse,
	type ITelemetryAlertRulesResponse,
	alertMetricLabel,
	alertModeLabel,
	alertRuleChannels,
	alertRulePath,
	alertRuleSummary,
	alertSourceHint,
	alertSourceLabel,
	alertSourceMismatch,
	formatAlertValue,
} from "./alerts-types";
import { EmptyState, StatTile } from "./telemetry-shared";

function windowLabel(minutes: number) {
	if (minutes >= 1440 && minutes % 1440 === 0) {
		const days = minutes / 1440;
		return i18next.t('daysdWindow', '{{days}}d window', { days });
	}
	if (minutes >= 60 && minutes % 60 === 0) {
		const hours = minutes / 60;
		return i18next.t('hourshWindow', '{{hours}}h window', { hours });
	}
	return i18next.t('minutesmWindow', '{{minutes}}m window', { minutes });
}

/** Icon plus label — the channel is never signalled by colour alone. */
function AlertChannelBadge({
	channel,
}: {
	readonly channel: AlertChannelMeta;
}) {
	const Icon = channel.value === "email" ? Mail : BellRing;
	return (
		<Badge
			variant="outline"
			className="text-[10px]"
			title={channel.description}
		>
			<Icon className="mr-1 h-3 w-3" aria-hidden="true" />
			{channel.label}
		</Badge>
	);
}

function AlertRuleRow({
	rule,
	onEdit,
	onToggle,
	onDelete,
	busy,
}: {
	readonly rule: ITelemetryAlertRule;
	readonly onEdit: (rule: ITelemetryAlertRule) => void;
	readonly onToggle: (rule: ITelemetryAlertRule, enabled: boolean) => void;
	readonly onDelete: (rule: ITelemetryAlertRule) => void;
	readonly busy: boolean;
}) {
	const { t } = useTranslation("admin");
	const mismatch = alertSourceMismatch(rule.metric, rule.source);
	const channels = alertRuleChannels(rule);
	return (
		<div className="flex flex-wrap items-start gap-4 border-b px-4 py-3 last:border-b-0">
			<div className="min-w-0 flex-1 space-y-1">
				<div className="flex flex-wrap items-center gap-2">
					<span className="truncate text-sm font-semibold">{rule.name}</span>
					<Badge variant="outline" className="text-[10px]">
						{alertMetricLabel(rule.metric)}
					</Badge>
					<Badge
						variant={rule.mode === "anomaly" ? "secondary" : "outline"}
						className="text-[10px]"
					>
						{alertModeLabel(rule.mode)}
					</Badge>
					<span className="text-[11px] text-muted-foreground">
						{windowLabel(rule.windowMinutes)}
					</span>
					{rule.source ? (
						<Badge
							variant="outline"
							className="text-[10px]"
							title={alertSourceHint(rule.source)}
						>
							{alertSourceLabel(rule.source)}
						</Badge>
					) : null}
					{rule.enabled ? null : (
						<Badge variant="outline" className="text-[10px]">
							{t('disabled', 'Disabled')}
						</Badge>
					)}
					{channels.length > 0 ? (
						channels.map((channel) => (
							<AlertChannelBadge key={channel.value} channel={channel} />
						))
					) : (
						<span
							className="text-[11px] text-muted-foreground"
							title={t('thisRuleOnlyWritesToTheInappInbox', 'This rule only writes to the in-app inbox.')}
						>
							{t('inboxOnly', 'Inbox only')}
						</span>
					)}
					{mismatch ? (
						<Badge
							variant="outline"
							className="border-amber-500/50 text-[10px] text-amber-700 dark:text-amber-400"
							title={mismatch}
						>
							<TriangleAlert className="mr-1 h-3 w-3" />
							{t('neverFires', 'Never fires')}
						</Badge>
					) : null}
				</div>
				<div className="truncate text-sm text-muted-foreground">
					{alertRuleSummary(rule)}
				</div>
				{mismatch ? (
					<div className="text-[11px] text-amber-700 dark:text-amber-400">
						{mismatch}
					</div>
				) : null}
				<div className="flex flex-wrap items-center gap-1.5 text-[11px] text-muted-foreground">
					<span>{t('lastValue', 'last value')}</span>
					<span className="font-mono tabular-nums">
						{formatAlertValue(rule.metric, rule.lastValue)}
					</span>
					{rule.mode === "threshold" ? (
						<>
							<span>vs</span>
							<span className="font-mono tabular-nums">
								{formatAlertValue(rule.metric, rule.threshold)}
							</span>
						</>
					) : null}
					<span>·</span>
					{rule.lastTriggeredAt ? (
						<>
							<span>{t('lastTriggered', 'last triggered')}</span>
							<RelativeTime value={rule.lastTriggeredAt} />
						</>
					) : (
						<span>{t('neverTriggered', 'never triggered')}</span>
					)}
					{rule.lastEvaluatedAt ? (
						<>
							<span>·</span>
							<span>evaluated</span>
							<RelativeTime value={rule.lastEvaluatedAt} />
						</>
					) : null}
				</div>
			</div>
			<div className="flex shrink-0 items-center gap-2">
				<div className="flex items-center gap-1.5">
					<Switch
						id={`alert-rule-enabled-${rule.id}`}
						checked={rule.enabled}
						disabled={busy}
						onCheckedChange={(enabled) => onToggle(rule, enabled)}
						aria-label={rule.enabled ? t('disableThisRule', 'Disable this rule') : t('enableThisRule', 'Enable this rule')}
					/>
					<span className="text-[11px] text-muted-foreground">
						{rule.enabled ? "Enabled" : "Disabled"}
					</span>
				</div>
				<Button
					variant="ghost"
					size="sm"
					onClick={() => onEdit(rule)}
					aria-label={t('editName', 'Edit {{name}}', { name: rule.name })}
				>
					<Pencil className="h-3.5 w-3.5" />
				</Button>
				<Button
					variant="ghost"
					size="sm"
					onClick={() => onDelete(rule)}
					aria-label={`Delete ${rule.name}`}
				>
					<Trash2 className="h-3.5 w-3.5 text-destructive" />
				</Button>
			</div>
		</div>
	);
}

export function AdminTelemetryAlertsPage() {
	const { t } = useTranslation("admin");
	const backend = useBackend();
	const auth = useAuth();
	const queryClient = useQueryClient();

	const profile = useInvoke(
		backend.userState.getProfile,
		backend.userState,
		[],
	);
	const info = useInvoke(
		backend.userState.getInfo,
		backend.userState,
		[],
		Boolean(auth?.isAuthenticated),
		[auth?.user?.profile?.sub, auth?.isAuthenticated],
	);

	const [dialogOpen, setDialogOpen] = useState(false);
	const [editingRule, setEditingRule] = useState<ITelemetryAlertRule | null>(
		null,
	);
	const [pendingDelete, setPendingDelete] =
		useState<ITelemetryAlertRule | null>(null);

	const rules = useQuery<ITelemetryAlertRulesResponse>({
		queryKey: [...ALERTS_QUERY_KEY, "rules"],
		queryFn: async () => {
			if (!profile.data) throw new Error("Profile not loaded");
			return backend.apiState.get<ITelemetryAlertRulesResponse>(
				profile.data,
				ALERT_RULES_PATH,
			);
		},
		enabled: !!profile.data,
	});

	const toggleRule = useMutation({
		mutationFn: async (input: { id: string; enabled: boolean }) => {
			if (!profile.data) throw new Error("Profile not loaded");
			return backend.apiState.patch(profile.data, alertRulePath(input.id), {
				enabled: input.enabled,
			});
		},
		onSuccess: async (_data, input) => {
			await queryClient.invalidateQueries({ queryKey: ALERTS_QUERY_KEY });
			toast.success(input.enabled ? "Rule enabled" : "Rule disabled");
		},
		onError: (error: Error) =>
			toast.error(error.message ?? "Failed to update the rule"),
	});

	const deleteRule = useMutation({
		mutationFn: async (id: string) => {
			if (!profile.data) throw new Error("Profile not loaded");
			return backend.apiState.del<ITelemetryAlertRuleDeleteResponse>(
				profile.data,
				alertRulePath(id),
			);
		},
		onSuccess: async (result) => {
			await queryClient.invalidateQueries({ queryKey: ALERTS_QUERY_KEY });
			setPendingDelete(null);
			const deleted = result?.eventsDeleted ?? 0;
			toast.success(
				deleted > 0
					? `Rule deleted — ${deleted.toLocaleString()} alert${deleted === 1 ? "" : "s"} removed from the inbox`
					: "Rule deleted",
			);
		},
		onError: (error: Error) =>
			toast.error(error.message ?? "Failed to delete the rule"),
	});

	const evaluate = useMutation({
		mutationFn: async () => {
			if (!profile.data) throw new Error("Profile not loaded");
			return backend.apiState.post<ITelemetryAlertEvaluationResponse>(
				profile.data,
				ALERT_EVALUATE_PATH,
			);
		},
		onSuccess: async (result) => {
			await queryClient.invalidateQueries({ queryKey: ALERTS_QUERY_KEY });
			toast.success(
				t('evaluatedEvaluatedRulesTriggeredTriggeredResolvedResolved', 'Evaluated {{evaluated}} rules — {{triggered}} triggered, {{resolved}} resolved', { evaluated: result.evaluated, triggered: result.triggered, resolved: result.resolved }),
			);
		},
		onError: (error: Error) =>
			toast.error(error.message ?? "Evaluation failed"),
	});

	const refresh = useCallback(() => {
		queryClient.invalidateQueries({ queryKey: ALERTS_QUERY_KEY });
	}, [queryClient]);

	const openCreate = useCallback(() => {
		setEditingRule(null);
		setDialogOpen(true);
	}, []);

	const openEdit = useCallback((rule: ITelemetryAlertRule) => {
		setEditingRule(rule);
		setDialogOpen(true);
	}, []);

	const onToggle = useCallback(
		(rule: ITelemetryAlertRule, enabled: boolean) =>
			toggleRule.mutate({ id: rule.id, enabled }),
		[toggleRule],
	);

	const ruleList = useMemo(() => rules.data?.rules ?? [], [rules.data?.rules]);

	const metricByRuleId = useMemo(() => {
		const map: Record<string, string> = {};
		for (const rule of ruleList) map[rule.id] = rule.metric;
		return map;
	}, [ruleList]);

	const enabledCount = ruleList.filter((rule) => rule.enabled).length;
	const anomalyCount = ruleList.filter(
		(rule) => rule.mode === "anomaly",
	).length;

	const perms = useMemo(
		() => new GlobalPermission(info.data?.permission ?? 0),
		[info.data?.permission],
	);
	const hasAccess = perms.hasPermission(GlobalPermission.Admin);

	if (info.isLoading) {
		return (
			<main className="flex h-full min-h-0 w-full grow flex-col bg-background p-6">
				<Skeleton className="h-12 w-72" />
				<div className="mt-4 space-y-2">
					<Skeleton className="h-16 w-full" />
					<Skeleton className="h-16 w-full" />
					<Skeleton className="h-16 w-full" />
				</div>
			</main>
		);
	}

	if (!hasAccess) {
		return (
			<main className="flex h-full w-full items-center justify-center bg-background p-6">
				<Card className="max-w-md text-center">
					<CardHeader>
						<CardTitle className="flex items-center justify-center gap-2 text-base">
							<Lock className="h-4 w-4" />
							{t('insufficientPermissions', 'Insufficient permissions')}
						</CardTitle>
						<CardDescription><Trans i18nKey="youNeedTheBadminbPermissionToManageTelemetryAlerts">You need the <b>Admin</b> permission to manage telemetry alerts.</Trans></CardDescription>
					</CardHeader>
				</Card>
			</main>
		);
	}

	return (
		<main className="flex h-full min-h-0 w-full grow flex-col overflow-hidden bg-background">
			<div className="flex-1 overflow-y-auto p-6">
				<div className="mx-auto max-w-7xl space-y-6">
					<div className="flex flex-wrap items-start justify-between gap-3">
						<div>
							<h1 className="flex items-center gap-2 text-3xl font-bold">
								<BellRing className="h-7 w-7 text-primary" />
								{t('alerts', 'Alerts')}
							</h1>
							<p className="text-muted-foreground">
								{t('thresholdAndAnomalyRulesOverAnonymousTelemetryEveryAlertLandsInTheInappInboxARuleCanAlsoMailThePlatformAlertingMailboxOrPushToEveryPlatformAdmin', "Threshold and anomaly rules over anonymous telemetry. Every alert lands in the in-app inbox; a rule can also mail the platform alerting mailbox or push to every platform admin.")}
							</p>
						</div>
						<div className="flex flex-wrap items-center gap-2">
							<Button asChild variant="ghost" size="sm">
								<Link href="/admin/telemetry">
									<ArrowLeft className="mr-1 h-3.5 w-3.5" />
									{t('telemetry', 'Telemetry')}
								</Link>
							</Button>
							<Button
								variant="outline"
								size="sm"
								onClick={() => evaluate.mutate()}
								disabled={evaluate.isPending || !profile.data}
							>
								<Play className="mr-1 h-3.5 w-3.5" />
								{evaluate.isPending ? "Evaluating…" : "Evaluate now"}
							</Button>
							<Button variant="outline" size="sm" onClick={refresh}>
								<RefreshCw className="mr-1 h-3.5 w-3.5" />
								{t('refresh', 'Refresh')}
							</Button>
							<Button size="sm" onClick={openCreate}>
								<Plus className="mr-1 h-3.5 w-3.5" />
								{t('newRule', 'New rule')}
							</Button>
						</div>
					</div>

					<div className="grid gap-3 sm:grid-cols-3">
						<StatTile
							label="Rules"
							value={ruleList.length.toLocaleString()}
							icon={<Gauge className="h-3.5 w-3.5" />}
							hint={t('valEnabled', '{{val}} enabled', { val: enabledCount.toLocaleString() })}
						/>
						<StatTile
							label={t('anomalyRules', 'Anomaly rules')}
							value={anomalyCount.toLocaleString()}
							hint="Baseline-relative detection"
						/>
						<StatTile
							label={t('thresholdRules', 'Threshold rules')}
							value={(ruleList.length - anomalyCount).toLocaleString()}
							hint="Fixed comparator against a value"
						/>
					</div>

					<Card>
						<CardHeader className="pb-3">
							<CardTitle className="text-base">{t('rules', 'Rules')}</CardTitle>
							<CardDescription>
								{t('eachRuleIsEvaluatedOverItsOwnWindowAnomalyRulesNeverFireBeforeTheirBaselineHasEnoughSamples', "Each rule is evaluated over its own window. Anomaly rules never fire before their baseline has enough samples.")}
							</CardDescription>
						</CardHeader>
						<CardContent className="p-0">
							{rules.isLoading ? (
								<div className="space-y-2 p-4">
									<Skeleton className="h-16 w-full" />
									<Skeleton className="h-16 w-full" />
									<Skeleton className="h-16 w-full" />
								</div>
							) : ruleList.length === 0 ? (
								<EmptyState
									message="No alert rules yet — create one to start watching a metric."
									className="m-4 py-10 text-sm"
								/>
							) : (
								<div>
									{ruleList.map((rule) => (
										<AlertRuleRow
											key={rule.id}
											rule={rule}
											onEdit={openEdit}
											onToggle={onToggle}
											onDelete={setPendingDelete}
											busy={toggleRule.isPending}
										/>
									))}
								</div>
							)}
						</CardContent>
					</Card>

					<TelemetryAlertsInbox
						profile={profile.data}
						metricByRuleId={metricByRuleId}
					/>
				</div>
			</div>

			<TelemetryAlertRuleDialog
				open={dialogOpen}
				onOpenChange={setDialogOpen}
				rule={editingRule}
				profile={profile.data}
			/>

			<AlertDialog
				open={Boolean(pendingDelete)}
				onOpenChange={(open) => {
					if (!open) setPendingDelete(null);
				}}
			>
				<AlertDialogContent>
					<AlertDialogHeader>
						<AlertDialogTitle>
							{t('delete2', 'Delete “')}{pendingDelete?.name}{t('andItsAlertHistory', '” and its alert history?')}
						</AlertDialogTitle>
						<AlertDialogDescription>
							{`The rule stops being evaluated and every alert it ever produced is permanently deleted from the inbox, including acknowledgements. This incident history is not stored anywhere else and cannot be restored.`}
						</AlertDialogDescription>
					</AlertDialogHeader>
					<AlertDialogFooter>
						<AlertDialogCancel disabled={deleteRule.isPending}>
							{t('cancel', 'Cancel')}
						</AlertDialogCancel>
						<AlertDialogAction
							className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
							onClick={(event) => {
								event.preventDefault();
								if (pendingDelete) deleteRule.mutate(pendingDelete.id);
							}}
							disabled={deleteRule.isPending}
						>
							{deleteRule.isPending
								? "Deleting…"
								: t('deleteRuleAndItsHistory', 'Delete rule and its history')}
						</AlertDialogAction>
					</AlertDialogFooter>
				</AlertDialogContent>
			</AlertDialog>
		</main>
	);
}
