"use client";

import { useTranslation } from "@flow-like/locales";
import type { UseQueryResult } from "@tanstack/react-query";
import type { TFunction } from "i18next";
import {
	CheckCircle2,
	ChevronDown,
	Copy,
	Download,
	ExternalLink,
	Info,
	Loader2,
	RefreshCw,
	ShieldAlert,
} from "lucide-react";
import {
	type ReactElement,
	type ReactNode,
	useEffect,
	useId,
	useRef,
	useState,
} from "react";
import { toast } from "sonner";
import { useInvalidateInvoke, useInvoke } from "../../../hooks/use-invoke";
import {
	TEAMS_AUTH_MODES,
	TEAMS_PERMISSIONS,
	TEAMS_PERMISSION_RSC,
	type TeamsAuthMode,
	type TeamsBotAccess,
	type TeamsBotConnection,
	type TeamsBotPackage,
	type TeamsBotSetup,
	type TeamsFailure,
	type TeamsOperation,
	type TeamsPermission,
	type TeamsSetupField,
	isTeamsBotLocked,
	parseTeamsApprovers,
	teamsConsentUrl,
	teamsFailure,
	teamsFailureDetail,
	teamsPackageFilename,
	teamsSetupChangesConnectedBot,
	teamsSetupFromConnection,
	validateTeamsSetup,
	withTeamsAuthMode,
	withTeamsPermission,
} from "../../../lib/teams-bot";
import { cn } from "../../../lib/utils";
import { useBackend } from "../../../state/backend-state";
import {
	AlertDialog,
	AlertDialogAction,
	AlertDialogCancel,
	AlertDialogContent,
	AlertDialogDescription,
	AlertDialogFooter,
	AlertDialogHeader,
	AlertDialogTitle,
} from "../../ui/alert-dialog";
import { Badge } from "../../ui/badge";
import { Button } from "../../ui/button";
import {
	Collapsible,
	CollapsibleContent,
	CollapsibleTrigger,
} from "../../ui/collapsible";
import { Input } from "../../ui/input";
import { Label } from "../../ui/label";
import { RadioGroup, RadioGroupItem } from "../../ui/radio-group";
import { Switch } from "../../ui/switch";
import { Textarea } from "../../ui/textarea";
import type { IConfigInterfaceProps } from "../interfaces";

interface TeamsErrorView {
	summary: string;
	detail?: string;
}

const EMPTY_SETUP: TeamsBotSetup = {
	mode: "flow_like_managed",
	name: "Flow-Like Bot",
	description: "",
	customer_tenant_id: "",
	home_tenant_id: "",
	client_id: "",
	client_secret: "",
	allowed_responders: [],
	permissions: [],
};

const SERVER_OPERATION_STATUSES = new Set(["provisioning", "disconnecting"]);

const TENANT_HELP_URL =
	"https://learn.microsoft.com/entra/fundamentals/how-to-find-tenant";

async function unavailable(
	_appId: string,
	_eventId: string,
): Promise<TeamsBotConnection> {
	throw new Error("Teams bots are not supported by this backend");
}

function describedBy(id: string, invalid: boolean, hint = true) {
	const ids = [hint && `${id}-hint`, invalid && `${id}-error`].filter(Boolean);
	return ids.length > 0 ? ids.join(" ") : undefined;
}

function saveTeamsPackage(result: TeamsBotPackage) {
	const bytes = Uint8Array.from(atob(result.base64), (character) =>
		character.charCodeAt(0),
	);
	const url = URL.createObjectURL(
		new Blob([bytes], { type: "application/zip" }),
	);
	const link = document.createElement("a");
	link.href = url;
	link.download = teamsPackageFilename(result);
	link.click();
	setTimeout(() => URL.revokeObjectURL(url), 1000);
}

export function TeamsConfig({
	appId,
	eventId,
	isEditing,
}: IConfigInterfaceProps) {
	const { t } = useTranslation("interfaces");
	const backend = useBackend();
	const events = backend.eventState;
	const offline = useInvoke(backend.isOffline, backend, [appId], !!appId);
	const get = events.getTeamsBot;
	const connection = useInvoke(
		get ?? unavailable,
		events,
		[appId, eventId ?? ""],
		!!eventId && offline.data === false && !!get,
	);
	const gate = teamsGateMessage(t, {
		offline,
		saved: !!eventId,
		supported: !!get,
		connection,
	});
	if (gate) return gate;
	if (!connection.data || !eventId) return null;
	return (
		<TeamsSetupForm
			appId={appId}
			eventId={eventId}
			isEditing={isEditing}
			saved={connection.data}
			refreshing={connection.isFetching}
			onRefresh={() => connection.refetch()}
		/>
	);
}

type Translate = TFunction<"interfaces">;

/** The prerequisite that is still missing, in the order a user resolves them. */
function teamsGateMessage(
	t: Translate,
	state: {
		offline: UseQueryResult<boolean, Error>;
		saved: boolean;
		supported: boolean;
		connection: UseQueryResult<TeamsBotConnection, Error>;
	},
): ReactElement | null {
	const { offline } = state;
	if (offline.isPending)
		return (
			<p className="text-sm" aria-live="polite">
				{t("checkingAppConnection", "Checking app connection…")}
			</p>
		);
	if (offline.error)
		return (
			<p role="alert" className="text-sm text-destructive">
				{t(
					"couldNotCheckTheAppConnection",
					"Could not check the app connection: {{message}}",
					{ message: offline.error.message },
				)}
			</p>
		);
	if (offline.data !== false)
		return (
			<p className="text-sm">
				{t(
					"teamsSyncThisAppToRunABot",
					"Sync this app to an online profile to run a Teams bot.",
				)}
			</p>
		);
	if (!state.saved)
		return (
			<p className="text-sm" aria-live="polite">
				{t(
					"teamsSaveThisEventToSetUpItsBot",
					"Save this Teams event to set up its bot and messaging endpoint.",
				)}
			</p>
		);
	if (!state.supported)
		return (
			<p className="text-sm">
				{t(
					"teamsThisServerDoesNotSupportTeamsBots",
					"This server does not support Teams bots yet.",
				)}
			</p>
		);
	return teamsConnectionGate(t, state.connection);
}

function teamsConnectionGate(
	t: Translate,
	connection: UseQueryResult<TeamsBotConnection, Error>,
): ReactElement | null {
	if (connection.isPending)
		return (
			<p className="flex items-center gap-2 text-sm" aria-live="polite">
				<Loader2 className="size-4 animate-spin" />
				{t("teamsLoadingSetup", "Loading Teams setup…")}
			</p>
		);
	if (!connection.error) return null;
	return (
		<div className="space-y-3">
			<p role="alert" className="text-sm text-destructive">
				{teamsFailureDetail(connection.error)}
			</p>
			<Button
				type="button"
				variant="outline"
				disabled={connection.isFetching}
				onClick={() => connection.refetch()}
			>
				<RefreshCw className="size-4" />
				{t("retry", "Retry")}
			</Button>
		</div>
	);
}

