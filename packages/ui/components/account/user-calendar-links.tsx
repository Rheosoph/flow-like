"use client";

import { useTranslation } from "@flow-like/locales";
import {
	CalendarDaysIcon,
	Loader2Icon,
	RefreshCwIcon,
	UnlinkIcon,
} from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { useAuth } from "react-oidc-context";
import { useHub } from "../../hooks/use-hub";
import { useUserAvailability } from "../../hooks/use-user-availability";
import { useBackend } from "../../state/backend-state";
import { useOAuthExecutionContext } from "../oauth/oauth-execution-provider";
import { Button } from "../ui/button";

const CALENDARS = [
	{
		id: "google",
		name: "Google Calendar",
		scopes: ["https://www.googleapis.com/auth/calendar.freebusy"],
	},
	{
		id: "microsoft",
		name: "Microsoft Outlook",
		scopes: ["Calendars.ReadBasic", "offline_access"],
	},
] as const;

/** A calendar connection supplies time ranges, independently of bot connector grants. */
export function UserCalendarLinks() {
	const { t } = useTranslation("bots");
	const state = useBackend().userState;
	const availability = useUserAvailability();
	const { hub } = useHub();
	const auth = useAuth();
	const oauth = useOAuthExecutionContext();
	const [pending, setPending] = useState<string>();
	const [error, setError] = useState<string>();
	const [notice, setNotice] = useState<string>();
	const current = useRef({
		state,
		subject: auth.user?.profile.sub,
		scope: availability.scope,
	});
	current.current = {
		state,
		subject: auth.user?.profile.sub,
		scope: availability.scope,
	};
	const mounted = useRef(true);
	useEffect(() => {
		mounted.current = true;
		return () => {
			mounted.current = false;
		};
	}, []);
	useEffect(() => {
		setPending(undefined);
		setError(undefined);
		setNotice(undefined);
	}, [state, auth.user?.profile.sub, availability.scope]);

	const links = availability.data?.calendar_links ?? [];
	const supported = Boolean(auth.isAuthenticated && state.linkCalendar);
	const providers = CALENDARS.filter(
		(provider) => hub?.oauth_providers?.[provider.id],
	);
	async function run(
		key: string,
		action: (active: () => boolean) => Promise<unknown>,
	) {
		if (pending) return;
		const captured = current.current;
		const active = () =>
			mounted.current &&
			captured.state === current.current.state &&
			captured.subject === current.current.subject &&
			captured.scope === current.current.scope;
		setPending(key);
		setError(undefined);
		setNotice(undefined);
		try {
			await action(active);
			if (active()) await availability.invalidate();
		} catch (cause) {
			if (active()) {
				if (
					cause instanceof Error &&
					cause.message === "OAuth authorization required"
				) {
					setNotice(
						t(
							"availability.calendarFinishSignIn",
							"Complete calendar sign-in, then select Link again.",
						),
					);
				} else {
					setError(
						cause instanceof Error
							? cause.message
							: t(
									"availability.calendarFailed",
									"The calendar could not be updated. Try again.",
								),
					);
				}
			}
		} finally {
			if (active()) setPending(undefined);
		}
	}

	return (
		<section
			className="space-y-3"
			aria-label={t("availability.calendars", "Linked calendars")}
		>
			<div className="flex items-center gap-2 text-sm font-medium">
				<CalendarDaysIcon className="size-4" aria-hidden />
				{t("availability.calendars", "Linked calendars")}
			</div>
			<p className="text-sm text-muted-foreground">
				{t(
					"availability.calendarPrivacy",
					"Your primary calendar sets your free/busy status. Other people see your status; bots can ask you to share busy time ranges. Event titles, attendees, and descriptions are not shared.",
				)}
			</p>
			{links.length > 0 && (
				<ul className="space-y-2">
					{links.map((link) => (
						<li
							key={link.id}
							className="flex items-center gap-2 rounded-md border p-2 text-sm"
						>
							<span className="min-w-0 flex-1 truncate">{link.label}</span>
							<Button
								type="button"
								variant="ghost"
								size="sm"
								disabled={Boolean(pending)}
								onClick={() =>
									void run(link.id, async (active) => {
										if (active()) await state.unlinkCalendar?.(link.id);
									})
								}
								aria-label={t(
									"availability.unlinkCalendar",
									"Unlink {{name}}",
									{ name: link.label },
								)}
							>
								<UnlinkIcon className="size-4" aria-hidden />
								{t("availability.unlink", "Unlink")}
							</Button>
						</li>
					))}
				</ul>
			)}
			<div className="flex flex-wrap gap-2">
				{supported &&
					providers.map((provider) => (
						<Button
							key={provider.id}
							type="button"
							variant="outline"
							size="sm"
							disabled={Boolean(pending)}
							onClick={() =>
								void run(provider.id, async (active) => {
									await oauth.withOAuthCheckFromPrerun(
										[
											{
												provider_id: provider.id,
												scopes: [...provider.scopes],
											},
										],
										async (tokens) => {
											if (!active()) return;
											const handle = tokens?.[provider.id]?.refresh_token;
											if (!handle?.startsWith("fl_oauth_"))
												throw new Error(
													t(
														"availability.calendarReconnect",
														"Reconnect this account through your hub before linking its calendar.",
													),
												);
											await state.linkCalendar?.({
												provider_id: provider.id,
												connection_handle: handle,
												label: provider.name,
											});
										},
									);
								})
							}
						>
							{pending === provider.id && (
								<Loader2Icon className="size-4 animate-spin" aria-hidden />
							)}
							{t("availability.linkCalendar", "Link {{name}}", {
								name: provider.name,
							})}
						</Button>
					))}
				{links.length > 0 && (
					<Button
						type="button"
						variant="outline"
						size="sm"
						disabled={Boolean(pending) || !state.refreshCalendars}
						onClick={() =>
							void run("refresh", async (active) => {
								if (active()) await state.refreshCalendars?.();
							})
						}
					>
						<RefreshCwIcon
							className={`size-4 ${pending === "refresh" ? "animate-spin" : ""}`}
							aria-hidden
						/>
						{t("availability.refreshCalendars", "Refresh calendars")}
					</Button>
				)}
			</div>
			{!supported && (
				<p className="text-xs text-muted-foreground">
					{t(
						"availability.calendarAccountRequired",
						"Sign in to link a calendar to your shared status.",
					)}
				</p>
			)}
			{supported && providers.length === 0 && (
				<p className="text-xs text-muted-foreground">
					{t(
						"availability.calendarProviderRequired",
						"Your hub needs Google or Microsoft sign-in configured to link calendars.",
					)}
				</p>
			)}
			{notice && (
				<p role="status" className="text-sm text-muted-foreground">
					{notice}
				</p>
			)}
			{(error || availability.data?.calendar_error) && (
				<p role="alert" className="text-sm text-destructive">
					{error || availability.data?.calendar_error}
				</p>
			)}
		</section>
	);
}
