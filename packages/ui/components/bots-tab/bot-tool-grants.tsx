"use client";

import { useTranslation } from "@flow-like/locales";
import { Loader2Icon, XIcon } from "lucide-react";
import { useBotToolGrants, useRevokeBotToolGrant } from "../../hooks/use-bots";
import type { IBot, IBotToolGrant } from "../../state/backend-state/bot-state";
import { Button } from "../ui/button";
import { useGrantLabel } from "./grant-labels";

const isStanding = (grant: IBotToolGrant) =>
	!grant.scope.startsWith("once:") && !grant.scope.startsWith("deny:");

/** What the bot may do without asking you, from your "Always allow" answers; each can be taken back. */
export function BotToolGrants({
	bot,
	name,
}: Readonly<{ bot: IBot; name: string }>) {
	const { t } = useTranslation("bots");
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
								<span className="min-w-0 flex-1 text-sm font-medium wrap-break-word">
									{label}
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