function teamsFailureSummary(
	t: Translate,
	failure: TeamsFailure,
): string | undefined {
	switch (failure) {
		case "forbidden":
			return t(
				"teamsFailureForbidden",
				"Teams bots are turned off on this server, or your role cannot change this event. Ask your administrator.",
			);
		case "busy":
			return t(
				"teamsFailureBusy",
				"Another operation on this bot is still running. Refresh the status, then try again.",
			);
		case "managed_limit":
			return t(
				"teamsFailureManagedLimit",
				"This app has reached its limit of Flow-Like managed bots. Disconnect a managed bot on another event, or connect your own bot instead.",
			);
		case "rate_limited":
			return t(
				"teamsFailureRateLimited",
				"Too many managed bot setups or renames for this app in the last hour. Try again later.",
			);
		case "managed_rejected":
			return t(
				"teamsFailureManagedRejected",
				"The server could not create or update the managed bot. If managed bots are not enabled here, connect your own bot instead.",
			);
		case "rejected":
			return t(
				"teamsFailureRejected",
				"Microsoft or the server rejected these settings. Check the IDs and secret, then save again. The saved bot is unchanged.",
			);
		case "unavailable":
			return t(
				"teamsFailureUnavailable",
				"Microsoft or the server could not be reached. Try again in a moment.",
			);
		default:
			return undefined;
	}
}

