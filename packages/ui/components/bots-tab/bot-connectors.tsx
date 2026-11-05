"use client";

import { useTranslation } from "@flow-like/locales";
import {
	Loader2Icon,
	PlugZapIcon,
	PlusIcon,
	RefreshCwIcon,
	Trash2Icon,
} from "lucide-react";
import { useState } from "react";
import { toast } from "sonner";
import {
	useBotConnectorAction,
	useBotConnectors,
	useBotSecrets,
	useConnectedAccounts,
} from "../../hooks/use-bots";
import { useBackend } from "../../state/backend-state";
import type {
	BotConnectorPolicy,
	IBotConnector,
	IConnectedAccount,
} from "../../state/backend-state/bot-state";
import { Button } from "../ui/button";
import { Input } from "../ui/input";
import {
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "../ui/select";
import {
	ACCOUNT_POLICY_KEYS,
	CONNECTOR_POLICIES,
	accountPolicy,
	isDrifted,
	normalizeMcpUrl,
	readsWriteTool,
	toolPolicy,
	withToolPolicy,
} from "./bot-connectors-model";
import { BotNotice, BotTag } from "./editor/bot-editor-parts";
import { useBotWorkspace } from "./use-bot-workspace";

function usePolicyLabels(): Record<BotConnectorPolicy, string> {
	const { t } = useTranslation("bots");
	return {
		read: t("connectors.policy.read", "Always allow"),
		ask: t("connectors.policy.ask", "Ask me"),
		never: t("connectors.policy.never", "Never"),
	};
}

function PolicySelect({
	label,
	value,
	onChange,
	disabled,
}: Readonly<{
	label: string;
	value: BotConnectorPolicy;
	onChange: (policy: BotConnectorPolicy) => void;
	disabled?: boolean;
}>) {
	const labels = usePolicyLabels();
	return (
		<Select
			value={value}
			onValueChange={(next) => onChange(next as BotConnectorPolicy)}
			disabled={disabled}
		>
			<SelectTrigger aria-label={label} className="h-8 w-36 text-xs">
				<SelectValue />
			</SelectTrigger>
			<SelectContent>
				{CONNECTOR_POLICIES.map((policy) => (
					<SelectItem key={policy} value={policy}>
						{labels[policy]}
					</SelectItem>
				))}
			</SelectContent>
		</Select>
	);
}

function ToolRow({
	connector,
	tool,
	busy,
	onChange,
	onConfirm,
}: Readonly<{
	connector: IBotConnector;
	tool: IBotConnector["tools_snapshot"][number];
	busy: boolean;
	onChange: (policies: Record<string, BotConnectorPolicy>) => void;
	onConfirm: () => void;
}>) {
	const { t } = useTranslation("bots");
	const { policy } = toolPolicy(connector, tool);
	const drifted = isDrifted(connector, tool);
	return (
		<li className="flex flex-col gap-1 py-2">
			<div className="flex items-center gap-2">
				<span className="min-w-0 flex-1 truncate font-mono text-xs">
					{tool.name}
				</span>
				{tool.destructive_hint && (
					<BotTag>{t("connectors.destructive", "Can delete")}</BotTag>
				)}
				{tool.read_only_hint && (
					<BotTag>{t("connectors.readOnly", "Read only")}</BotTag>
				)}
				<PolicySelect
					label={t("connectors.toolPolicy", "Access for {{name}}", {
						name: tool.name,
					})}
					value={policy}
					disabled={busy}
					onChange={(next) =>
						onChange(withToolPolicy(connector.tool_policies, tool.name, next))
					}
				/>
			</div>
			{drifted && (
				<div className="flex items-center gap-2 text-xs text-warning">
					<span className="flex-1">
						{t(
							"connectors.drifted",
							"This tool changed since you set it, so it asks every time.",
						)}
					</span>
					<Button
						type="button"
						variant="outline"
						size="sm"
						disabled={busy}
						onClick={onConfirm}
					>
						{t("connectors.confirm", "Accept the change")}
					</Button>
				</div>
			)}
			{tool.description && (
				<p className="line-clamp-2 text-xs text-muted-foreground">
					{tool.description}
				</p>
			)}
			{readsWriteTool(tool, policy) && (
				<p className="text-xs text-warning">
					{t(
						"connectors.writeWarning",
						"This tool can change things and will run without asking.",
					)}
				</p>
			)}
		</li>
	);
}

function AccountRows({
	connector,
	busy,
	onChange,
}: Readonly<{
	connector: IBotConnector;
	busy: boolean;
	onChange: (policies: Record<string, BotConnectorPolicy>) => void;
}>) {
	const { t } = useTranslation("bots");
	const names = {
		read: t("connectors.account.read", "Reading (GET, HEAD)"),
		write: t(
			"connectors.account.write",
			"Changing things (POST, PUT, PATCH, DELETE)",
		),
	};
	return (
		<ul className="divide-y divide-border">
			{ACCOUNT_POLICY_KEYS.map((key) => (
				<li key={key} className="flex items-center gap-2 py-2">
					<span className="min-w-0 flex-1 text-xs">{names[key]}</span>
					<PolicySelect
						label={names[key]}
						value={accountPolicy(connector, key).policy}
						disabled={busy}
						onChange={(next) =>
							onChange(withToolPolicy(connector.tool_policies, key, next))
						}
					/>
				</li>
			))}
		</ul>
	);
}

function ConnectorItem({
	botId,
	connector,
}: Readonly<{ botId: string; connector: IBotConnector }>) {
	const { t } = useTranslation("bots");
	const action = useBotConnectorAction();
	const mcp = connector.kind === "mcp";
	const fail = (error: unknown) =>
		toast.error(
			error instanceof Error
				? error.message
				: t("connectors.failed", "The connector could not be changed."),
		);
	const run = (request: Parameters<typeof action.mutate>[0]["action"]) =>
		action.mutate({ botId, action: request }, { onError: fail });
	return (
		<li
			data-slot="bot-connector"
			className="flex flex-col gap-2 rounded-lg border bg-card p-3"
		>
			<div className="flex flex-wrap items-center gap-2">
				<PlugZapIcon className="size-4 text-muted-foreground" aria-hidden />
				<span className="text-sm font-medium">{connector.label}</span>
				<BotTag>
					{mcp
						? t("connectors.kind.mcp", "MCP server")
						: t("connectors.kind.oauth", "Account")}
				</BotTag>
				<span className="min-w-0 flex-1 truncate text-xs text-muted-foreground">
					{mcp ? connector.url : connector.provider_id}
				</span>
				{mcp && (
					<Button
						type="button"
						variant="ghost"
						size="sm"
						disabled={action.isPending}
						onClick={() => run({ type: "refresh", connectorId: connector.id })}
					>
						{action.isPending ? (
							<Loader2Icon className="animate-spin" aria-hidden />
						) : (
							<RefreshCwIcon aria-hidden />
						)}
						{t("connectors.refresh", "Refresh tools")}
					</Button>
				)}
				<Button
					type="button"
					variant="ghost"
					size="icon"
					className="size-8"
					aria-label={t("connectors.remove", "Remove {{label}}", {
						label: connector.label,
					})}
					disabled={action.isPending}
					onClick={() => run({ type: "delete", connectorId: connector.id })}
				>
					<Trash2Icon aria-hidden />
				</Button>
			</div>
			<div className="flex items-center gap-2 text-xs">
				<span className="flex-1 text-muted-foreground">
					{t("connectors.default", "Tools without a hint of their own")}
				</span>
				<PolicySelect
					label={t("connectors.defaultPolicy", "Default access")}
					value={connector.default_policy}
					disabled={action.isPending}
					onChange={(next) =>
						run({
							type: "update",
							connectorId: connector.id,
							request: { default_policy: next },
						})
					}
				/>
			</div>
			{mcp ? (
				connector.tools_snapshot.length === 0 ? (
					<p className="text-xs text-muted-foreground">
						{t(
							"connectors.noTools",
							"No tools yet. Refresh to ask the server for them.",
						)}
					</p>
				) : (
					<ul className="divide-y divide-border">
						{connector.tools_snapshot.map((tool) => (
							<ToolRow
								key={tool.name}
								connector={connector}
								tool={tool}
								busy={action.isPending}
								onChange={(policies) =>
									run({
										type: "update",
										connectorId: connector.id,
										request: { tool_policies: policies },
									})
								}
								onConfirm={() =>
									run({
										type: "update",
										connectorId: connector.id,
										request: { confirm_tools: [tool.name] },
									})
								}
							/>
						))}
					</ul>
				)
			) : (
				<AccountRows
					connector={connector}
					busy={action.isPending}
					onChange={(policies) =>
						run({
							type: "update",
							connectorId: connector.id,
							request: { tool_policies: policies },
						})
					}
				/>
			)}
		</li>
	);
}

const NO_SECRET = "__none__";

/** Providers an account connector can use; the hub configures which of them exist. */
const ACCOUNT_PROVIDERS = [
	"google",
	"microsoft",
	"github",
	"notion",
	"atlassian",
	"linkedin",
] as const;

/**
 * Opens the OAuth consent flow for the bot's workspace app: it signs the person in to the
 * provider when needed and records their consent for the workspace, which is what lets the
 * desktop send the account's token for this bot. Resolves false when nothing was needed.
 */
function useWorkspaceConsent(botId: string) {
	const backend = useBackend();
	const { query } = useBotWorkspace(botId);
	const appId = query.data?.app_id;
	return async (providerId: string, scopes: string[]): Promise<boolean> => {
		const check = backend.eventState.checkOAuthRequirements;
		if (!appId || !check) return false;
		const result = await check.call(backend.eventState, appId, [
			{ provider_id: providerId, scopes },
		]);
		if (result.missingProviders.length === 0) return false;
		window.dispatchEvent(
			new CustomEvent("flow:oauth-required", {
				detail: {
					missingProviders: result.missingProviders,
					appId,
					boardId: "",
					nodeId: "",
				},
			}),
		);
		return true;
	};
}

function AddMcpServer({
	botId,
	onDone,
}: Readonly<{ botId: string; onDone: () => void }>) {
	const { t } = useTranslation("bots");
	const action = useBotConnectorAction();
	const secrets = useBotSecrets(botId);
	const [label, setLabel] = useState("");
	const [url, setUrl] = useState("");
	const [secret, setSecret] = useState(NO_SECRET);
	const ready = label.trim() !== "" && normalizeMcpUrl(url) !== "";
	const submit = () =>
		action.mutate(
			{
				botId,
				action: {
					type: "create",
					request: {
						kind: "mcp",
						label: label.trim(),
						url: normalizeMcpUrl(url),
						auth:
							secret === NO_SECRET
								? { type: "none" }
								: { type: "secret", name: secret },
					},
				},
			},
			{
				onSuccess: (created) => {
					if (created) {
						action.mutate({
							botId,
							action: { type: "refresh", connectorId: created.id },
						});
					}
					onDone();
				},
				onError: (error) => toast.error(error.message),
			},
		);
	return (
		<div className="flex flex-col gap-2 rounded-lg border bg-muted/30 p-3">
			<Input
				aria-label={t("connectors.mcp.label", "Name")}
				placeholder={t("connectors.mcp.labelPlaceholder", "Name, e.g. Docs")}
				value={label}
				onChange={(event) => setLabel(event.target.value)}
				className="h-8 text-sm"
			/>
			<Input
				aria-label={t("connectors.mcp.url", "Server URL")}
				placeholder={t(
					"connectors.mcp.urlPlaceholder",
					"https://example.com/mcp",
				)}
				value={url}
				onChange={(event) => setUrl(event.target.value)}
				className="h-8 font-mono text-sm"
			/>
			<Select value={secret} onValueChange={setSecret}>
				<SelectTrigger
					aria-label={t("connectors.mcp.auth", "Sign-in")}
					className="h-8 text-sm"
				>
					<SelectValue />
				</SelectTrigger>
				<SelectContent>
					<SelectItem value={NO_SECRET}>
						{t("connectors.mcp.authNone", "No sign-in")}
					</SelectItem>
					{(secrets.data ?? []).map((item) => (
						<SelectItem key={item.name} value={item.name}>
							{t(
								"connectors.mcp.authSecret",
								"Bearer token from secret {{name}}",
								{
									name: item.name,
								},
							)}
						</SelectItem>
					))}
				</SelectContent>
			</Select>
			<div className="flex justify-end gap-2">
				<Button type="button" variant="ghost" size="sm" onClick={onDone}>
					{t("connectors.cancel", "Cancel")}
				</Button>
				<Button
					type="button"
					size="sm"
					disabled={!ready || action.isPending}
					onClick={submit}
				>
					{t("connectors.mcp.add", "Add server")}
				</Button>
			</div>
		</div>
	);
}

function AddAccount({
	botId,
	onDone,
}: Readonly<{ botId: string; onDone: () => void }>) {
	const { t } = useTranslation("bots");
	const action = useBotConnectorAction();
	const accounts = useConnectedAccounts();
	const consent = useWorkspaceConsent(botId);
	const connectNew = async (providerId: string) => {
		const started = await consent(providerId, []);
		if (!started) {
			toast.message(
				t(
					"connectors.account.connected",
					"{{provider}} is already connected for this bot.",
					{ provider: providerId },
				),
			);
		}
	};
	const usable = (accounts.data ?? []).filter(
		(account): account is IConnectedAccount & { provider_id: string } =>
			Boolean(account.provider_id) && account.state === "ready",
	);
	const [handle, setHandle] = useState("");
	const chosen = usable.find((account) => account.handle === handle);
	const submit = () => {
		if (!chosen) return;
		action.mutate(
			{
				botId,
				action: {
					type: "create",
					request: {
						kind: "oauth",
						label: chosen.provider_id,
						auth: { type: "oauth", provider_id: chosen.provider_id },
						grant: chosen.handle,
						scopes: chosen.scopes,
					},
				},
			},
			{
				onSuccess: () => {
					void consent(chosen.provider_id, chosen.scopes);
					onDone();
				},
				onError: (error) => toast.error(error.message),
			},
		);
	};
	return (
		<div className="flex flex-col gap-2 rounded-lg border bg-muted/30 p-3">
			{usable.length === 0 ? (
				<p className="text-xs text-muted-foreground">
					{t(
						"connectors.account.none",
						"No connected accounts yet. Connect one under Settings, Connected accounts, then pick it here.",
					)}
				</p>
			) : (
				<Select value={handle} onValueChange={setHandle}>
					<SelectTrigger
						aria-label={t("connectors.account.pick", "Account")}
						className="h-8 text-sm"
					>
						<SelectValue
							placeholder={t(
								"connectors.account.pickPlaceholder",
								"Pick an account",
							)}
						/>
					</SelectTrigger>
					<SelectContent>
						{usable.map((account) => (
							<SelectItem key={account.handle} value={account.handle}>
								{account.provider_id}
							</SelectItem>
						))}
					</SelectContent>
				</Select>
			)}
			<div className="flex flex-wrap items-center gap-1.5">
				<span className="text-xs text-muted-foreground">
					{t("connectors.account.connectNew", "Connect a new account:")}
				</span>
				{ACCOUNT_PROVIDERS.map((provider) => (
					<Button
						key={provider}
						type="button"
						variant="outline"
						size="sm"
						onClick={() => void connectNew(provider)}
					>
						{provider}
					</Button>
				))}
			</div>
			<div className="flex justify-end gap-2">
				<Button type="button" variant="ghost" size="sm" onClick={onDone}>
					{t("connectors.cancel", "Cancel")}
				</Button>
				<Button
					type="button"
					size="sm"
					disabled={!chosen || action.isPending}
					onClick={submit}
				>
					{t("connectors.account.add", "Add account")}
				</Button>
			</div>
		</div>
	);
}

/** The MCP servers and accounts the bot may use, with what each tool may do unasked. */
export function BotConnectorsCard({
	botId,
}: Readonly<{ botId: string | null }>) {
	const { t } = useTranslation("bots");
	const connectors = useBotConnectors(botId);
	const [adding, setAdding] = useState<"mcp" | "account" | null>(null);
	if (!botId) {
		return (
			<p className="text-xs text-muted-foreground">
				{t(
					"connectors.afterSave",
					"Save the bot first, then connect services here.",
				)}
			</p>
		);
	}
	const list = connectors.data ?? [];
	return (
		<div data-slot="bot-connectors" className="flex flex-col gap-2.5">
			<BotNotice>
				{t(
					"connectors.notice",
					"What these services send back is treated as data, never as instructions. Anything that changes things asks you first unless you allow it.",
				)}
			</BotNotice>
			{list.length > 0 && (
				<ul className="flex flex-col gap-2">
					{list.map((connector) => (
						<ConnectorItem
							key={connector.id}
							botId={botId}
							connector={connector}
						/>
					))}
				</ul>
			)}
			{adding === "mcp" && (
				<AddMcpServer botId={botId} onDone={() => setAdding(null)} />
			)}
			{adding === "account" && (
				<AddAccount botId={botId} onDone={() => setAdding(null)} />
			)}
			{adding === null && (
				<div className="flex flex-wrap gap-2">
					<Button
						type="button"
						variant="outline"
						size="sm"
						onClick={() => setAdding("mcp")}
					>
						<PlusIcon aria-hidden />
						{t("connectors.addMcp", "Add MCP server")}
					</Button>
					<Button
						type="button"
						variant="outline"
						size="sm"
						onClick={() => setAdding("account")}
					>
						<PlusIcon aria-hidden />
						{t("connectors.addAccount", "Add account")}
					</Button>
				</div>
			)}
		</div>
	);
}
