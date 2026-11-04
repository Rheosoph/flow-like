"use client";

import { useTranslation } from "@flow-like/locales";
import {
	CalendarDaysIcon,
	CopyIcon,
	DownloadIcon,
	PlusIcon,
} from "lucide-react";
import { type FormEvent, useCallback, useEffect, useId, useState } from "react";
import {
	downloadMeetingCalendar,
	localDateTime,
	meetingError,
	meetingJoinUrl,
	meetingPhase,
	scheduleRange,
} from "../../lib/meetings/helpers";
import type { Meeting, MeetingClient } from "../../lib/meetings/types";
import { Button } from "../ui/button";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogHeader,
	DialogTitle,
	DialogTrigger,
} from "../ui/dialog";
import { Input } from "../ui/input";
import { Label } from "../ui/label";
import { Switch } from "../ui/switch";

interface RoomMeetingsProps {
	conversationId: string;
	client: MeetingClient;
	me: string;
	joinBaseUrl: string;
	onJoin?: (meetingId: string) => void;
}

export function RoomMeetings(props: RoomMeetingsProps) {
	return <RoomMeetingsContent key={props.conversationId} {...props} />;
}

function RoomMeetingsContent({
	conversationId,
	client,
	me,
	joinBaseUrl,
	onJoin,
}: RoomMeetingsProps) {
	const { t } = useTranslation("bots");
	const [open, setOpen] = useState(false);
	const [meetings, setMeetings] = useState<Meeting[]>([]);
	const [scheduling, setScheduling] = useState(false);
	const [busy, setBusy] = useState(false);
	const [error, setError] = useState<string>();
	const [share, setShare] = useState<{ id: string; url: string }>();
	const [copied, setCopied] = useState(false);
	const [now, setNow] = useState(Date.now);
	const refresh = useCallback(async () => {
		try {
			setMeetings(await client.list(conversationId));
			setError(undefined);
		} catch (cause) {
			setError(meetingError(cause));
		}
	}, [client, conversationId]);
	useEffect(() => {
		if (!open) return;
		void refresh();
		const timer = setInterval(() => {
			setNow(Date.now());
			void refresh();
		}, 15_000);
		return () => clearInterval(timer);
	}, [open, refresh]);
	const copy = async (url: string) => {
		setCopied(false);
		try {
			await navigator.clipboard.writeText(url);
			setCopied(true);
		} catch {
			/* The selectable link remains available when clipboard access is blocked. */
		}
	};
	const showInvite = async (meeting: Meeting, rotate = false) => {
		setBusy(true);
		try {
			const { invite_token } = await (rotate
				? client.rotateInvite(meeting.id)
				: client.invite(meeting.id));
			const url = meetingJoinUrl(joinBaseUrl, meeting.id, invite_token);
			setShare({ id: meeting.id, url });
			await copy(url);
		} catch (cause) {
			setError(meetingError(cause));
		} finally {
			setBusy(false);
		}
	};
	return (
		<Dialog open={open} onOpenChange={setOpen}>
			<DialogTrigger asChild>
				<Button
					variant="ghost"
					size="icon"
					aria-label={t("crew.meetings.open", "Meetings")}
					title={t("crew.meetings.open", "Meetings")}
				>
					<CalendarDaysIcon aria-hidden />
				</Button>
			</DialogTrigger>
			<DialogContent className="sm:max-w-xl">
				<DialogHeader>
					<DialogTitle>{t("crew.meetings.open", "Meetings")}</DialogTitle>
					<DialogDescription>
						{t(
							"crew.meetings.description",
							"Schedule a call for this room. Members can join, and invited guests wait for the host to admit them.",
						)}
					</DialogDescription>
				</DialogHeader>
				{error && (
					<p role="alert" className="text-sm text-destructive">
						{error}
					</p>
				)}
				{scheduling ? (
					<ScheduleMeeting
						client={client}
						conversationId={conversationId}
						onCancel={() => setScheduling(false)}
						onCreated={(meeting, token) => {
							setMeetings((previous) => [...previous, meeting]);
							setScheduling(false);
							setCopied(false);
							setShare({
								id: meeting.id,
								url: meetingJoinUrl(
									joinBaseUrl,
									meeting.id,
									meeting.guests_allowed ? token : undefined,
								),
							});
						}}
					/>
				) : (
					<Button className="self-start" onClick={() => setScheduling(true)}>
						<PlusIcon aria-hidden />
						{t("crew.meetings.schedule", "Schedule meeting")}
					</Button>
				)}
				{share && (
					<div className="space-y-2 rounded-lg border bg-muted/30 p-3">
						<Label htmlFor="meeting-share-link">
							{t("crew.meetings.shareLink", "Meeting link")}
						</Label>
						<div className="flex gap-2">
							<Input
								id="meeting-share-link"
								readOnly
								value={share.url}
								onFocus={(event) => event.target.select()}
							/>
							<Button
								size="icon"
								variant="outline"
								aria-label={t("crew.meetings.copy", "Copy link")}
								onClick={() => void copy(share.url)}
							>
								<CopyIcon aria-hidden />
							</Button>
						</div>
						{copied && (
							<output className="text-xs text-muted-foreground">
								{t("crew.meetings.copied", "Link copied")}
							</output>
						)}
					</div>
				)}
				<ul className="space-y-3">
					{meetings
						.slice()
						.sort((a, b) => Date.parse(a.starts_at) - Date.parse(b.starts_at))
						.map((meeting) => {
							const phase = meetingPhase(meeting, now);
							const host = meeting.host_id === me;
							return (
								<li
									key={meeting.id}
									className="space-y-3 rounded-lg border p-3"
								>
									<div>
										<p className="break-words font-medium">{meeting.title}</p>
										<p className="text-sm text-muted-foreground">
											{new Date(meeting.starts_at).toLocaleString()} –{" "}
											{new Date(meeting.ends_at).toLocaleTimeString([], {
												hour: "2-digit",
												minute: "2-digit",
											})}
										</p>
									</div>
									<div className="flex flex-wrap items-center gap-2">
										{phase === "ended" ? (
											<span className="text-xs text-muted-foreground">
												{t("crew.meetings.ended", "Meeting ended")}
											</span>
										) : (
											<>
												<Button
													asChild={!onJoin}
													size="sm"
													variant={phase === "open" ? "default" : "outline"}
													onClick={
														onJoin
															? () => {
																	setOpen(false);
																	onJoin(meeting.id);
																}
															: undefined
													}
												>
													{onJoin ? (
														phase === "open" ? (
															t("crew.meetings.join", "Join meeting")
														) : (
															t("crew.meetings.view", "View meeting")
														)
													) : (
														<a
															href={meetingJoinUrl(joinBaseUrl, meeting.id)}
															target="_blank"
															rel="noreferrer"
														>
															{phase === "open"
																? t("crew.meetings.join", "Join meeting")
																: t("crew.meetings.view", "View meeting")}
														</a>
													)}
												</Button>
												{host && meeting.guests_allowed && (
													<Button
														size="sm"
														variant="outline"
														disabled={busy}
														onClick={() => void showInvite(meeting)}
													>
														{t("crew.meetings.inviteGuests", "Invite guests")}
													</Button>
												)}
												<Button
													size="sm"
													variant="ghost"
													aria-label={t(
														"crew.meetings.calendar",
														"Add to calendar",
													)}
													title={t("crew.meetings.calendar", "Add to calendar")}
													onClick={() =>
														downloadMeetingCalendar(
															meeting,
															share?.id === meeting.id
																? share.url
																: meetingJoinUrl(joinBaseUrl, meeting.id),
														)
													}
												>
													<DownloadIcon aria-hidden />
												</Button>
												{host && (
													<Button
														size="sm"
														variant="ghost"
														disabled={busy}
														onClick={async () => {
															setBusy(true);
															try {
																await client.end(meeting.id);
																if (share?.id === meeting.id)
																	setShare(undefined);
																await refresh();
															} catch (cause) {
																setError(meetingError(cause));
															} finally {
																setBusy(false);
															}
														}}
													>
														{phase === "upcoming"
															? t("crew.meetings.cancel", "Cancel meeting")
															: t("crew.meetings.end", "End meeting")}
													</Button>
												)}
											</>
										)}
									</div>
									{host &&
										meeting.guests_allowed &&
										share?.id === meeting.id &&
										phase !== "ended" && (
											<Button
												size="sm"
												variant="link"
												disabled={busy}
												onClick={() => void showInvite(meeting, true)}
											>
												{t("crew.meetings.replaceLink", "Replace guest link")}
											</Button>
										)}
								</li>
							);
						})}
				</ul>
				{meetings.length === 0 && !scheduling && (
					<p className="text-sm text-muted-foreground">
						{t(
							"crew.meetings.empty",
							"No meetings scheduled for this room yet.",
						)}
					</p>
				)}
			</DialogContent>
		</Dialog>
	);
}