function TeamsSetupForm({
	appId,
	eventId,
	isEditing,
	saved,
	refreshing,
	onRefresh,
}: Readonly<{
	appId: string;
	eventId: string;
	isEditing: boolean;
	saved: TeamsBotConnection;
	refreshing: boolean;
	onRefresh: () => void;
}>) {
	const { t } = useTranslation("interfaces");
	const backend = useBackend();
	const events = backend.eventState;
	const get = events.getTeamsBot;
	const checkAccess = events.getTeamsBotAccess;
	const prefix = useId();
	const invalidate = useInvalidateInvoke();
	const [form, setForm] = useState<TeamsBotSetup>(EMPTY_SETUP);
	const [approvers, setApprovers] = useState("");
	const [busy, setBusy] = useState<TeamsOperation | null>(null);
	const [error, setError] = useState<TeamsErrorView | null>(null);
	const [fieldErrors, setFieldErrors] = useState<TeamsSetupField[]>([]);
	const [pendingSave, setPendingSave] = useState<TeamsBotSetup | null>(null);
	const [confirmDisconnect, setConfirmDisconnect] = useState(false);

	const scope = `${appId}/${eventId}`;
	const scopeRef = useRef<string | null>(scope);
	const hydratedScope = useRef<string | null>(null);

	useEffect(() => {
		scopeRef.current = scope;
		setBusy(null);
		setError(null);
		setFieldErrors([]);
		setPendingSave(null);
		setConfirmDisconnect(false);
		return () => {
			scopeRef.current = null;
		};
	}, [scope]);

	// Refetches must not overwrite a draft: after a failed save the entered
	// credentials stay for the retry. Only a new event or a successful write
	// replaces the form.
	useEffect(() => {
		if (hydratedScope.current === scope) return;
		hydratedScope.current = scope;
		setForm(teamsSetupFromConnection(saved));
		setApprovers(saved.allowed_responders.join("\n"));
	}, [scope, saved]);

	const editable = isEditing && !busy;
	const custom = form.mode !== "flow_like_managed";
	const locked = isTeamsBotLocked(saved);
	const serverBusy = SERVER_OPERATION_STATUSES.has(saved.status);
	const endpoint = saved.endpoint;
	const invalid = (field: TeamsSetupField) => fieldErrors.includes(field);
	const showAccess =
		saved.status === "ready" && (saved.permissions?.length ?? 0) > 0;

	function applyConnection(result: TeamsBotConnection) {
		setForm(teamsSetupFromConnection(result));
		setApprovers(result.allowed_responders.join("\n"));
	}

	function editField(field: TeamsSetupField, value: string) {
		setForm((previous) => ({ ...previous, [field]: value }));
		setFieldErrors((previous) => previous.filter((key) => key !== field));
	}

	async function run(
		operation: TeamsOperation,
		task: () => Promise<(() => void) | undefined>,
	) {
		const started = scope;
		const current = () => scopeRef.current === started;
		setBusy(operation);
		setError(null);
		try {
			const apply = await task();
			if (current()) apply?.();
		} catch (cause) {
			if (!current()) return;
			const detail = teamsFailureDetail(cause);
			const summary = teamsFailureSummary(
				t,
				teamsFailure(cause, operation, form.mode),
			);
			setError(summary ? { summary, detail } : { summary: detail });
		} finally {
			if (operation !== "download" && get)
				await invalidate(get, [appId, eventId]);
			if (operation === "setup" && checkAccess)
				await invalidate(checkAccess, [appId, eventId]);
			if (current()) setBusy(null);
		}
	}

	async function submit(input: TeamsBotSetup) {
		setPendingSave(null);
		await run("setup", async () => {
			const result = await events.setupTeamsBot?.(appId, eventId, input);
			if (!result) return undefined;
			return () => {
				applyConnection(result);
				toast.success(t("teamsBotSaved", "Teams bot saved"));
			};
		});
	}

	function save() {
		if (!events.setupTeamsBot) return;
		const input = {
			...form,
			allowed_responders: parseTeamsApprovers(approvers),
		};
		const issues = validateTeamsSetup(input, saved);
		setFieldErrors(issues);
		setError(null);
		if (issues.length > 0) return;
		if (teamsSetupChangesConnectedBot(input, saved)) {
			setPendingSave(input);
			return;
		}
		void submit(input);
	}

	function download() {
		void run("download", async () => {
			const result = await events.getTeamsBotPackage?.(appId, eventId);
			if (result) saveTeamsPackage(result);
			return undefined;
		});
	}

	function rotate() {
		void run("rotate", async () => {
			await events.rotateTeamsBotSecret?.(appId, eventId);
			return () =>
				toast.success(t("teamsBotSecretRenewed", "Bot secret renewed"));
		});
	}

	function disconnect() {
		setConfirmDisconnect(false);
		void run("disconnect", async () => {
			const result = await events.disconnectTeamsBot?.(appId, eventId);
			return () => {
				if (result) applyConnection(result);
				toast.success(t("teamsBotDisconnected", "Teams bot disconnected"));
			};
		});
	}

	async function copyEndpoint() {
		try {
			await navigator.clipboard.writeText(endpoint);
			toast.success(t("teamsEndpointCopied", "Endpoint copied"));
		} catch {
			setError({
				summary: t(
					"teamsSelectAndCopyTheEndpointManually",
					"Select and copy the endpoint manually.",
				),
			});
		}
	}

	const saveDisabled =
		!editable ||
		serverBusy ||
		!events.setupTeamsBot ||
		(form.mode === "flow_like_managed" && !saved.managed_available);

	return (
		<div className="max-w-3xl space-y-6">
			<p className="text-sm text-muted-foreground">
				{t(
					"teamsConnectThisChatEventToTeams",
					"Connect this Chat Event to Teams. Messages run your flow on the server; your desktop can be closed.",
				)}
			</p>
			<TeamsAuthPathPicker
				value={form.mode}
				managedAvailable={saved.managed_available}
				disabled={!editable || locked}
				locked={locked}
				onChange={(mode) => {
					setForm((previous) => withTeamsAuthMode(previous, mode));
					setFieldErrors([]);
				}}
			/>
			<div className="grid gap-4 sm:grid-cols-2">
				<TeamsField
					id={`${prefix}-name`}
					label={t("teamsBotName", "Bot name")}
					error={
						invalid("name")
							? t("teamsBotNameLength", "Use a bot name of 1–30 characters.")
							: undefined
					}
				>
					<Input
						id={`${prefix}-name`}
						value={form.name}
						maxLength={30}
						disabled={!editable}
						aria-invalid={invalid("name")}
						aria-describedby={describedBy(
							`${prefix}-name`,
							invalid("name"),
							false,
						)}
						onChange={(e) => editField("name", e.target.value)}
					/>
				</TeamsField>
				<TeamsField
					id={`${prefix}-tenant`}
					label={t("teamsOrganizationTenantId", "Teams organization tenant ID")}
					hint={<TenantIdHelp />}
					error={
						invalid("customer_tenant_id")
							? t(
									"teamsEnterTheTenantIdOfTheOrganization",
									"Enter the tenant ID of the organization that will use this bot.",
								)
							: undefined
					}
				>
					<Input
						id={`${prefix}-tenant`}
						value={form.customer_tenant_id}
						disabled={!editable}
						placeholder="00000000-0000-0000-0000-000000000000"
						aria-invalid={invalid("customer_tenant_id")}
						aria-describedby={describedBy(
							`${prefix}-tenant`,
							invalid("customer_tenant_id"),
						)}
						onChange={(e) => editField("customer_tenant_id", e.target.value)}
					/>
				</TeamsField>
			</div>
			<TeamsField
				id={`${prefix}-description`}
				label={t("teamsDescriptionOptional", "Description (optional)")}
				error={
					invalid("description")
						? t(
								"teamsDescriptionLength",
								"Keep the description to 4000 characters or fewer.",
							)
						: undefined
				}
			>
				<Textarea
					id={`${prefix}-description`}
					value={form.description}
					maxLength={4000}
					disabled={!editable}
					aria-invalid={invalid("description")}
					aria-describedby={describedBy(
						`${prefix}-description`,
						invalid("description"),
						false,
					)}
					onChange={(e) => editField("description", e.target.value)}
				/>
			</TeamsField>
			{custom && (
				<TeamsCustomerRegistration
					form={form}
					endpoint={endpoint}
					editable={editable}
					keepsSecret={!!saved.client_id && saved.status !== "disconnected"}
					invalid={invalid}
					onEdit={editField}
					onCopyEndpoint={copyEndpoint}
				/>
			)}
			<TeamsApprovers
				id={`${prefix}-approvers`}
				value={approvers}
				editable={editable}
				invalid={invalid("allowed_responders")}
				onChange={(value) => {
					setApprovers(value);
					setFieldErrors((previous) =>
						previous.filter((key) => key !== "allowed_responders"),
					);
				}}
			/>
			<TeamsPermissions
				permissions={form.permissions}
				editable={editable}
				onToggle={(permission, enabled) =>
					setForm((previous) =>
						withTeamsPermission(previous, permission, enabled),
					)
				}
			>
				{showAccess && checkAccess && (
					<TeamsGraphAccess
						appId={appId}
						eventId={eventId}
						check={checkAccess}
					/>
				)}
			</TeamsPermissions>
			{error && (
				<div role="alert" className="space-y-1 text-sm text-destructive">
					<p>{error.summary}</p>
					{error.detail && (
						<p className="text-xs text-muted-foreground">{error.detail}</p>
					)}
				</div>
			)}
			<div className="flex flex-wrap items-center gap-3">
				<Button type="button" disabled={saveDisabled} onClick={save}>
					{busy === "setup" && <Loader2 className="size-4 animate-spin" />}
					{saved.configured
						? t("teamsSaveBotSettings", "Save bot settings")
						: form.mode === "flow_like_managed"
							? t("teamsCreateBot", "Create bot")
							: t("teamsConnectBot", "Connect bot")}
				</Button>
				<Button
					type="button"
					variant="ghost"
					disabled={!!busy || refreshing}
					onClick={onRefresh}
				>
					<RefreshCw className={cn("size-4", refreshing && "animate-spin")} />
					{t("teamsRefreshStatus", "Refresh status")}
				</Button>
			</div>
			{busy === "setup" && (
				<p className="text-sm text-muted-foreground" aria-live="polite">
					{t(
						"teamsSettingUpTheBot",
						"Setting up the bot. This can take a minute. If Microsoft is still preparing the registration, save again to continue setup.",
					)}
				</p>
			)}
			<TeamsStatus connection={saved} />
			{saved.configured && (
				<TeamsInstall
					connection={saved}
					downloading={busy === "download"}
					disabled={!!busy || !events.getTeamsBotPackage}
					onDownload={download}
				/>
			)}
			{locked && (
				<TeamsMaintenance
					connection={saved}
					busy={busy}
					disabled={!editable || serverBusy}
					canRotate={!!events.rotateTeamsBotSecret}
					canDisconnect={!!events.disconnectTeamsBot}
					onRotate={rotate}
					onDisconnect={() => setConfirmDisconnect(true)}
				/>
			)}
			<AlertDialog
				open={pendingSave !== null}
				onOpenChange={(open) => {
					if (!open) setPendingSave(null);
				}}
			>
				<AlertDialogContent>
					<AlertDialogHeader>
						<AlertDialogTitle>
							{t("teamsChangeTheConnectedBot", "Change the connected bot?")}
						</AlertDialogTitle>
						<AlertDialogDescription>
							{t(
								"teamsChangeTheConnectedBotDescription",
								"These changes affect the bot's Microsoft identity or the organization that can use it. The current bot keeps working until the new settings are verified. If the app ID or tenant changes, download the Teams app again and reinstall it.",
							)}
						</AlertDialogDescription>
					</AlertDialogHeader>
					<AlertDialogFooter>
						<AlertDialogCancel>{t("cancel", "Cancel")}</AlertDialogCancel>
						<AlertDialogAction
							onClick={() => {
								if (pendingSave) void submit(pendingSave);
							}}
						>
							{t("teamsSaveChanges", "Save changes")}
						</AlertDialogAction>
					</AlertDialogFooter>
				</AlertDialogContent>
			</AlertDialog>
			<AlertDialog open={confirmDisconnect} onOpenChange={setConfirmDisconnect}>
				<AlertDialogContent>
					<AlertDialogHeader>
						<AlertDialogTitle>
							{t("teamsDisconnectThisBot", "Disconnect this bot?")}
						</AlertDialogTitle>
						<AlertDialogDescription>
							{saved.mode === "flow_like_managed"
								? t(
										"teamsDisconnectManagedDescription",
										"Disconnecting removes the managed Microsoft registration and stops this bot. Remove the app from Teams separately.",
									)
								: t(
										"teamsDisconnectCustomerDescription",
										"Disconnecting stops this event's bot access. Your Microsoft registration remains in your account.",
									)}
						</AlertDialogDescription>
					</AlertDialogHeader>
					<AlertDialogFooter>
						<AlertDialogCancel>{t("cancel", "Cancel")}</AlertDialogCancel>
						<AlertDialogAction
							className="bg-destructive text-white hover:bg-destructive/90"
							onClick={disconnect}
						>
							{t("teamsDisconnectBot", "Disconnect bot")}
						</AlertDialogAction>
					</AlertDialogFooter>
				</AlertDialogContent>
			</AlertDialog>
		</div>
	);
}

