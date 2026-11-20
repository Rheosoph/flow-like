"use client";

import { useTranslation } from "@flow-like/locales";
import {
	CalendarPlusIcon,
	CopyIcon,
	ExternalLinkIcon,
	LinkIcon,
} from "lucide-react";
import { useState } from "react";
import { toast } from "sonner";
import {
	useBotCalendarFeed,
	useDeleteBotCalendarFeed,
	useRotateBotCalendarFeed,
	useUpdateBotCalendarFeed,
} from "../../hooks/use-bots";
import { getErrorMessage } from "../../lib/error-message";
import { openExternalUrl } from "../../lib/open-external";
import type {
	IBot,
	ICalendarFeed,
	IUpdateCalendarFeed,
} from "../../state/backend-state/bot-state";
import { Button } from "../ui/button";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogFooter,
	DialogHeader,
	DialogTitle,
} from "../ui/dialog";
import { Input } from "../ui/input";
import { SettingSwitch } from "./bot-calendar-dialogs";
import {
	FEED_DRAFT,
	type FeedFlag,
	type FeedTargetId,
	feedCalendarName,
	feedFlags,
	feedTargets,
} from "./bot-calendar-feed-model";

type Confirming = "rotate" | "off" | null;

function useFlagLabels(): Readonly<Record<FeedFlag, string>> {
	const { t } = useTranslation("bots");
	return {
		include_events: t("calendar.feed.events", "Events"),
		include_tasks: t("calendar.feed.tasks", "Tasks"),
		include_memories: t("calendar.feed.memories", "Memories"),
		include_notes: t("calendar.feed.notes", "Include notes"),
	};
}

function useTargetLabels(): Readonly<Record<FeedTargetId, string>> {
	const { t } = useTranslation("bots");
	return {
		apple: t("calendar.feed.apple", "Apple Calendar"),
		google: t("calendar.feed.google", "Google Calendar"),
		outlook: t("calendar.feed.outlook", "Outlook.com"),
		microsoft: t("calendar.feed.microsoft", "Microsoft 365"),
	};
}

function FlagSwitches({
	flags,
	values,
	disabled,
	onChange,
}: Readonly<{
	flags: readonly FeedFlag[];
	values: Readonly<Required<IUpdateCalendarFeed>>;
	disabled?: boolean;
	onChange: (flag: FeedFlag, value: boolean) => void;
}>) {
	const { t } = useTranslation("bots");
	const labels = useFlagLabels();
	const sensitive = values.include_memories || values.include_notes;
	return (
		<div className="flex flex-col gap-2">
			{flags.map((flag) => (
				<SettingSwitch
					key={flag}
					label={labels[flag]}
					checked={values[flag]}
					disabled={disabled}
					onChange={(value) => onChange(flag, value)}
				/>
			))}
			{sensitive && (
				<p className="text-xs text-muted-foreground">
					{t(
						"calendar.feed.privacy",
						"The link is private, but anyone who has it, and services like Google that fetch it, can read what it shows.",
					)}
				</p>
			)}
		</div>
	);
}

function valuesOf(feed: ICalendarFeed): Required<IUpdateCalendarFeed> {
	return {
		include_events: feed.include_events,
		include_tasks: feed.include_tasks,
		include_memories: feed.include_memories,
		include_notes: feed.include_notes,
	};
}

function CreateFeed({
	name,
	flags,
	busy,
	onCreate,
}: Readonly<{
	name: string;
	flags: readonly FeedFlag[];
	busy: boolean;
	onCreate: (values: Required<IUpdateCalendarFeed>) => void;
}>) {
	const { t } = useTranslation("bots");
	const [values, setValues] = useState(FEED_DRAFT);
	const offered = {
		...values,
		include_events: values.include_events && flags.includes("include_events"),
		include_tasks: values.include_tasks && flags.includes("include_tasks"),
	};
	return (
		<>
			<p className="text-sm text-muted-foreground">
				{t(
					"calendar.feed.intro",
					"Show {{name}}'s calendar in Apple Calendar, Google Calendar, Outlook or any other calendar app. It is read only and keeps itself up to date.",
					{ name },
				)}
			</p>
			<FlagSwitches
				flags={flags}
				values={offered}
				disabled={busy}
				onChange={(flag, value) =>
					setValues((current) => ({ ...current, [flag]: value }))
				}
			/>
			<DialogFooter>
				<Button type="button" disabled={busy} onClick={() => onCreate(offered)}>
					<LinkIcon className="size-4" aria-hidden />
					{t("calendar.feed.create", "Create link")}
				</Button>
			</DialogFooter>
		</>
	);
}