export function ScheduleMeeting({
	client,
	conversationId,
	onCancel,
	onCreated,
}: {
	client: MeetingClient;
	conversationId: string;
	onCancel: () => void;
	onCreated: (meeting: Meeting, token: string) => void;
}) {
	const { t } = useTranslation("bots");
	const id = useId();
	const [title, setTitle] = useState("");
	const [start, setStart] = useState(() =>
		localDateTime(new Date(Math.ceil(Date.now() / 900_000) * 900_000)),
	);
	const [end, setEnd] = useState(() =>
		localDateTime(
			new Date(Math.ceil(Date.now() / 900_000) * 900_000 + 3600_000),
		),
	);
	const [guests, setGuests] = useState(true);
	const [busy, setBusy] = useState(false);
	const [error, setError] = useState<string>();
	const submit = async (event: FormEvent) => {
		event.preventDefault();
		if (busy) return;
		setError(undefined);
		setBusy(true);
		try {
			if (!title.trim())
				throw new Error(
					t("crew.meetings.titleRequired", "Enter a meeting title."),
				);
			const result = await client.create({
				conversation_id: conversationId,
				title: title.trim(),
				...scheduleRange(start, end),
				guests_allowed: guests,
			});
			onCreated(result.meeting, result.invite_token);
		} catch (cause) {
			setError(meetingError(cause));
		} finally {
			setBusy(false);
		}
	};
	return (
		<form
			onSubmit={submit}
			className="space-y-4 rounded-lg border p-4"
			aria-label={t("crew.meetings.schedule", "Schedule meeting")}
		>
			<div className="space-y-2">
				<Label htmlFor={`${id}-title`}>
					{t("crew.meetings.title", "Title")}
				</Label>
				<Input
					id={`${id}-title`}
					value={title}
					onChange={(event) => setTitle(event.target.value)}
					maxLength={160}
					required
					autoFocus
				/>
			</div>
			<div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
				<div className="space-y-2">
					<Label htmlFor={`${id}-start`}>
						{t("crew.meetings.start", "Starts")}
					</Label>
					<Input
						id={`${id}-start`}
						type="datetime-local"
						value={start}
						onChange={(event) => setStart(event.target.value)}
						required
					/>
				</div>
				<div className="space-y-2">
					<Label htmlFor={`${id}-end`}>
						{t("crew.meetings.finish", "Ends")}
					</Label>
					<Input
						id={`${id}-end`}
						type="datetime-local"
						value={end}
						onChange={(event) => setEnd(event.target.value)}
						required
					/>
				</div>
			</div>
			<p className="text-xs text-muted-foreground">
				{t(
					"crew.meetings.timeZone",
					"Times use your time zone: {{zone}}. Joining opens 15 minutes before the start.",
					{ zone: Intl.DateTimeFormat().resolvedOptions().timeZone },
				)}
			</p>
			<div className="flex items-center gap-2">
				<Switch
					id={`${id}-guests`}
					checked={guests}
					onCheckedChange={setGuests}
				/>
				<Label htmlFor={`${id}-guests`}>
					{t(
						"crew.meetings.allowGuests",
						"Allow guests with an invitation link",
					)}
				</Label>
			</div>
			{error && (
				<p role="alert" className="text-sm text-destructive">
					{error}
				</p>
			)}
			<div className="flex justify-end gap-2">
				<Button
					type="button"
					variant="ghost"
					disabled={busy}
					onClick={onCancel}
				>
					{t("crew.meetings.back", "Back")}
				</Button>
				<Button type="submit" disabled={busy}>
					{busy
						? t("crew.meetings.saving", "Scheduling…")
						: t("crew.meetings.schedule", "Schedule meeting")}
				</Button>
			</div>
		</form>
	);
}