function TeamsField({
	id,
	label,
	hint,
	error,
	children,
}: Readonly<{
	id: string;
	label: string;
	hint?: ReactNode;
	error?: string;
	children: ReactNode;
}>) {
	return (
		<div className="space-y-2">
			<Label htmlFor={id}>{label}</Label>
			{children}
			{hint && (
				<div id={`${id}-hint`} className="text-xs text-muted-foreground">
					{hint}
				</div>
			)}
			{error && (
				<p id={`${id}-error`} role="alert" className="text-xs text-destructive">
					{error}
				</p>
			)}
		</div>
	);
}

interface TeamsAuthPathCopy {
	title: string;
	summary: string;
	pros: string;
	cons: string;
	permissions: string;
	effort: string;
	signIn: string;
}

function useTeamsAuthPaths(): Record<TeamsAuthMode, TeamsAuthPathCopy> {
	const { t } = useTranslation("interfaces");
	return {
		flow_like_managed: {
			title: t("teamsManagedTitle", "Flow-Like managed"),
			summary: t(
				"teamsManagedSummary",
				"Flow-Like creates the bot and manages its Microsoft credentials.",
			),
			pros: t(
				"teamsManagedPros",
				"Fastest setup. No Azure subscription or bot registration needed.",
			),
			cons: t(
				"teamsManagedCons",
				"The bot identity belongs to Flow-Like; you cannot change its Microsoft-side settings.",
			),
			permissions: t(
				"teamsManagedPermissions",
				"Permission to upload custom apps in Teams, or a Teams admin who uploads it for you.",
			),
			effort: t(
				"teamsManagedEffort",
				"About 2 minutes: enter your tenant ID, create the bot, then upload the app to Teams.",
			),
			signIn: t(
				"teamsManagedSignIn",
				"No Microsoft sign-in. Teams tells the flow who sent each message; the bot cannot act as that person in Microsoft 365.",
			),
		},
		customer_teams: {
			title: t("teamsCustomerTeamsTitle", "Your bot in Teams"),
			summary: t(
				"teamsCustomerTeamsSummary",
				"Connect a bot you register in Teams Developer Portal.",
			),
			pros: t(
				"teamsCustomerTeamsPros",
				"You own the bot identity. No Azure subscription needed.",
			),
			cons: t(
				"teamsCustomerTeamsCons",
				"You create the client secret and renew it before it expires.",
			),
			permissions: t(
				"teamsCustomerTeamsPermissions",
				"Permission to register apps in Microsoft Entra ID and to upload custom apps in Teams. Your Microsoft admin may need to help.",
			),
			effort: t(
				"teamsCustomerTeamsEffort",
				"About 15 minutes: register the bot, create a client secret, and paste its IDs here.",
			),
			signIn: t(
				"teamsCustomerTeamsSignIn",
				"No Microsoft sign-in in this integration. Teams identifies each sender; flows cannot call Microsoft 365 as that person.",
			),
		},
		customer_azure: {
			title: t("teamsCustomerAzureTitle", "Your bot in Azure"),
			summary: t(
				"teamsCustomerAzureSummary",
				"Connect an Azure Bot resource your organization owns.",
			),
			pros: t(
				"teamsCustomerAzurePros",
				"Full control of the Azure resource, its channels and Microsoft-side policies.",
			),
			cons: t(
				"teamsCustomerAzureCons",
				"Needs an Azure subscription. You maintain the resource and renew its secret.",
			),
			permissions: t(
				"teamsCustomerAzurePermissions",
				"An Azure subscription, permission to create the Azure Bot and its Entra app, and permission to upload custom apps in Teams.",
			),
			effort: t(
				"teamsCustomerAzureEffort",
				"About 30 minutes: create the Azure Bot, enable its Teams channel, create a client secret, and paste its IDs here.",
			),
			signIn: t(
				"teamsCustomerAzureSignIn",
				"No Microsoft sign-in in this integration, even if you add an OAuth connection to the Azure Bot.",
			),
		},
	};
}

function TeamsAuthPathPicker({
	value,
	managedAvailable,
	disabled,
	locked,
	onChange,
}: Readonly<{
	value: TeamsAuthMode;
	managedAvailable: boolean;
	disabled: boolean;
	locked: boolean;
	onChange: (mode: TeamsAuthMode) => void;
}>) {
	const { t } = useTranslation("interfaces");
	const paths = useTeamsAuthPaths();
	const prefix = useId();
	const facts: TeamsAuthPathFact[] = [
		["pros", t("teamsPros", "Pros")],
		["cons", t("teamsCons", "Cons")],
		["permissions", t("teamsYouNeed", "You need")],
		["effort", t("teamsWhatToExpect", "What to expect")],
		["signIn", t("teamsMicrosoftSignIn", "Microsoft sign-in")],
	];
	return (
		<fieldset className="space-y-3" disabled={disabled}>
			<legend className="mb-2 text-sm font-medium">
				{t("teamsWhoManagesTheBot", "Who manages the bot?")}
			</legend>
			<RadioGroup
				value={value}
				disabled={disabled}
				onValueChange={(mode) => onChange(mode as TeamsAuthMode)}
				className="gap-2"
			>
				{TEAMS_AUTH_MODES.map((mode) => (
					<TeamsAuthPathRow
						key={mode}
						id={`${prefix}-${mode}`}
						mode={mode}
						path={paths[mode]}
						facts={facts}
						selected={value === mode}
						unavailable={mode === "flow_like_managed" && !managedAvailable}
						disabled={disabled}
					/>
				))}
			</RadioGroup>
			{locked && (
				<p className="text-xs text-muted-foreground">
					{t(
						"teamsDisconnectToChooseAnotherPath",
						"Disconnect this bot to choose another management path.",
					)}
				</p>
			)}
		</fieldset>
	);
}

type TeamsAuthPathFact = [keyof TeamsAuthPathCopy, string];

