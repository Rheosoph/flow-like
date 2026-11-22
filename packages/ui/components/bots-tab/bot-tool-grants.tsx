"use client";

import { useTranslation } from "@flow-like/locales";
import { Loader2Icon, XIcon } from "lucide-react";
import { useId } from "react";
import {
	botAutoApproves,
	useBotToolGrants,
	useRevokeBotToolGrant,
	useSetBotAutoApprove,
} from "../../hooks/use-bots";
import { isAutoApproveGrant } from "../../lib/tool-grant-scopes";
import type { IBot, IBotToolGrant } from "../../state/backend-state/bot-state";
import { Button } from "../ui/button";
import { Switch } from "../ui/switch";
import { useGrantLabel } from "./grant-labels";

const isStanding = (grant: IBotToolGrant) =>
	!grant.scope.startsWith("once:") &&
	!grant.scope.startsWith("deny:") &&
	!isAutoApproveGrant(grant);

/** The bot's auto approval: everything a grant could allow, without asking. */
function AutoApproveSwitch({
	bot,
	name,
	enabled,
}: Readonly<{ bot: IBot; name: string; enabled: boolean }>) {
	const { t } = useTranslation("bots");
	const id = useId();
	const { mutate, isPending } = useSetBotAutoApprove();
	return (
		<div className="flex items-start justify-between gap-3 rounded-lg bg-muted/60 px-3 py-2.5">
			<div className="flex min-w-0 flex-col gap-0.5">
				<label htmlFor={id} className="text-sm font-medium">
					{t("panel.grants.auto.title", "Approve automatically")}
				</label>
				<p className="text-xs text-muted-foreground">
					{t(
						"panel.grants.auto.description",
						"{{name}} acts without asking you first. Deleting data, changing settings and, after reading the web, sending data to a site or reading from one you haven't allowed still ask.",
						{ name },
					)}
				</p>
			</div>
			<Switch
				id={id}
				checked={enabled}
				disabled={isPending}
				onCheckedChange={(checked) =>
					mutate({ botId: bot.id, enabled: checked })
				}
			/>
		</div>
	);
}

/** What the bot may do without asking you, from your "Always allow" answers; each can be taken back. */
export function BotToolGrants({
	bot,
	name,
}: Readonly<{ bot: IBot; name: string }>) {
	const { t, i18n } = useTranslation("bots");
	const grants = useBotToolGrants(bot.id);
	const grantLabel = useGrantLabel(
		grants.data?.map((grant) => grant.scope) ?? [],
	);
	const { mutate: revoke, variables, isPending } = useRevokeBotToolGrant();
	if (!grants.data) return null;
	const standing = grants.data.filter(isStanding);
	return (
		<section className="flex flex-col gap-3" data-slot="bot-tool-grants">
			<h3 className="text-xs text-muted-foreground">
				{t("panel.grants.title", "Allowed actions")}
			</h3>
			<AutoApproveSwitch
				bot={bot}
				name={name}
				enabled={botAutoApproves(grants.data)}
			/>
			{standing.length === 0 ? (
				<p className="text-sm text-muted-foreground">
					{t(
						"panel.grants.none",
						"Nothing yet. {{name}} asks you before it acts, and you can tell it to stop asking.",
						{ name },
					)}
				</p>
			) : (
				<ul className="flex flex-col gap-1.5">
					{standing.map((grant) => {
						const revoking =
							isPending &&
							variables?.tool === grant.tool &&
							variables.scope === grant.scope;
						const label = grantLabel(grant.tool, grant.scope);
						const revokeLabel = t(
							"panel.grants.revoke",
							"Stop allowing: {{action}}",
							{
								action: label,
							},
						);
						return (
							<li
								key={`${grant.tool}\u0000${grant.scope}`}
								className="flex items-center gap-2.5 rounded-lg bg-muted/60 py-1.5 pr-1.5 pl-3"
							>
								<span className="flex min-w-0 flex-1 flex-col">
									<span className="text-sm font-medium wrap-break-word">
										{label}
									</span>
									{grant.expires_at && (
										<span className="text-xs text-muted-foreground">
											{t(
												"panel.grants.expires",
												"Lapses on {{date}} if unused",
												{
													date: new Date(grant.expires_at).toLocaleDateString(
														i18n.resolvedLanguage ?? i18n.language,
													),
												},
											)}
										</span>
									)}
								</span>
								<Button
									type="button"
									variant="ghost"
									size="icon"
									className="size-7 shrink-0 text-muted-foreground"
									disabled={revoking}
									aria-label={revokeLabel}
									title={revokeLabel}
									onClick={() =>
										revoke({
											botId: bot.id,
											tool: grant.tool,
											scope: grant.scope,
										})
									}
								>
									{revoking ? (
										<Loader2Icon className="animate-spin" aria-hidden />
									) : (
										<XIcon aria-hidden />
									)}
								</Button>
							</li>
						);
					})}
				</ul>
			)}
		</section>
	);
}
