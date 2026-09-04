"use client";
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
	EmptyState,
	type IEvent,
	type IEventRegistration,
	Skeleton,
	Table,
	TableBody,
	TableCell,
	TableHead,
	TableHeader,
	TableRow,
	Tooltip,
	TooltipContent,
	TooltipProvider,
	TooltipTrigger,
	cn,
	formatRelativeTime,
	parseTemporalValue,
	useBackend,
} from "@flow-like/flow-like-ui";
import { i18n as i18next, useTranslation } from "@flow-like/locales";
import {
	Clock,
	ExternalLink,
	Globe,
	Link2,
	Mail,
	MessageSquare,
	Radio,
	Rss,
	Terminal,
	Timer,
	Trash2,
	Webhook,
	Workflow,
	Zap,
} from "lucide-react";
import Link from "next/link";
import { useCallback, useEffect, useState } from "react";

const SINK_TYPE_CONFIG: Record<
	string,
	{ label: string; icon: typeof Zap; color: string }
> = {
	discord: {
		label: "Discord",
		icon: MessageSquare,
		color: "bg-indigo-500/10 text-indigo-500",
	},
	email: { label: "Email", icon: Mail, color: "bg-red-500/10 text-red-500" },
	slack: {
		label: "Slack",
		icon: MessageSquare,
		color: "bg-purple-500/10 text-purple-500",
	},
	telegram: {
		label: "Telegram",
		icon: MessageSquare,
		color: "bg-blue-500/10 text-blue-500",
	},
	web_watcher: {
		label: "Web Watcher",
		icon: Globe,
		color: "bg-cyan-500/10 text-cyan-500",
	},
	rss: { label: "RSS", icon: Rss, color: "bg-orange-500/10 text-orange-500" },
	deeplink: {
		label: "Deeplink",
		icon: Link2,
		color: "bg-violet-500/10 text-violet-500",
	},
	http: { label: "HTTP", icon: Globe, color: "bg-green-500/10 text-green-500" },
	webhook: {
		label: "Webhook",
		icon: Webhook,
		color: "bg-emerald-500/10 text-emerald-500",
	},
	mqtt: { label: "MQTT", icon: Radio, color: "bg-teal-500/10 text-teal-500" },
	mcp: { label: "MCP", icon: Terminal, color: "bg-gray-500/10 text-gray-500" },
	file: { label: "File", icon: Zap, color: "bg-yellow-500/10 text-yellow-500" },
	github: {
		label: "GitHub",
		icon: Zap,
		color: "bg-slate-500/10 text-slate-500",
	},
	nfc: { label: "NFC", icon: Radio, color: "bg-pink-500/10 text-pink-500" },
	geolocation: {
		label: "Geolocation",
		icon: Globe,
		color: "bg-lime-500/10 text-lime-500",
	},
	notion: {
		label: "Notion",
		icon: Zap,
		color: "bg-stone-500/10 text-stone-500",
	},
	shortcut: {
		label: "Shortcut",
		icon: Zap,
		color: "bg-amber-500/10 text-amber-500",
	},
	cron: { label: "Cron Job", icon: Timer, color: "bg-sky-500/10 text-sky-500" },
};

function getSinkTypeConfig(type: string) {
	return (
		SINK_TYPE_CONFIG[type] ?? {
			label: type,
			icon: Zap,
			color: "bg-gray-500/10 text-gray-500",
		}
	);
}