function TeamsAuthPathRow({
	id,
	mode,
	path,
	facts,
	selected,
	unavailable,
	disabled,
}: Readonly<{
	id: string;
	mode: TeamsAuthMode;
	path: TeamsAuthPathCopy;
	facts: readonly TeamsAuthPathFact[];
	selected: boolean;
	unavailable: boolean;
	disabled: boolean;
}>) {
	const { t } = useTranslation("interfaces");
	const inactive = disabled || unavailable;
	return (
		<div
			className={cn(
				"rounded-lg border text-sm transition-colors",
				selected ? "border-primary bg-primary/5" : "border-border",
				!selected && (inactive ? "opacity-60" : "hover:bg-muted/50"),
			)}
		>
			<label
				htmlFor={id}
				className={cn(
					"flex items-start gap-3 p-4",
					inactive ? "cursor-not-allowed" : "cursor-pointer",
				)}
			>
				<RadioGroupItem
					id={id}
					value={mode}
					disabled={unavailable}
					className="mt-0.5"
					aria-describedby={
						selected ? `${id}-summary ${id}-facts` : `${id}-summary`
					}
				/>
				<span className="min-w-0 flex-1 space-y-0.5">
					<span className="flex flex-wrap items-center gap-x-2 gap-y-1">
						<span className="font-medium">{path.title}</span>
						{unavailable && (
							<Badge
								variant="outline"
								className="font-normal text-muted-foreground"
							>
								{t("teamsNotEnabledOnThisServer", "Not enabled on this server")}
							</Badge>
						)}
					</span>
					<span id={`${id}-summary`} className="block text-muted-foreground">
						{path.summary}
					</span>
				</span>
			</label>
			{selected && (
				<dl
					id={`${id}-facts`}
					className="grid gap-x-6 gap-y-3 pr-4 pb-4 pl-11 text-xs sm:grid-cols-2"
				>
					{facts.map(([key, label], index) => (
						<div
							key={key}
							className={cn(
								"space-y-0.5",
								index === facts.length - 1 &&
									index % 2 === 0 &&
									"sm:col-span-2",
							)}
						>
							<dt className="font-medium">{label}</dt>
							<dd className="text-muted-foreground">{path[key]}</dd>
						</div>
					))}
				</dl>
			)}
		</div>
	);
}

function TenantIdHelp() {
	const { t } = useTranslation("interfaces");
	return (
		<span>
			{t(
				"teamsTenantIdHelp",
				"Only this organization's Teams conversations can run the event. Find the ID in the Microsoft Entra admin center under Overview → Tenant ID, or ask your Microsoft 365 admin.",
			)}{" "}
			<ExternalLinkText href={TENANT_HELP_URL}>
				{t("teamsHowToFindYourTenantId", "How to find your tenant ID")}
			</ExternalLinkText>
		</span>
	);
}

function ExternalLinkText({
	href,
	children,
}: Readonly<{ href: string; children: ReactNode }>) {
	return (
		<a
			className="inline-flex items-center gap-1 text-primary underline"
			href={href}
			target="_blank"
			rel="noreferrer"
		>
			{children}
			<ExternalLink className="size-3" />
		</a>
	);
}

function TeamsCustomerRegistration({
	form,
	endpoint,
	editable,
	keepsSecret,
	invalid,
	onEdit,
	onCopyEndpoint,
}: Readonly<{
	form: TeamsBotSetup;
	endpoint: string;
	editable: boolean;
	keepsSecret: boolean;
	invalid: (field: TeamsSetupField) => boolean;
	onEdit: (field: TeamsSetupField, value: string) => void;
	onCopyEndpoint: () => void;
}>) {
	const { t } = useTranslation("interfaces");
	const prefix = useId();
	const mode = form.mode;
	return (
		<div className="space-y-4 rounded-lg border p-4">
			<h3 className="text-sm font-medium">
				{t(
					"teamsConnectYourMicrosoftRegistration",
					"Connect your Microsoft registration",
				)}
			</h3>
			<ol className="list-decimal space-y-2 pl-5 text-sm text-muted-foreground">
				<li>
					{mode === "customer_teams" ? (
						<>
							{t(
								"teamsCreateABotInTeamsDeveloperPortal",
								"Create a bot in Teams Developer Portal under Tools → Bot management.",
							)}{" "}
							<ExternalLinkText href="https://dev.teams.microsoft.com/">
								{t(
									"teamsOpenTeamsDeveloperPortal",
									"Open Teams Developer Portal",
								)}
							</ExternalLinkText>
						</>
					) : (
						<>
							{t(
								"teamsCreateAnAzureBotInTheAzurePortal",
								"Create an Azure Bot in the Azure portal with app type Single Tenant, then enable its Microsoft Teams channel.",
							)}{" "}
							<ExternalLinkText href="https://portal.azure.com/">
								{t("teamsOpenAzurePortal", "Open Azure portal")}
							</ExternalLinkText>
						</>
					)}
				</li>
				<li>
					{t(
						"teamsSetItsMessagingEndpoint",
						"Set its messaging endpoint to the URL below.",
					)}
				</li>
				<li>
					{t(
						"teamsCopyItsIdsAndSecret",
						"Copy its application (client) ID, home tenant ID and client secret value into these fields.",
					)}
				</li>
			</ol>
			<div className="space-y-2">
				<Label htmlFor={`${prefix}-endpoint`}>
					{t("teamsMessagingEndpoint", "Messaging endpoint")}
				</Label>
				<div className="flex gap-2">
					<Input
						id={`${prefix}-endpoint`}
						readOnly
						value={endpoint}
						className="font-mono text-xs"
					/>
					<Button
						type="button"
						variant="outline"
						aria-label={t(
							"teamsCopyMessagingEndpoint",
							"Copy messaging endpoint",
						)}
						onClick={onCopyEndpoint}
					>
						<Copy className="size-4" />
					</Button>
				</div>
			</div>
			<div className="grid gap-4 sm:grid-cols-2">
				<TeamsField
					id={`${prefix}-client`}
					label={t("teamsApplicationClientId", "Application (client) ID")}
					hint={
						mode === "customer_teams"
							? t(
									"teamsClientIdHintTeams",
									"Shown as the bot ID in Teams Developer Portal.",
								)
							: t(
									"teamsClientIdHintAzure",
									"Shown as Microsoft App ID on the Azure Bot's Configuration page.",
								)
					}
					error={
						invalid("client_id")
							? t(
									"teamsEnterTheApplicationClientId",
									"Enter the bot's application (client) ID.",
								)
							: undefined
					}
				>
					<Input
						id={`${prefix}-client`}
						disabled={!editable}
						value={form.client_id}
						aria-invalid={invalid("client_id")}
						aria-describedby={describedBy(
							`${prefix}-client`,
							invalid("client_id"),
						)}
						onChange={(e) => onEdit("client_id", e.target.value)}
					/>
				</TeamsField>
				<TeamsField
					id={`${prefix}-home`}
					label={t("teamsAppsHomeTenantId", "App's home tenant ID")}
					hint={t(
						"teamsHomeTenantHint",
						"Shown as Directory (tenant) ID on the Entra app registration's Overview page.",
					)}
					error={
						invalid("home_tenant_id")
							? t(
									"teamsEnterTheHomeTenantId",
									"Enter the tenant ID where the bot's Entra app is registered.",
								)
							: undefined
					}
				>
					<Input
						id={`${prefix}-home`}
						disabled={!editable}
						value={form.home_tenant_id}
						aria-invalid={invalid("home_tenant_id")}
						aria-describedby={describedBy(
							`${prefix}-home`,
							invalid("home_tenant_id"),
						)}
						onChange={(e) => onEdit("home_tenant_id", e.target.value)}
					/>
				</TeamsField>
			</div>
			<TeamsField
				id={`${prefix}-secret`}
				label={t("teamsClientSecretValue", "Client secret value")}
				hint={t(
					"teamsSecretStorageHint",
					"Stored encrypted on the server. Excluded from the event configuration and the downloaded ZIP.",
				)}
				error={
					invalid("client_secret")
						? t(
								"teamsEnterTheClientSecretValue",
								"Enter the client secret value, not its secret ID.",
							)
						: undefined
				}
			>
				<Input
					id={`${prefix}-secret`}
					type="password"
					autoComplete="off"
					data-1p-ignore
					data-lpignore="true"
					data-bwignore
					data-form-type="other"
					disabled={!editable}
					value={form.client_secret}
					placeholder={
						keepsSecret
							? t(
									"teamsLeaveBlankToKeepTheSecret",
									"Leave blank to keep the saved secret",
								)
							: t(
									"teamsPasteTheValueNotTheId",
									"Paste the value, not the secret ID",
								)
					}
					aria-invalid={invalid("client_secret")}
					aria-describedby={describedBy(
						`${prefix}-secret`,
						invalid("client_secret"),
					)}
					onChange={(e) => onEdit("client_secret", e.target.value)}
				/>
			</TeamsField>
			<TeamsDisclosure
				title={t(
					"teamsUsingTheBotInAnotherTenant",
					"Using the bot in another tenant",
				)}
			>
				<p className="text-muted-foreground">
					{t(
						"teamsAnotherTenantHelp",
						"Set the Entra app's supported accounts to “Accounts in any organizational directory.” Keep the Azure Bot app type as Single Tenant. The home tenant field always identifies where the Entra app was registered.",
					)}
				</p>
			</TeamsDisclosure>
		</div>
	);
}

