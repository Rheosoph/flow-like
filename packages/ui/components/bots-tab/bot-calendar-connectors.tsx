"use client";

import { useTranslation } from "@flow-like/locales";
import { Loader2Icon, PlusIcon } from "lucide-react";
import { useState } from "react";
import { toast } from "sonner";
import {
	useBotConnectorAction,
	useBotConnectors,
	useConnectedAccounts,
} from "../../hooks/use-bots";
import { useHub } from "../../hooks/use-hub";
import type {
	IBotConnector,
	IConnectedAccount,
} from "../../state/backend-state/bot-state";
import { Button } from "../ui/button";
import {
	AddCalDav,
	ConnectorItem,
	useWorkspaceConsent,
} from "./bot-connectors";
import {
	CALENDAR_ACCOUNT_PROVIDERS,
	CALENDAR_ACCOUNT_SCOPES,
	type CalendarAccountProvider,
	calendarAccountProvider,
	calendarAccountRequest,
	hasCalendarScope,
	isCalendarConnector,
	withCalendarScopes,
} from "./bot-connectors-model";

function useProviderNames(): Record<CalendarAccountProvider, string> {
	const { t } = useTranslation("bots");
	return {
		google: t("connectors.calendarSetup.google", "Google Calendar"),
		microsoft: t("connectors.calendarSetup.microsoft", "Outlook calendar"),
	};
}

function readyAccount(
	accounts: readonly IConnectedAccount[] | undefined,
	provider: CalendarAccountProvider,
): IConnectedAccount | undefined {
	return accounts?.find(
		(account) =>
			account.provider_id?.toLowerCase() === provider &&
			account.state === "ready",
	);
}

/**
 * Attaches the person's connected account as a calendar, asking for calendar access. Without a
 * connected account it starts the sign-in; choosing the provider again afterwards attaches it.
 */
function ConnectCalendarAccount({
	botId,
	provider,
}: Readonly<{ botId: string; provider: CalendarAccountProvider }>) {
	const { t } = useTranslation("bots");
	const name = useProviderNames()[provider];
	const accounts = useConnectedAccounts();
	const action = useBotConnectorAction();
	const consent = useWorkspaceConsent(botId);
	const [busy, setBusy] = useState(false);
	const signIn = async () => {
		const started = await consent(provider, [
			...CALENDAR_ACCOUNT_SCOPES[provider],
		]);
		toast.message(
			started
				? t(
						"connectors.calendarSetup.finishSignIn",
						"Sign in to {{name}}, then choose {{name}} again to add it.",
						{ name },
					)
				: t(
						"connectors.account.none",
						"No connected accounts yet. Connect one under Settings, Connected accounts, then pick it here.",
					),
		);
	};
	const connect = async () => {
		setBusy(true);
		try {
			const fresh = await accounts.refetch();
			const account = readyAccount(fresh.data, provider);
			if (!account) {
				await signIn();
				return;
			}
			const request = calendarAccountRequest(provider, name, account);
			action.mutate(
				{ botId, action: { type: "create", request } },
				{
					onSuccess: () => void consent(provider, request.scopes ?? []),
					onError: (error) => toast.error(error.message),
				},
			);
		} finally {
			setBusy(false);
		}
	};
	return (
		<Button
			type="button"
			variant="outline"
			size="sm"
			disabled={busy || action.isPending}
			onClick={() => void connect()}
		>
			{busy || action.isPending ? (
				<Loader2Icon className="animate-spin" aria-hidden />
			) : (
				<PlusIcon aria-hidden />
			)}
			{name}
		</Button>
	);
}

/** An account attached before calendar access was asked for: the host's reads would be refused. */
function CalendarAccessRow({
	botId,
	connector,
	provider,
}: Readonly<{
	botId: string;
	connector: IBotConnector;
	provider: CalendarAccountProvider;
}>) {
	const { t } = useTranslation("bots");
	const action = useBotConnectorAction();
	const consent = useWorkspaceConsent(botId);
	const allow = () => {
		const scopes = withCalendarScopes(provider, connector.scopes);
		action.mutate(
			{
				botId,
				action: {
					type: "update",
					connectorId: connector.id,
					request: { scopes },
				},
			},
			{
				onSuccess: () => void consent(provider, scopes),
				onError: (error) => toast.error(error.message),
			},
		);
	};
	return (
		<div className="flex flex-wrap items-center gap-2 text-xs">
			<span className="min-w-0 flex-1 text-warning">
				{t(
					"connectors.calendarSetup.noAccess",
					"Connected without calendar access, so the bot can't read this calendar yet.",
				)}
			</span>
			<Button
				type="button"
				variant="outline"
				size="sm"
				disabled={action.isPending}
				onClick={allow}
			>
				{t("connectors.calendarSetup.allow", "Allow calendar access")}
			</Button>
		</div>
	);
}

/** The calendars the bot reads: Google and Outlook accounts and CalDAV servers. */
export function BotCalendarConnectorsCard({
	botId,
	localOnly,
}: Readonly<{ botId: string | null; localOnly: boolean }>) {
	const { t } = useTranslation("bots");
	const connectors = useBotConnectors(botId);
	const { hub } = useHub();
	const [addingCalDav, setAddingCalDav] = useState(false);
	if (!botId) {
		return (
			<p className="text-xs text-muted-foreground">
				{t(
					"connectors.calendarSetup.afterSave",
					"Save the bot first, then connect its calendars here.",
				)}
			</p>
		);
	}
	const calendars = (connectors.data ?? []).filter(isCalendarConnector);
	const linked = new Set(calendars.map(calendarAccountProvider));
	const offered = localOnly
		? []
		: CALENDAR_ACCOUNT_PROVIDERS.filter(
				(provider) =>
					Boolean(hub?.oauth_providers?.[provider]) && !linked.has(provider),
			);
	return (
		<div data-slot="bot-calendars" className="flex flex-col gap-2.5">
			{calendars.length > 0 && (
				<ul className="flex flex-col gap-2">
					{calendars.map((connector) => {
						const provider = calendarAccountProvider(connector);
						return (
							<ConnectorItem
								key={connector.id}
								botId={botId}
								connector={connector}
							>
								{provider && !hasCalendarScope(provider, connector.scopes) && (
									<CalendarAccessRow
										botId={botId}
										connector={connector}
										provider={provider}
									/>
								)}
							</ConnectorItem>
						);
					})}
				</ul>
			)}
			{addingCalDav ? (
				<AddCalDav botId={botId} onDone={() => setAddingCalDav(false)} />
			) : (
				<div className="flex flex-wrap gap-2">
					{offered.map((provider) => (
						<ConnectCalendarAccount
							key={provider}
							botId={botId}
							provider={provider}
						/>
					))}
					<Button
						type="button"
						variant="outline"
						size="sm"
						onClick={() => setAddingCalDav(true)}
					>
						<PlusIcon aria-hidden />
						{t("connectors.calendarSetup.caldav", "iCloud & CalDAV")}
					</Button>
				</div>
			)}
			{localOnly && (
				<p className="text-xs text-muted-foreground">
					{t(
						"connectors.calendarSetup.localOnly",
						"Google and Outlook calendars need a bot on your account. A bot on this computer reads CalDAV calendars such as iCloud or Nextcloud.",
					)}
				</p>
			)}
		</div>
	);
}