function getConfigSummary(
	config: Record<string, unknown>,
	type: string,
): string {
	switch (type) {
		case "http":
			return `${(config.method as string) ?? "GET"} ${(config.path as string) ?? "/"}`;
		case "cron":
			if (config.expression)
				return i18next.t("cronExpression", "Cron: {{expression}}", {
					expression: config.expression,
				});
			if (config.scheduled_for) {
				const sf = config.scheduled_for as { date: string; time: string };
				return i18next.t("scheduledDateTime", "Scheduled: {{date}} {{time}}", {
					date: sf.date,
					time: sf.time,
				});
			}
			return "Scheduled";
		case "deeplink":
			return i18next.t("routeVal", "Route: {{val}}", {
				val: (config.route as string) ?? "",
			});
		case "webhook":
			return i18next.t("pathVal", "Path: {{val}}", {
				val: (config.path as string) ?? "/",
			});
		case "discord":
			return i18next.t("botVal", "Bot: {{val}}", {
				val: (config.bot_name as string) ?? "Discord Bot",
			});
		case "telegram":
			return i18next.t("botVal", "Bot: {{val}}", {
				val: (config.bot_name as string) ?? "Telegram Bot",
			});
		case "slack":
			return i18next.t("botVal", "Bot: {{val}}", {
				val: (config.bot_name as string) ?? "Slack Bot",
			});
		case "email":
			return i18next.t("imapVal", "IMAP: {{val}}", {
				val: (config.imap_server as string) ?? "",
			});
		case "mqtt":
			return i18next.t("topicVal", "Topic: {{val}}", {
				val: (config.topic as string) ?? "",
			});
		case "github":
			return i18next.t("repoVal", "Repo: {{val}}", {
				val: (config.repository as string) ?? "",
			});
		case "file":
			return i18next.t("pathVal", "Path: {{val}}", {
				val: (config.path as string) ?? "",
			});
		case "web_watcher":
			return i18next.t("urlVal", "URL: {{val}}", {
				val: (config.url as string) ?? "",
			});
		case "rss":
			return i18next.t("feedVal", "Feed: {{val}}", {
				val: (config.feed_url as string) ?? "",
			});
		default:
			return "";
	}
}

interface SinkWithEvent extends IEventRegistration {
	event?: IEvent;
}