function TeamsDisclosure({
	title,
	open,
	onOpenChange,
	className,
	children,
}: Readonly<{
	title: string;
	open?: boolean;
	onOpenChange?: (open: boolean) => void;
	className?: string;
	children: ReactNode;
}>) {
	return (
		<Collapsible
			open={open}
			onOpenChange={onOpenChange}
			className={cn("text-sm", className)}
		>
			<CollapsibleTrigger className="group flex cursor-pointer items-center gap-1 font-medium">
				<ChevronDown className="size-4 transition-transform group-data-[state=closed]:-rotate-90" />
				{title}
			</CollapsibleTrigger>
			<CollapsibleContent className="mt-3 space-y-3">
				{children}
			</CollapsibleContent>
		</Collapsible>
	);
}

function TeamsApprovers({
	id,
	value,
	editable,
	invalid,
	onChange,
}: Readonly<{
	id: string;
	value: string;
	editable: boolean;
	invalid: boolean;
	onChange: (value: string) => void;
}>) {
	const { t } = useTranslation("interfaces");
	const [open, setOpen] = useState(false);
	return (
		<TeamsDisclosure
			title={t("teamsApprovalsAndActions", "Approvals and actions")}
			open={open || invalid}
			onOpenChange={setOpen}
			className="rounded-lg border p-4"
		>
			<p className="text-muted-foreground">
				{t(
					"teamsApprovalsHelp",
					"Use existing Interaction nodes to show choices or forms in Teams. By default, only the person who started the flow can answer. Replies continue the waiting execution; its timeout still applies.",
				)}
			</p>
			<TeamsField
				id={id}
				label={t(
					"teamsAllowedApproverObjectIds",
					"Allowed approver object IDs (optional)",
				)}
				hint={t(
					"teamsApproversHint",
					"When provided, only these people can answer. They must belong to the configured Teams organization and respond in the original conversation.",
				)}
				error={
					invalid
						? t(
								"teamsEnterValidApproverIds",
								"Enter up to 100 valid approver object IDs.",
							)
						: undefined
				}
			>
				<Textarea
					id={id}
					disabled={!editable}
					value={value}
					aria-invalid={invalid}
					aria-describedby={describedBy(id, invalid)}
					onChange={(e) => onChange(e.target.value)}
					placeholder={t(
						"teamsOneObjectIdPerLine",
						"One Microsoft Entra user object ID per line",
					)}
				/>
			</TeamsField>
		</TeamsDisclosure>
	);
}

interface TeamsPermissionCopy {
	title: string;
	description: string;
}

function useTeamsPermissionCopy(): Record<
	TeamsPermission,
	TeamsPermissionCopy
> {
	const { t } = useTranslation("interfaces");
	return {
		read_messages: {
			title: t(
				"teamsPermissionReadMessagesTitle",
				"Read channel and chat messages",
			),
			description: t(
				"teamsPermissionReadMessagesDescription",
				"Gives the flow the conversation around each mention, and lets the Get Teams Messages node read earlier messages.",
			),
		},
		meeting_details: {
			title: t("teamsPermissionMeetingDetailsTitle", "Read meeting details"),
			description: t(
				"teamsPermissionMeetingDetailsDescription",
				"Adds the meeting title, time, join link and organizer to runs started from a meeting chat.",
			),
		},
		conversation_details: {
			title: t(
				"teamsPermissionConversationDetailsTitle",
				"Read team, channel and chat details",
			),
			description: t(
				"teamsPermissionConversationDetailsDescription",
				"Adds team and channel descriptions, their links and group chat topics to the Teams context.",
			),
		},
	};
}