function openTarget(href: string, onFailed: () => void) {
	void openExternalUrl(href).catch(onFailed);
}

function ExistingFeed({
	feed,
	name,
	flags,
	busy,
	confirming,
	onConfirming,
	onChange,
	onRotate,
	onDelete,
}: Readonly<{
	feed: ICalendarFeed;
	name: string;
	flags: readonly FeedFlag[];
	busy: boolean;
	confirming: Confirming;
	onConfirming: (next: Confirming) => void;
	onChange: (update: IUpdateCalendarFeed) => void;
	onRotate: () => void;
	onDelete: () => void;
}>) {
	const { t } = useTranslation("bots");
	const targetLabels = useTargetLabels();
	const copy = async () => {
		try {
			await navigator.clipboard.writeText(feed.url);
			toast.success(t("calendar.feed.copied", "Link copied"));
		} catch (error) {
			toast.error(getErrorMessage(error));
		}
	};
	if (confirming) {
		const rotate = confirming === "rotate";
		return (
			<>
				<p role="alert" className="text-sm font-medium">
					{rotate
						? t("calendar.feed.rotateTitle", "Replace the link?")
						: t("calendar.feed.offTitle", "Turn off the calendar link?")}
				</p>
				<p className="text-sm text-muted-foreground">
					{rotate
						? t(
								"calendar.feed.rotateBody",
								"Calendars subscribed to the current link stop updating. Subscribe again with the new link.",
							)
						: t(
								"calendar.feed.offBody",
								"Calendars subscribed to this link stop updating. You can create a new link later.",
							)}
				</p>
				<DialogFooter>
					<Button
						type="button"
						variant="outline"
						disabled={busy}
						onClick={() => onConfirming(null)}
					>
						{t("crew.cancel", "Cancel")}
					</Button>
					<Button
						type="button"
						variant={rotate ? "default" : "destructive"}
						disabled={busy}
						onClick={rotate ? onRotate : onDelete}
					>
						{rotate
							? t("calendar.feed.rotateConfirm", "Replace link")
							: t("calendar.feed.offConfirm", "Turn off")}
					</Button>
				</DialogFooter>
			</>
		);
	}
	return (
		<>
			<div className="grid grid-cols-2 gap-2">
				{feedTargets(feed, feedCalendarName(name)).map((target) => (
					<Button
						key={target.id}
						type="button"
						variant="outline"
						size="sm"
						className="justify-start gap-2"
						onClick={() =>
							openTarget(target.href, () => {
								void copy();
								toast.error(
									t(
										"calendar.feed.openFailed",
										"Couldn't open {{app}}; the link is copied instead.",
										{ app: targetLabels[target.id] },
									),
								);
							})
						}
					>
						<ExternalLinkIcon className="size-3.5" aria-hidden />
						{targetLabels[target.id]}
					</Button>
				))}
			</div>
			<p className="text-xs text-muted-foreground">
				{t(
					"calendar.feed.slow",
					"Google and Outlook refresh subscribed calendars only every few hours.",
				)}
			</p>
			<div className="flex items-center gap-1.5">
				<Input
					readOnly
					aria-label={t("calendar.feed.linkLabel", "Subscription link")}
					value={feed.url}
					className="h-8 font-mono text-xs"
				/>
				<Button
					type="button"
					variant="outline"
					size="sm"
					className="shrink-0 gap-1.5"
					onClick={() => void copy()}
				>
					<CopyIcon className="size-3.5" aria-hidden />
					{t("calendar.feed.copy", "Copy link")}
				</Button>
			</div>
			<FlagSwitches
				flags={flags}
				values={valuesOf(feed)}
				disabled={busy}
				onChange={(flag, value) => onChange({ [flag]: value })}
			/>
			<DialogFooter className="sm:justify-between">
				<Button
					type="button"
					variant="ghost"
					size="sm"
					disabled={busy}
					onClick={() => onConfirming("rotate")}
				>
					{t("calendar.feed.rotate", "New link")}
				</Button>
				<Button
					type="button"
					variant="ghost"
					size="sm"
					className="text-destructive hover:text-destructive"
					disabled={busy}
					onClick={() => onConfirming("off")}
				>
					{t("calendar.feed.off", "Turn off")}
				</Button>
			</DialogFooter>
		</>
	);
}

