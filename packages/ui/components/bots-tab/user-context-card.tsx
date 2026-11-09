"use client";

import { useTranslation } from "@flow-like/locales";
import { MapPinIcon, UserRoundIcon } from "lucide-react";
import Link from "next/link";
import { useCallback, useEffect, useId, useRef, useState } from "react";
import { useAuth } from "react-oidc-context";
import { useConversation, useSendInboxEntry } from "../../hooks/use-inbox";
import { useUserAvailability } from "../../hooks/use-user-availability";
import { useUserIdentity } from "../../hooks/use-user-lookup";
import {
	type UserContextInput,
	clearUserContextSessions,
	collectUserContext,
	currentUserContextActivity,
	defaultUserContextInput,
	deniedUserContext,
	grantUserContextSession,
	hasUserContextGrant,
	revokeUserContextSession,
	setUserContextAccount,
	trackUserContextCollection,
	userContextSessionInput,
} from "../../lib/bot-user-context";
import { useAssistantSurface } from "../../state/assistant-surface";
import { useAuthStatusStore } from "../../state/backend-state";
import type {
	IUserContextResponse,
	UserContextField,
} from "../../state/backend-state/inbox-state";
import {
	RequestCard,
	RequestCardFooter,
	RequestCardHeader,
	RequestMark,
} from "../interfaces/chat-default/request-card";
import { Button } from "../ui/button";
import { Input } from "../ui/input";
import type { UserContextCardData } from "./user-context-entry";

const FIELD_LABELS: Record<UserContextField, string> = {
	location: "Location",
	timezone: "Timezone and local time",
	locale: "Language and units",
	preferences: "Personal preferences",
	availability: "Availability",
	free_busy: "Calendar free/busy times",
	current_context: "Current Flow-Like activity",
	device: "Device and input capabilities",
};

// Keep logout revocation active even when no request card is mounted.
useAuthStatusStore.subscribe((state, previous) => {
	if (state.signedIn !== previous.signedIn) clearUserContextSessions();
});