function TeamsPermissions({
	permissions,
	editable,
	onToggle,
	children,
}: Readonly<{
	permissions: readonly TeamsPermission[];
	editable: boolean;
	onToggle: (permission: TeamsPermission, enabled: boolean) => void;
	children?: ReactNode;
}>) {
	const { t } = useTranslation("interfaces");
	const copy = useTeamsPermissionCopy();
	const prefix = useId();
	return (
		<section
			aria-labelledby={`${prefix}-title`}
			className="space-y-4 rounded-lg border p-4"
		>
			<div className="space-y-1">
				<h3 id={`${prefix}-title`} className="text-sm font-medium">
					{t("teamsWhatTheBotCanRead", "What the bot can read")}
				</h3>
				<p className="text-sm text-muted-foreground">
					{t(
						"teamsWhatTheBotCanReadHint",
						"Without these permissions the bot only sees messages that @mention it or are sent to it directly.",
					)}
				</p>
			</div>
			<ul className="divide-y">
				{TEAMS_PERMISSIONS.map((permission) => {
					const enabled = permissions.includes(permission);
					return (
						<TeamsPermissionRow
							key={permission}
							id={`${prefix}-${permission}`}
							permission={permission}
							copy={copy[permission]}
							enabled={enabled}
							disabled={!editable}
							onToggle={(value) => onToggle(permission, value)}
						>
							{permission === "read_messages" && enabled && (
								<p className="flex gap-2 rounded-md border bg-muted/40 p-3 text-xs text-muted-foreground">
									<Info className="mt-px size-3.5 shrink-0" />
									{t(
										"teamsReadMessagesNote",
										"With this on, Teams sends the bot every message in the chats and channels where it is installed. The flow still runs only when someone @mentions the bot or writes to it in a 1:1 chat; other messages become conversation context, with each sender's name.",
									)}
								</p>
							)}
						</TeamsPermissionRow>
					);
				})}
			</ul>
			<p className="text-xs text-muted-foreground">
				{t(
					"teamsPermissionsApplyNote",
					"Changes apply after you download the updated app and update it in Teams. A team owner approves the permissions when installing it.",
				)}
			</p>
			{children}
		</section>
	);
}

function TeamsPermissionRow({
	id,
	permission,
	copy,
	enabled,
	disabled,
	onToggle,
	children,
}: Readonly<{
	id: string;
	permission: TeamsPermission;
	copy: TeamsPermissionCopy;
	enabled: boolean;
	disabled: boolean;
	onToggle: (enabled: boolean) => void;
	children?: ReactNode;
}>) {
	return (
		<li className="flex items-start justify-between gap-4 py-3 first:pt-0 last:pb-0">
			<div className="min-w-0 space-y-1.5">
				<Label htmlFor={id} className="leading-5">
					{copy.title}
				</Label>
				<p id={`${id}-description`} className="text-sm text-muted-foreground">
					{copy.description}
				</p>
				<p className="font-mono text-xs text-muted-foreground">
					{TEAMS_PERMISSION_RSC[permission].join(" · ")}
				</p>
				{children}
			</div>
			<Switch
				id={id}
				className="mt-0.5"
				checked={enabled}
				disabled={disabled}
				aria-describedby={`${id}-description`}
				onCheckedChange={onToggle}
			/>
		</li>
	);
}

interface TeamsAccessCopy {
	label: string;
	guidance?: string;
	detail?: string;
	tone: "ok" | "action" | "failed" | "neutral";
}

function useTeamsAccessCopy(
	access: UseQueryResult<TeamsBotAccess, Error>,
): TeamsAccessCopy {
	const { t } = useTranslation("interfaces");
	const failed = (detail?: string | null): TeamsAccessCopy => ({
		label: t("teamsGraphAccessFailed", "Check failed"),
		guidance: t(
			"teamsGraphAccessFailedGuidance",
			"Microsoft did not confirm access. Check again in a moment.",
		),
		detail: detail || undefined,
		tone: "failed",
	});
	if (access.isPending)
		return {
			label: t("teamsGraphAccessChecking", "Checking…"),
			tone: "neutral",
		};
	if (access.error) return failed(teamsFailureDetail(access.error));
	const { status, message } = access.data;
	switch (status) {
		case "ready":
			return {
				label: t("teamsGraphAccessReady", "Ready"),
				guidance: t(
					"teamsGraphAccessReadyGuidance",
					"Microsoft accepts the bot in your organization. Each team or chat still approves the permissions when the app is installed.",
				),
				tone: "ok",
			};
		case "consent_required":
			return {
				label: t("teamsGraphAccessConsentRequired", "Needs admin consent"),
				guidance: t(
					"teamsGraphAccessConsentRequiredGuidance",
					"An admin of your Microsoft 365 organization must approve the bot once before it can read messages or details.",
				),
				detail: message || undefined,
				tone: "action",
			};
		case "not_connected":
			return {
				label: t("teamsGraphAccessNotConnected", "Not connected"),
				guidance: t(
					"teamsGraphAccessNotConnectedGuidance",
					"Connect the bot, then check again.",
				),
				tone: "neutral",
			};
		case "not_needed":
			return {
				label: t("teamsGraphAccessNotNeeded", "Not needed"),
				guidance: t(
					"teamsGraphAccessNotNeededGuidance",
					"Save at least one read permission to check access.",
				),
				tone: "neutral",
			};
		default:
			return failed(message);
	}
}

const ACCESS_BADGE: Readonly<
	Record<TeamsAccessCopy["tone"], "secondary" | "outline" | "destructive">
> = {
	ok: "secondary",
	action: "outline",
	failed: "destructive",
	neutral: "secondary",
};

function TeamsAccessIcon({
	tone,
}: Readonly<{ tone: TeamsAccessCopy["tone"] }>) {
	if (tone === "ok") return <CheckCircle2 />;
	if (tone === "action") return <ShieldAlert />;
	return null;
}

function TeamsGraphAccess({
	appId,
	eventId,
	check,
}: Readonly<{
	appId: string;
	eventId: string;
	check: (appId: string, eventId: string) => Promise<TeamsBotAccess>;
}>) {
	const { t } = useTranslation("interfaces");
	const events = useBackend().eventState;
	const access = useInvoke(check, events, [appId, eventId]);
	const copy = useTeamsAccessCopy(access);
	const consentUrl = teamsConsentUrl(access.data);
	return (
		<div className="flex flex-wrap items-start justify-between gap-3 border-t pt-4">
			<output className="block min-w-0 flex-1 basis-64 space-y-1">
				<span className="flex flex-wrap items-center gap-2 text-sm">
					<span className="text-muted-foreground">
						{t("teamsGraphAccess", "Microsoft Graph access")}
					</span>
					<Badge variant={ACCESS_BADGE[copy.tone]}>
						{access.isPending ? (
							<Loader2 className="animate-spin" />
						) : (
							<TeamsAccessIcon tone={copy.tone} />
						)}
						{copy.label}
					</Badge>
				</span>
				{copy.guidance && (
					<span className="block text-sm text-muted-foreground">
						{copy.guidance}
					</span>
				)}
				{copy.detail && (
					<span className="block text-xs text-muted-foreground">
						{copy.detail}
					</span>
				)}
			</output>
			<div className="flex flex-wrap gap-2">
				{consentUrl && (
					<Button asChild size="sm">
						<a href={consentUrl} target="_blank" rel="noreferrer">
							{t("teamsGrantAdminConsent", "Grant admin consent")}
							<ExternalLink />
						</a>
					</Button>
				)}
				<Button
					type="button"
					size="sm"
					variant="outline"
					disabled={access.isFetching}
					onClick={() => access.refetch()}
				>
					<RefreshCw className={cn(access.isFetching && "animate-spin")} />
					{t("teamsCheckAgain", "Check again")}
				</Button>
			</div>
		</div>
	);
}