export default function Page() {
	const { t } = useTranslation("common");
	const backend = useBackend();

	const [sinks, setSinks] = useState<SinkWithEvent[]>([]);
	const [appNames, setAppNames] = useState<Record<string, string>>({});
	const [isLoading, setIsLoading] = useState(true);

	const fetchSinks = useCallback(async () => {
		if (!backend.sinkState) {
			setIsLoading(false);
			return;
		}
		setIsLoading(true);
		try {
			const data = await backend.sinkState.listEventSinks();

			// Fetch event data for each sink to get board_id, node_id, etc.
			const sinksWithEvents: SinkWithEvent[] = await Promise.all(
				data.map(async (sink) => {
					try {
						const event = await backend.eventState.getEvent(
							sink.app_id,
							sink.event_id,
						);
						return { ...sink, event };
					} catch {
						return sink;
					}
				}),
			);
			setSinks(sinksWithEvents);

			// Fetch app names for all unique app IDs
			const uniqueAppIds = [...new Set(data.map((s) => s.app_id))];
			const names: Record<string, string> = {};
			await Promise.all(
				uniqueAppIds.map(async (appId) => {
					try {
						const meta = await backend.appState.getAppMeta(appId);
						names[appId] = meta.name;
					} catch {
						names[appId] = appId;
					}
				}),
			);
			setAppNames(names);
		} catch (error) {
			console.error("Failed to fetch sinks:", error);
		} finally {
			setIsLoading(false);
		}
	}, [backend.sinkState, backend.appState, backend.eventState]);

	useEffect(() => {
		fetchSinks();
	}, [fetchSinks]);

	const [deleteDialog, setDeleteDialog] = useState<{
		open: boolean;
		sink: IEventRegistration | null;
	}>({ open: false, sink: null });

	const handleDelete = useCallback(async () => {
		if (!deleteDialog.sink || !backend.sinkState) return;
		try {
			await backend.sinkState.removeEventSink(deleteDialog.sink.event_id);
			fetchSinks();
		} catch (error) {
			console.error("Failed to remove sink:", error);
		} finally {
			setDeleteDialog({ open: false, sink: null });
		}
	}, [deleteDialog.sink, backend.sinkState, fetchSinks]);

	if (!backend.sinkState) {
		return (
			<EmptyState
				icons={[Zap]}
				title={t("notAvailable", "Not available")}
				description={t(
					"activeSinksAreOnlyAvailableInTheDesktopApp",
					"Active sinks are only available in the desktop app.",
				)}
			/>
		);
	}

	return (
		<TooltipProvider>
			<div className="h-full flex flex-col max-h-full overflow-auto min-h-0">
				<div className="container mx-auto px-2 pb-4 flex flex-col h-full gap-6">
					<div className="flex flex-col gap-2 pt-2">
						<h1 className="text-3xl font-bold tracking-tight">
							{t("activeSinks", "Active Sinks")}
						</h1>
						<p className="text-muted-foreground">
							{t(
								"viewAndManageAllActiveEventTriggersAcrossAllApps",
								"View and manage all active event triggers across all apps",
							)}
						</p>
					</div>

					{isLoading ? (
						<SinksLoadingSkeleton />
					) : sinks.length === 0 ? (
						<div className="flex-1 flex justify-center w-full">
							<EmptyState
								icons={[Zap, Clock, Globe]}
								title={t("noActiveSinks", "No active sinks")}
								className="flex-1 w-full max-w-full"
								description={t(
									"sinksAreCreatedWhenYouConfigureEventsWithTriggersLikeHttpEndpointsCronJobsOrWebhooksConfigureAnEventToGetStarted",
									"Sinks are created when you configure events with triggers like HTTP endpoints, cron jobs, or webhooks. Configure an event to get started.",
								)}
							/>
						</div>
					) : (
						<SinksTable
							sinks={sinks}
							appNames={appNames}
							onDelete={(sink) => setDeleteDialog({ open: true, sink })}
						/>
					)}
				</div>

				<AlertDialog
					open={deleteDialog.open}
					onOpenChange={(open) =>
						!open && setDeleteDialog({ open: false, sink: null })
					}
				>
					<AlertDialogContent>
						<AlertDialogHeader>
							<AlertDialogTitle>
								{t("disableSink", "Disable Sink")}
							</AlertDialogTitle>
							<AlertDialogDescription>
								{t(
									"disableSinkConfirmation",
									'This will disable the "{{name}}" sink. The event configuration will remain, but it won\'t be triggered automatically anymore. You can re-enable it by saving the event again.',
									{ name: deleteDialog.sink?.name ?? "" },
								)}
							</AlertDialogDescription>
						</AlertDialogHeader>
						<AlertDialogFooter>
							<AlertDialogCancel>{t("cancel", "Cancel")}</AlertDialogCancel>
							<AlertDialogAction
								onClick={handleDelete}
								className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
							>
								{t("disable", "Disable")}
							</AlertDialogAction>
						</AlertDialogFooter>
					</AlertDialogContent>
				</AlertDialog>
			</div>
		</TooltipProvider>
	);
}

function SinksLoadingSkeleton() {
	return (
		<Card>
			<CardHeader>
				<Skeleton className="h-6 w-32" />
			</CardHeader>
			<CardContent>
				<div className="space-y-3">
					{Array.from({ length: 3 }).map((_, i) => (
						<Skeleton key={i} className="h-12 w-full" />
					))}
				</div>
			</CardContent>
		</Card>
	);
}

function SinksTable({
	sinks,
	appNames,
	onDelete,
}: Readonly<{
	sinks: SinkWithEvent[];
	appNames: Record<string, string>;
	onDelete: (sink: SinkWithEvent) => void;
}>) {
	const { t } = useTranslation("common");
	return (
		<Card>
			<CardHeader>
				<CardTitle className="text-lg">
					{t("registeredSinks", "Registered Sinks")}
				</CardTitle>
				<CardDescription>
					{t("countActiveSinksAcrossAllApps", {
						defaultValue_one: "{{count}} active sink across all apps",
						defaultValue_other: "{{count}} active sinks across all apps",
						count: sinks.length,
					})}
				</CardDescription>
			</CardHeader>
			<CardContent>
				<Table>
					<TableHeader>
						<TableRow>
							<TableHead>{t("event", "Event")}</TableHead>
							<TableHead>Type</TableHead>
							<TableHead className="hidden md:table-cell">
								{t("configuration", "Configuration")}
							</TableHead>
							<TableHead className="hidden lg:table-cell">
								{t("app", "App")}
							</TableHead>
							<TableHead className="hidden xl:table-cell">
								{t("created", "Created")}
							</TableHead>
							<TableHead className="text-right">
								{t("actions", "Actions")}
							</TableHead>
						</TableRow>
					</TableHeader>
					<TableBody>
						{sinks.map((sink) => (
							<SinkRow
								key={sink.event_id}
								sink={sink}
								appName={appNames[sink.app_id] ?? sink.app_id}
								onDelete={() => onDelete(sink)}
							/>
						))}
					</TableBody>
				</Table>
			</CardContent>
		</Card>
	);
}