export function UserContextCard({
	context,
}: Readonly<{ context: UserContextCardData }>) {
	const { request } = context;
	const formId = useId();
	const { t } = useTranslation("bots");
	const auth = useAuth();
	const account = auth.user?.profile.sub;
	const conversation = useConversation(request.conversation_id).data;
	const availability = useUserAvailability({
		local: conversation?.local,
		enabled: false,
	});
	const proposal = request.status_change;
	const bot = useUserIdentity(request.bot_id);
	const subject = useUserIdentity(request.subject_id);
	const send = useSendInboxEntry();
	const [input, setInput] = useState<UserContextInput>(
		() => userContextSessionInput(request) ?? defaultUserContextInput(),
	);
	const [sent, setSent] = useState<IUserContextResponse["decision"]>();
	const [busy, setBusy] = useState(false);
	const [error, setError] = useState<string>();
	const [unavailable, setUnavailable] = useState<UserContextField[]>([]);
	const [clock, setClock] = useState(Date.now);
	const [foreground, setForeground] = useState(
		() =>
			typeof document !== "undefined" && document.visibilityState === "visible",
	);
	const [granted, setGranted] = useState(() => hasUserContextGrant(request));
	const controller = useRef<AbortController | undefined>(undefined);
	const attempted = useRef(false);
	const answer = context.answer ?? sent;
	const subjectId = conversation?.local
		? "local"
		: auth.isAuthenticated
			? account
			: undefined;
	const mine = subjectId === request.subject_id;
	const members = conversation?.members
		.map((member) => member.principal_id)
		.sort();
	const audienceMatches =
		conversation?.encryption === "plain" &&
		conversation?.kind === request.audience &&
		JSON.stringify(members) ===
			JSON.stringify([...request.audience_member_ids].sort());
	const expired = clock >= request.expires_at;
	const disabled =
		busy || !mine || !audienceMatches || expired || Boolean(answer);
	const authorization = JSON.stringify([
		availability.scope,
		subjectId,
		members,
		auth.isAuthenticated,
		conversation?.encryption,
		conversation?.kind,
		request,
	]);
	const currentAuthorization = useRef(authorization);
	currentAuthorization.current = authorization;
	const currentRequest = useRef(request);
	currentRequest.current = request;

	useEffect(() => {
		setUserContextAccount(`${availability.scope}:${account ?? "signedout"}`);
		setGranted(hasUserContextGrant(request));
	}, [account, availability.scope, request]);

	useEffect(() => {
		const scope = authorization;
		return () => {
			if (scope !== currentAuthorization.current) {
				controller.current?.abort();
				setBusy(false);
			}
		};
	}, [authorization]);

	useEffect(() => {
		const visibility = () =>
			setForeground(document.visibilityState === "visible");
		document.addEventListener("visibilitychange", visibility);
		const timer = setInterval(() => {
			setClock(Date.now());
			setGranted(hasUserContextGrant(currentRequest.current));
		}, 1_000);
		return () => {
			document.removeEventListener("visibilitychange", visibility);
			clearInterval(timer);
			controller.current?.abort();
		};
	}, []);

	const respond = useCallback(
		async (decision: IUserContextResponse["decision"], automatic = false) => {
			if (disabled || !subjectId) return;
			if (proposal && (automatic || decision === "session")) return;
			if (automatic && !hasUserContextGrant(request)) return;
			setBusy(true);
			setError(undefined);
			const abort = new AbortController();
			controller.current = abort;
			const untrack = trackUserContextCollection(request, abort);
			try {
				const response =
					decision === "deny"
						? deniedUserContext(request)
						: await collectUserContext(request, input, {
								subjectId,
								decision,
								activity: currentUserContextActivity(
									useAssistantSurface.getState(),
								),
								signal: abort.signal,
								availability: availability.load,
							});
				if (
					abort.signal.aborted ||
					authorization !== currentAuthorization.current ||
					(automatic && !hasUserContextGrant(request))
				)
					return;
				await send.mutateAsync({
					conversationId: request.conversation_id,
					request: {
						message_id: `uctx_reply_${request.id}`,
						kind: "message",
						envelope: {
							v: 1,
							enc: "none",
							body: {
								text:
									decision === "deny"
										? "Context sharing declined."
										: proposal
											? "Status change approved."
											: "Context shared for this request.",
								reply_to: request.id,
								mentions: [request.bot_id],
								user_context_response: response,
							},
						},
					},
				});
				if (proposal && decision === "once" && conversation?.local) {
					const until = response.values.availability?.until;
					if (typeof until === "number")
						availability.applyLocalStatus(proposal.status, until);
				}
				if (
					abort.signal.aborted ||
					authorization !== currentAuthorization.current ||
					(automatic && !hasUserContextGrant(request))
				)
					return;
				if (proposal && decision === "once") {
					void availability.invalidate().catch(() => {});
				}
				setUnavailable(Object.keys(response.unavailable) as UserContextField[]);
				if (decision === "session") {
					if (!automatic) grantUserContextSession(request, input);
					setGranted(true);
				}
				if (decision === "deny") {
					revokeUserContextSession(request);
					setGranted(false);
				}
				setSent(decision);
			} catch {
				if (!abort.signal.aborted)
					setError(
						t(
							"context.failed",
							"Couldn't share your context. Check the request and try again.",
						),
					);
			} finally {
				untrack();
				setBusy(false);
			}
		},
		[
			disabled,
			subjectId,
			request,
			input,
			send.mutateAsync,
			t,
			authorization,
			proposal,
			availability.load,
			availability.applyLocalStatus,
			availability.invalidate,
			conversation?.local,
		],
	);

	useEffect(() => {
		if (
			!proposal &&
			!disabled &&
			granted &&
			hasUserContextGrant(request) &&
			!attempted.current &&
			foreground
		) {
			attempted.current = true;
			void respond("session", true);
		}
	}, [disabled, granted, respond, request, foreground, proposal]);

	const change = <Key extends keyof UserContextInput>(
		key: Key,
		value: UserContextInput[Key],
	) => setInput((current) => ({ ...current, [key]: value }));
	const selectClass = "h-9 w-full rounded-md border bg-background px-2 text-sm";
	const field = (name: UserContextField) => request.fields.includes(name);
	const tone = answer === "deny" ? "muted" : answer ? "good" : "warning";
	return (
		<RequestCard
			tone={tone}
			live={!answer && !expired}
			data-slot="user-context-card"
			className="max-w-prose"
		>
			<RequestCardHeader
				dense
				tone={tone}
				live={!answer && !expired}
				mark={
					<RequestMark
						icon={field("location") ? MapPinIcon : UserRoundIcon}
						tone={tone}
					/>
				}
				eyebrow={t("context.label", "User context request")}
				title={t(
					proposal ? "context.statusTitle" : "context.title",
					proposal
						? "{{bot}} asks {{person}} to confirm a status change"
						: "{{bot}} asks {{person}} to share context",
					{
						bot: bot.label,
						person: mine ? t("context.you", "you") : subject.label,
					},
				)}
				description={
					<div className="space-y-2">
						<p>{request.purpose}</p>
						{proposal && (
							<div className="space-y-1 rounded-md border p-2">
								<p className="font-medium">
									{t(
										"context.statusProposal",
										"Set status to {{status}} for {{minutes}} minutes",
										{
											status: t(
												`availability.status.${proposal.status}`,
												proposal.status,
											),
											minutes: proposal.duration_minutes,
										},
									)}
								</p>
								<p>
									{conversation?.local
										? t(
												"context.localStatus",
												"This changes status for this app session and can pause foreground alerts. Your shared status and push notifications on other devices stay as they are.",
											)
										: t(
												"context.publicStatus",
												"Other people will see this status across Flow-Like. It replaces your current manual or calendar status until the timer ends.",
											)}
								</p>
								<p>
									{proposal.status === "focus"
										? t(
												"context.focusEffect",
												"Focus pauses routine notifications. Your mention and call exceptions still apply.",
											)
										: t(
												"context.statusEffect",
												"This status does not pause notifications.",
											)}
								</p>
							</div>
						)}
						<p>
							{request.fields
								.map((name) => t(`context.fields.${name}`, FIELD_LABELS[name]))
								.join(", ")}
						</p>
						<p className="font-medium">
							{request.audience === "group"
								? t(
										"context.groupAudience",
										"Shared with everyone in this group and usable in the bot's group replies.",
									)
								: t(
										"context.directAudience",
										"Shared in this direct conversation and usable in the bot's replies.",
									)}
						</p>
						<p>
							{t(
								"context.device",
								"Source: this device ({{platform}}). Context is collected only while this app is open.",
								{
									platform:
										typeof window !== "undefined" &&
										"__TAURI_INTERNALS__" in window
											? t("context.native", "Flow-Like app")
											: t("context.browser", "browser"),
								},
							)}
						</p>
						<p>
							{t(
								"context.retention",
								"This snapshot is current for up to five minutes and is used for this answer. Shared messages remain in this conversation.",
							)}
						</p>
						{answer && answer !== "deny" && unavailable.length > 0 && (
							<p>
								{t("context.unavailable", "Unavailable: {{fields}}", {
									fields: unavailable
										.map((name) =>
											t(`context.fields.${name}`, FIELD_LABELS[name]),
										)
										.join(", "),
								})}
							</p>
						)}
						{answer && (
							<output className="block">
								{answer === "deny"
									? t("context.declined", "Sharing declined")
									: proposal
										? t("context.statusApplied", "Status change applied")
										: t("context.shared", "Context shared for this request")}
							</output>
						)}
						{!answer && expired && (
							<output className="block">
								{t("context.expired", "This request has expired.")}
							</output>
						)}
					</div>
				}
			/>
			{!answer && !expired && mine && (
				<div className="space-y-3 px-4 pb-4 text-sm">
					{field("location") && (
						<>
							<label className="block space-y-1">
								<span>{t("context.locationSource", "Location source")}</span>
								<select
									className={selectClass}
									value={input.locationMode}
									disabled={busy}
									onChange={(event) =>
										change(
											"locationMode",
											event.target.value as "device" | "manual",
										)
									}
								>
									<option value="device">
										{request.precision === "precise"
											? t("context.precise", "Precise device location")
											: t(
													"context.approximate",
													"Approximate device location (at least 1.5 km)",
												)}
									</option>
									<option value="manual">
										{t("context.manualLocation", "Enter a city or place")}
									</option>
								</select>
							</label>
							{input.locationMode === "manual" && (
								<Input
									aria-label={t("context.place", "City or place")}
									value={input.place}
									maxLength={200}
									disabled={busy}
									onChange={(event) => change("place", event.target.value)}
								/>
							)}
						</>
					)}
					{field("timezone") && (
						<label className="block space-y-1" htmlFor={`${formId}-timezone`}>
							<span>{t("context.timezone", "Timezone")}</span>
							<Input
								id={`${formId}-timezone`}
								value={input.timezone}
								disabled={busy}
								onChange={(event) => change("timezone", event.target.value)}
								placeholder="Europe/Berlin"
							/>
						</label>
					)}
					{field("locale") && (
						<div className="grid gap-3 sm:grid-cols-2">
							<label className="space-y-1" htmlFor={`${formId}-language`}>
								<span>{t("context.language", "Language")}</span>
								<Input
									id={`${formId}-language`}
									value={input.language}
									maxLength={100}
									disabled={busy}
									onChange={(event) => change("language", event.target.value)}
								/>
							</label>
							<label className="space-y-1">
								<span>{t("context.units", "Units")}</span>
								<select
									className={selectClass}
									value={input.units}
									disabled={busy}
									onChange={(event) =>
										change(
											"units",
											event.target.value as UserContextInput["units"],
										)
									}
								>
									<option value="unspecified">
										{t("context.unspecified", "No preference")}
									</option>
									<option value="metric">
										{t("context.metric", "Metric")}
									</option>
									<option value="imperial">
										{t("context.imperial", "Imperial")}
									</option>
								</select>
							</label>
						</div>
					)}
					{field("preferences") && (
						<div className="grid gap-3 sm:grid-cols-2">
							<label className="space-y-1" htmlFor={`${formId}-name`}>
								<span>{t("context.name", "Preferred name (optional)")}</span>
								<Input
									id={`${formId}-name`}
									value={input.displayName}
									maxLength={100}
									disabled={busy}
									onChange={(event) =>
										change("displayName", event.target.value)
									}
								/>
							</label>
							<label className="space-y-1">
								<span>{t("context.style", "Response style")}</span>
								<select
									className={selectClass}
									value={input.responseStyle}
									disabled={busy}
									onChange={(event) =>
										change(
											"responseStyle",
											event.target.value as UserContextInput["responseStyle"],
										)
									}
								>
									<option value="default">
										{t("context.unspecified", "No preference")}
									</option>
									<option value="concise">
										{t("context.concise", "Concise")}
									</option>
									<option value="detailed">
										{t("context.detailed", "Detailed")}
									</option>
								</select>
							</label>
						</div>
					)}
					{field("availability") && !proposal && (
						<>
							<label className="block space-y-1">
								<span>
									{t("context.availabilitySource", "Availability source")}
								</span>
								<select
									className={selectClass}
									value={input.availabilityMode ?? "account"}
									disabled={busy}
									onChange={(event) =>
										change(
											"availabilityMode",
											event.target.value as "account" | "manual",
										)
									}
								>
									<option value="account">
										{t(
											"context.accountAvailability",
											"My current Flow-Like status",
										)}
									</option>
									<option value="manual">
										{t(
											"context.manualAvailability",
											"Enter availability for this answer",
										)}
									</option>
								</select>
							</label>
							{input.availabilityMode === "manual" && (
								<>
									<label className="block space-y-1">
										<span>
											{t("context.availability", "Your availability")}
										</span>
										<select
											className={selectClass}
											value={input.availability}
											disabled={busy}
											onChange={(event) =>
												change(
													"availability",
													event.target
														.value as UserContextInput["availability"],
												)
											}
										>
											<option value="unspecified">
												{t("context.unspecified", "No preference")}
											</option>
											<option value="available">
												{t("context.available", "Available")}
											</option>
											<option value="busy">{t("context.busy", "Busy")}</option>
											<option value="focus">
												{t("availability.status.focus", "Focus")}
											</option>
											<option value="away">{t("context.away", "Away")}</option>
										</select>
									</label>
									<Input
										aria-label={t(
											"context.availabilityNote",
											"Availability note (optional)",
										)}
										value={input.availabilityNote}
										maxLength={300}
										disabled={busy}
										onChange={(event) =>
											change("availabilityNote", event.target.value)
										}
									/>
								</>
							)}
						</>
					)}
					{field("free_busy") && (
						<p>
							{t(
								"context.freeBusyHint",
								"Share busy time ranges from your linked calendars for up to the next 24 hours. Event details are not included.",
							)}{" "}
							<Link href="/account#availability" className="underline">
								{t(
									"context.manageAvailability",
									"Manage availability and calendars",
								)}
							</Link>
						</p>
					)}
					{field("current_context") && (
						<label className="block space-y-1" htmlFor={`${formId}-activity`}>
							<span>
								{t(
									"context.activity",
									"Current activity (leave empty to share the open Flow-Like surface)",
								)}
							</span>
							<Input
								id={`${formId}-activity`}
								value={input.activity}
								maxLength={500}
								disabled={busy}
								onChange={(event) => change("activity", event.target.value)}
							/>
							<span className="text-xs text-muted-foreground">
								{t(
									"context.activityHint",
									"Only surface and selection identifiers are shared. Document contents are not included.",
								)}
							</span>
						</label>
					)}
					{!proposal && (
						<p className="text-xs text-muted-foreground">
							{t(
								"context.sessionHint",
								"Session access lasts up to 30 minutes on this device, for this bot, purpose, fields and conversation. You can revoke it here.",
							)}
						</p>
					)}
				</div>
			)}
			{((!answer && !expired && mine) || granted) && (
				<RequestCardFooter
					start={
						error ? (
							<p role="alert" className="text-critical">
								{error}
							</p>
						) : !audienceMatches && !answer ? (
							<p>
								{t(
									"context.changed",
									"The conversation changed. Ask the bot for a new request.",
								)}
							</p>
						) : undefined
					}
				>
					{!answer && !expired && mine && (
						<>
							<Button
								variant="ghost"
								size="sm"
								disabled={disabled}
								onClick={() => void respond("deny")}
							>
								{t("context.decline", "Decline")}
							</Button>
							<Button
								variant="outline"
								size="sm"
								disabled={disabled}
								onClick={() => void respond("once")}
							>
								{proposal
									? t("context.applyStatus", "Apply status change")
									: t("context.once", "Share once")}
							</Button>
							{!proposal && (
								<Button
									size="sm"
									disabled={disabled}
									onClick={() => void respond("session")}
								>
									{busy
										? t("context.sharing", "Sharing…")
										: t("context.session", "Allow for this session")}
								</Button>
							)}
						</>
					)}
					{granted && mine && (
						<Button
							variant="ghost"
							size="sm"
							onClick={() => {
								revokeUserContextSession(request);
								setGranted(false);
							}}
						>
							{t("context.revoke", "Revoke session access")}
						</Button>
					)}
				</RequestCardFooter>
			)}
		</RequestCard>
	);
}