function useTeamsStatusCopy(status: string): {
	label: string;
	guidance: string;
	failed: boolean;
} {
	const { t } = useTranslation("interfaces");
	switch (status) {
		case "ready":
			return {
				label: t("teamsStatusReady", "Connected"),
				guidance: t(
					"teamsStatusReadyGuidance",
					"Credentials verified. Install the app in Teams and send the bot a test message.",
				),
				failed: false,
			};
		case "provisioning":
			return {
				label: t("teamsStatusProvisioning", "Setting up"),
				guidance: t(
					"teamsStatusProvisioningGuidance",
					"Microsoft is preparing the bot. Refresh the status in a minute; saving is paused until setup finishes.",
				),
				failed: false,
			};
		case "setup_failed":
			return {
				label: t("teamsStatusSetupFailed", "Setup failed"),
				guidance: t(
					"teamsStatusSetupFailedGuidance",
					"Setup did not finish. Review the details and save again to retry.",
				),
				failed: true,
			};
		case "disconnecting":
			return {
				label: t("teamsStatusDisconnecting", "Disconnecting"),
				guidance: t(
					"teamsStatusDisconnectingGuidance",
					"The bot is being removed. Refresh the status in a minute.",
				),
				failed: false,
			};
		case "disconnect_failed":
			return {
				label: t("teamsStatusDisconnectFailed", "Disconnect failed"),
				guidance: t(
					"teamsStatusDisconnectFailedGuidance",
					"Microsoft-side cleanup did not finish. Try disconnecting again.",
				),
				failed: true,
			};
		case "disconnected":
			return {
				label: t("teamsStatusDisconnected", "Disconnected"),
				guidance: t(
					"teamsStatusDisconnectedGuidance",
					"Choose who manages the bot and connect it again.",
				),
				failed: false,
			};
		case "not_configured":
			return {
				label: t("teamsStatusNotConfigured", "Not set up"),
				guidance: t(
					"teamsStatusNotConfiguredGuidance",
					"Choose who manages the bot, fill in the details and save.",
				),
				failed: false,
			};
		default:
			return {
				label: status.replaceAll("_", " "),
				guidance: t(
					"teamsStatusUnknownGuidance",
					"Refresh the status to see the latest state.",
				),
				failed: false,
			};
	}
}

function TeamsStatus({
	connection,
}: Readonly<{ connection: TeamsBotConnection }>) {
	const { t } = useTranslation("interfaces");
	const copy = useTeamsStatusCopy(connection.status);
	return (
		<div className="space-y-1 text-sm" aria-live="polite">
			<p className="flex flex-wrap items-center gap-2">
				<span className="text-muted-foreground">
					{t("teamsStatus", "Status")}
				</span>
				<Badge variant={copy.failed ? "destructive" : "secondary"}>
					{copy.label}
				</Badge>
				{connection.secret_expires_at && (
					<span className="text-xs text-muted-foreground">
						{t("teamsSecretExpires", "Secret expires {{date}}", {
							date: new Date(connection.secret_expires_at).toLocaleDateString(),
						})}
					</span>
				)}
			</p>
			<p className="text-muted-foreground">{copy.guidance}</p>
		</div>
	);
}

function TeamsInstall({
	connection,
	downloading,
	disabled,
	onDownload,
}: Readonly<{
	connection: TeamsBotConnection;
	downloading: boolean;
	disabled: boolean;
	onDownload: () => void;
}>) {
	const { t } = useTranslation("interfaces");
	return (
		<section className="space-y-3 rounded-lg border p-4">
			<h3 className="font-medium">
				{t("teamsInstallInTeams", "Install in Teams")}
			</h3>
			<p className="text-sm text-muted-foreground">
				{connection.last_activity_at
					? t(
							"teamsLastActivityReceived",
							"Last Teams activity received {{date}}.",
							{ date: new Date(connection.last_activity_at).toLocaleString() },
						)
					: t(
							"teamsSendATestMessageAfterInstallation",
							"Send a test message after installation, then refresh status to confirm Teams can reach this event.",
						)}
			</p>
			<p className="text-sm text-muted-foreground">
				{t(
					"teamsInstallSteps",
					"Download the app, then in Teams choose Apps → Manage your apps → Upload an app. If uploads are disabled, send the ZIP to your Teams admin. Activate this event before sending a message to the bot.",
				)}
			</p>
			<p className="text-sm text-muted-foreground">
				{t(
					"teamsUpdateAppAfterChanges",
					"After changing read permissions or after Flow-Like updates the Teams app, download the app again and update it in Teams.",
				)}
			</p>
			<Button
				type="button"
				variant="outline"
				disabled={disabled}
				onClick={onDownload}
			>
				{downloading ? (
					<Loader2 className="size-4 animate-spin" />
				) : (
					<Download className="size-4" />
				)}
				{t("teamsDownloadTeamsApp", "Download Teams app (.zip)")}
			</Button>
			<p className="text-xs text-muted-foreground">
				{t(
					"teamsSupportedInteractions",
					"Direct messages, mentions, and card replies are supported. Reading every message or calling Microsoft Graph as a user requires additional permissions and setup.",
				)}
			</p>
			<p className="text-xs text-muted-foreground">
				{t(
					"teamsFileSupport",
					"Files uploaded in 1:1 chats reach the bot. In group chats, channels and meeting chats Teams only delivers pasted images to bots; uploaded files are not sent. With “Read channel and chat messages” the bot sees the file's name and link, not its content.",
				)}
			</p>
		</section>
	);
}

function TeamsMaintenance({
	connection,
	busy,
	disabled,
	canRotate,
	canDisconnect,
	onRotate,
	onDisconnect,
}: Readonly<{
	connection: TeamsBotConnection;
	busy: TeamsOperation | null;
	disabled: boolean;
	canRotate: boolean;
	canDisconnect: boolean;
	onRotate: () => void;
	onDisconnect: () => void;
}>) {
	const { t } = useTranslation("interfaces");
	return (
		<TeamsDisclosure
			title={t("teamsConnectionMaintenance", "Connection maintenance")}
		>
			<div className="flex flex-wrap gap-2">
				{connection.mode === "flow_like_managed" && connection.configured && (
					<Button
						type="button"
						variant="outline"
						disabled={disabled || !canRotate}
						onClick={onRotate}
					>
						{busy === "rotate" && <Loader2 className="size-4 animate-spin" />}
						{t("teamsRenewBotSecret", "Renew bot secret")}
					</Button>
				)}
				<Button
					type="button"
					variant="ghost"
					disabled={disabled || !canDisconnect}
					onClick={onDisconnect}
				>
					{busy === "disconnect" && <Loader2 className="size-4 animate-spin" />}
					{t("teamsDisconnectBotEllipsis", "Disconnect bot…")}
				</Button>
			</div>
		</TeamsDisclosure>
	);
}