function SinkRow({
	sink,
	appName,
	onDelete,
}: Readonly<{
	sink: SinkWithEvent;
	appName: string;
	onDelete: () => void;
}>) {
	const { t } = useTranslation("common");
	const config = getSinkTypeConfig(sink.type);
	const Icon = config.icon;
	const configSummary = getConfigSummary(sink.config, sink.type);
	const eventHref = `/library/config/pages?id=${sink.app_id}&event=${sink.event_id}`;
	const appHref = `/library/config/pages?id=${sink.app_id}`;

	// Build board link with node focus if event data is available
	const boardHref = sink.event?.board_id
		? `/flow?id=${sink.event.board_id}&app=${sink.app_id}${sink.event.node_id ? `&node=${sink.event.node_id}` : ""}${sink.event.board_version ? `&version=${sink.event.board_version.join("_")}` : ""}`
		: null;

	return (
		<TableRow className="group">
			<TableCell>
				<div className="flex flex-col gap-0.5">
					<span className="font-medium">{sink.name}</span>
					<Link
						href={eventHref}
						className="text-xs text-muted-foreground truncate max-w-[200px] hover:text-primary hover:underline"
					>
						{sink.event_id}
					</Link>
				</div>
			</TableCell>
			<TableCell>
				<Badge variant="secondary" className={cn("gap-1.5", config.color)}>
					<Icon className="h-3.5 w-3.5" />
					{config.label}
				</Badge>
			</TableCell>
			<TableCell className="hidden md:table-cell">
				<span className="text-sm text-muted-foreground font-mono">
					{configSummary || "—"}
				</span>
			</TableCell>
			<TableCell className="hidden lg:table-cell">
				<Link
					href={appHref}
					className="text-sm text-muted-foreground truncate max-w-[150px] block hover:text-primary hover:underline"
				>
					{appName}
				</Link>
			</TableCell>
			<TableCell className="hidden xl:table-cell">
				<span className="text-sm text-muted-foreground">
					{formatRelativeTime(parseTemporalValue(sink.created_at))}
				</span>
			</TableCell>
			<TableCell className="text-right">
				<div className="flex items-center justify-end gap-1">
					{boardHref && (
						<Tooltip>
							<TooltipTrigger asChild>
								<Link href={boardHref}>
									<Button variant="ghost" size="icon" className="h-8 w-8">
										<Workflow className="h-4 w-4" />
									</Button>
								</Link>
							</TooltipTrigger>
							<TooltipContent>
								{t("openFlowBoard", "Open Flow Board")}
							</TooltipContent>
						</Tooltip>
					)}
					<Tooltip>
						<TooltipTrigger asChild>
							<Link href={eventHref}>
								<Button variant="ghost" size="icon" className="h-8 w-8">
									<ExternalLink className="h-4 w-4" />
								</Button>
							</Link>
						</TooltipTrigger>
						<TooltipContent>{t("goToEvent", "Go to Event")}</TooltipContent>
					</Tooltip>
					<Tooltip>
						<TooltipTrigger asChild>
							<Button
								variant="ghost"
								size="icon"
								className="h-8 w-8 text-destructive hover:text-destructive hover:bg-destructive/10"
								onClick={onDelete}
							>
								<Trash2 className="h-4 w-4" />
							</Button>
						</TooltipTrigger>
						<TooltipContent>{t("disableSink", "Disable Sink")}</TooltipContent>
					</Tooltip>
				</div>
			</TableCell>
		</TableRow>
	);
}