function FeedDialog({
	bot,
	name,
	flags,
	open,
	onOpenChange,
}: Readonly<{
	bot: IBot;
	name: string;
	flags: readonly FeedFlag[];
	open: boolean;
	onOpenChange: (open: boolean) => void;
}>) {
	const { t } = useTranslation("bots");
	const local = bot.local === true;
	const feed = useBotCalendarFeed(local || !open ? null : bot.id);
	const update = useUpdateBotCalendarFeed();
	const rotate = useRotateBotCalendarFeed();
	const remove = useDeleteBotCalendarFeed();
	const [confirming, setConfirming] = useState<Confirming>(null);
	const [error, setError] = useState<string | null>(null);
	const busy = update.isPending || rotate.isPending || remove.isPending;
	const run = async (action: () => Promise<unknown>) => {
		setError(null);
		try {
			await action();
			setConfirming(null);
		} catch (failure) {
			setError(
				t(
					"calendar.feed.failed",
					"Couldn't change the calendar link: {{error}}",
					{ error: getErrorMessage(failure) },
				),
			);
		}
	};
	const change = (values: IUpdateCalendarFeed) =>
		run(() => update.mutateAsync({ botId: bot.id, update: values }));
	return (
		<Dialog
			open={open}
			onOpenChange={(next) => {
				setConfirming(null);
				setError(null);
				onOpenChange(next);
			}}
		>
			<DialogContent className="max-h-[85vh] gap-3 overflow-y-auto sm:max-w-md">
				<DialogHeader>
					<DialogTitle>
						{t("calendar.feed.title", "Show in your calendar")}
					</DialogTitle>
					<DialogDescription className="sr-only">
						{t(
							"calendar.feed.intro",
							"Show {{name}}'s calendar in Apple Calendar, Google Calendar, Outlook or any other calendar app. It is read only and keeps itself up to date.",
							{ name },
						)}
					</DialogDescription>
				</DialogHeader>
				{local && (
					<p className="text-sm text-muted-foreground">
						{t(
							"calendar.feed.localOnly",
							"A calendar link needs your account: this bot lives only on this device.",
						)}
					</p>
				)}
				{!local && feed.isError && (
					<p role="alert" className="text-sm text-destructive">
						{t(
							"calendar.feed.loadFailed",
							"Couldn't load the calendar link: {{error}}",
							{ error: getErrorMessage(feed.error) },
						)}
					</p>
				)}
				{!local && feed.isSuccess && feed.data === null && (
					<CreateFeed
						name={name}
						flags={flags}
						busy={busy}
						onCreate={(values) => void change(values)}
					/>
				)}
				{!local && feed.data && (
					<ExistingFeed
						feed={feed.data}
						name={name}
						flags={flags}
						busy={busy}
						confirming={confirming}
						onConfirming={setConfirming}
						onChange={(values) => void change(values)}
						onRotate={() => void run(() => rotate.mutateAsync(bot.id))}
						onDelete={() => void run(() => remove.mutateAsync(bot.id))}
					/>
				)}
				{error && (
					<p role="alert" className="text-sm text-destructive">
						{error}
					</p>
				)}
			</DialogContent>
		</Dialog>
	);
}

/** "Show in your calendar": the bot's private calendar link; absent when the bot has nothing to show. */
export function CalendarFeedButton({
	bot,
	name,
}: Readonly<{ bot: IBot; name: string }>) {
	const { t } = useTranslation("bots");
	const [open, setOpen] = useState(false);
	const flags = feedFlags(bot);
	if (!flags) return null;
	return (
		<>
			<Button
				type="button"
				variant="ghost"
				size="sm"
				className="h-7 gap-1.5 px-2 text-xs"
				onClick={() => setOpen(true)}
			>
				<CalendarPlusIcon className="size-3.5" aria-hidden />
				{t("calendar.feed.button", "Show in your calendar")}
			</Button>
			<FeedDialog
				bot={bot}
				name={name}
				flags={flags}
				open={open}
				onOpenChange={setOpen}
			/>
		</>
	);
}
