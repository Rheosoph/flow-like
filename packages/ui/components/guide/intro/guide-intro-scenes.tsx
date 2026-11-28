"use client";

import { useTranslation } from "@flow-like/locales";
import {
	BotIcon,
	CheckIcon,
	EyeIcon,
	LayoutGridIcon,
	Loader2Icon,
	MailIcon,
	SendIcon,
	UserIcon,
	WorkflowIcon,
} from "lucide-react";
import {
	type CSSProperties,
	type ReactNode,
	useCallback,
	useEffect,
	useMemo,
	useRef,
	useState,
} from "react";
import type { GuideTourId } from "../../../lib/guide/guide-model";
import { cn } from "../../../lib/utils";
import { BotAvatar } from "../../bots/bot-avatar";
import type { BotAppearance } from "../../bots/types";
import { Button } from "../../ui/button";

export type IntroStepId = "ask" | "app" | "flow" | "choose";
export type IntroHost = "device" | "cloud";
type IntroWhere = "device" | "server" | "cloud";

const POST: BotAppearance = { kind: "mochi" };
const SCOUT: BotAppearance = { kind: "pip" };

function prefersReducedMotion(): boolean {
	return (
		typeof window !== "undefined" &&
		window.matchMedia?.("(prefers-reduced-motion: reduce)").matches === true
	);
}

const enter =
	"motion-safe:animate-in motion-safe:fade-in motion-safe:slide-in-from-bottom-2 motion-safe:duration-300";

function Chip({
	tone,
	children,
	className,
}: Readonly<{
	tone: "good" | "busy" | "quiet";
	children: ReactNode;
	className?: string;
}>) {
	return (
		<span
			className={cn(
				"inline-flex w-fit items-center gap-1.5 whitespace-nowrap rounded-full px-2.5 py-1 font-semibold text-xs",
				tone === "good" && "bg-good-bg text-good",
				tone === "busy" && "bg-warning-bg text-warning",
				tone === "quiet" && "bg-muted text-muted-foreground",
				className,
			)}
		>
			{children}
		</span>
	);
}

const LAYER_REACHED: Readonly<Record<IntroStepId, number>> = {
	ask: 1,
	app: 2,
	flow: 3,
	choose: 4,
};

/** You, a bot, an app, a flow: how far down the story has gone. */
export function IntroRail({ step }: Readonly<{ step: IntroStepId }>) {
	const { t } = useTranslation("guide");
	const layers = [
		{
			icon: UserIcon,
			label: t("intro.rail.you", "You"),
			hint: t("intro.rail.youHint", "say what you need"),
		},
		{
			icon: BotIcon,
			label: t("intro.rail.bot", "A bot"),
			hint: t("intro.rail.botHint", "does it, asks first"),
		},
		{
			icon: LayoutGridIcon,
			label: t("intro.rail.app", "An app"),
			hint: t("intro.rail.appHint", "does the routine"),
		},
		{
			icon: WorkflowIcon,
			label: t("intro.rail.flow", "A flow"),
			hint: t("intro.rail.flowHint", "what the app is made of"),
		},
	];
	const reached = LAYER_REACHED[step];

	return (
		<ol className="m-0 flex w-full list-none gap-2 p-0 sm:w-40 sm:flex-none sm:flex-col sm:gap-0">
			{layers.map(({ icon: Icon, label, hint }, index) => {
				const current = index === reached;
				const passed = index < reached;
				return (
					<li
						key={label}
						aria-current={current ? "step" : undefined}
						className={cn(
							"relative flex flex-1 flex-col items-center gap-1.5 text-center sm:flex-none sm:flex-row sm:items-start sm:gap-3 sm:pb-8 sm:text-left sm:last:pb-0",
							"sm:before:absolute sm:before:top-7 sm:before:bottom-0 sm:before:left-[13px] sm:before:w-0.5 sm:last:before:hidden",
							passed ? "sm:before:bg-foreground" : "sm:before:bg-border",
							current || passed ? "text-foreground" : "text-muted-foreground",
						)}
					>
						<span
							className={cn(
								"relative z-10 flex size-7 flex-none items-center justify-center rounded-lg border-2 transition-colors",
								current &&
									"border-primary bg-primary text-primary-foreground shadow-[0_0_0_5px_color-mix(in_oklch,var(--primary)_18%,transparent)]",
								passed && "border-foreground bg-foreground text-background",
								!current && !passed && "border-border bg-background",
							)}
						>
							<Icon className="size-3.5" aria-hidden="true" />
						</span>
						<span className="flex flex-col gap-0.5 sm:pt-1">
							<span className="font-semibold text-xs sm:text-sm">{label}</span>
							<span className="hidden text-muted-foreground text-xs leading-snug sm:block">
								{hint}
							</span>
						</span>
					</li>
				);
			})}
		</ol>
	);
}

type ChatPhase =
	| "draft"
	| "typing"
	| "asking"
	| "reading"
	| "found"
	| "declined";

function useChatPhase() {
	const [phase, setPhase] = useState<ChatPhase>("draft");
	const timer = useRef<ReturnType<typeof setTimeout> | null>(null);

	useEffect(
		() => () => {
			if (timer.current) clearTimeout(timer.current);
		},
		[],
	);

	const after = useCallback((now: ChatPhase, next: ChatPhase, ms: number) => {
		if (timer.current) clearTimeout(timer.current);
		setPhase(now);
		timer.current = setTimeout(
			() => setPhase(next),
			prefersReducedMotion() ? 0 : ms,
		);
	}, []);

	return {
		phase,
		send: useCallback(() => after("typing", "asking", 1100), [after]),
		allow: useCallback(() => after("reading", "found", 1600), [after]),
		decline: useCallback(() => setPhase("declined"), []),
	};
}

function BotLine({
	working,
	children,
}: Readonly<{ working?: boolean; children: ReactNode }>) {
	return (
		<div className={cn("flex items-start gap-2.5", enter)}>
			<BotAvatar
				appearance={POST}
				size={28}
				agentState={working ? "working" : "idle"}
				decorative
			/>
			<div className="flex min-w-0 flex-1 flex-col gap-2.5">{children}</div>
		</div>
	);
}

function BotBubble({ children }: Readonly<{ children: ReactNode }>) {
	return (
		<p
			className={cn(
				"m-0 w-fit max-w-[85%] rounded-2xl bg-muted px-3.5 py-2.5 text-sm leading-relaxed",
				enter,
			)}
		>
			{children}
		</p>
	);
}

/** You ask Post for something; Post asks before it reads your mail. */
export function ChatScene() {
	const { t } = useTranslation("guide");
	const { phase, send, allow, decline } = useChatPhase();
	const request = t(
		"intro.chat.request",
		"Every Monday, send me a summary of new invoices.",
	);
	const sent = phase !== "draft";
	const asked = !["draft", "typing"].includes(phase);
	const answered = ["reading", "found"].includes(phase);

	return (
		<div className="flex flex-col overflow-hidden rounded-2xl border bg-card shadow-sm">
			<div className="flex items-center gap-2.5 border-b px-4 py-3">
				<BotAvatar
					appearance={POST}
					size={28}
					agentState={phase === "reading" ? "working" : "idle"}
					decorative
				/>
				<span className="font-semibold text-sm">Post</span>
				<span className="text-muted-foreground text-xs">
					{t("intro.chat.role", "Handles your mail")}
				</span>
			</div>
			<div className="flex min-h-64 flex-col gap-3 p-4" aria-live="polite">
				{sent && (
					<p
						className={cn(
							"m-0 max-w-[85%] self-end rounded-2xl bg-foreground px-3.5 py-2.5 text-background text-sm leading-relaxed",
							enter,
						)}
					>
						{request}
					</p>
				)}
				{phase === "typing" && (
					<BotLine working>
						<output
							className="fl-intro-typing block pt-2"
							aria-label={t("intro.chat.typing", "Post is typing")}
						>
							<span />
							<span />
							<span />
						</output>
					</BotLine>
				)}
				{asked && (
					<BotLine>
						<BotBubble>
							{t(
								"intro.chat.reply",
								"Happy to. First, I need to read your Finance inbox.",
							)}
						</BotBubble>
						<div className="flex max-w-md flex-col gap-2.5 rounded-2xl border border-warning-line bg-warning-bg p-4">
							<span className="font-semibold text-sm">
								{t(
									"intro.chat.approvalTitle",
									"Let Post read the Finance inbox?",
								)}
							</span>
							<span className="text-muted-foreground text-xs leading-relaxed">
								{t(
									"intro.chat.approvalBody",
									"Read only, only for this job. You can take it back any time.",
								)}
							</span>
							{phase === "asking" ? (
								<div className="flex flex-wrap gap-2">
									<Button size="sm" className="fl-intro-ring" onClick={allow}>
										{t("intro.chat.allow", "Allow")}
									</Button>
									<Button size="sm" variant="outline" onClick={decline}>
										{t("intro.chat.decline", "Not now")}
									</Button>
								</div>
							) : (
								<Chip tone={answered ? "good" : "quiet"}>
									{answered && (
										<CheckIcon className="size-3" aria-hidden="true" />
									)}
									{answered
										? t("intro.chat.allowed", "Allowed for this job")
										: t("intro.chat.declined", "Not allowed")}
								</Chip>
							)}
						</div>
						{phase === "reading" && (
							<Chip tone="busy" className={enter}>
								<Loader2Icon
									className="size-3 motion-safe:animate-spin"
									aria-hidden="true"
								/>
								{t("intro.chat.reading", "Reading Finance inbox")}
							</Chip>
						)}
						{phase === "found" && (
							<BotBubble>
								{t(
									"intro.chat.found",
									"Found 4 new invoices. I put them in Invoice Inbox for you.",
								)}
							</BotBubble>
						)}
						{phase === "declined" && (
							<BotBubble>
								{t(
									"intro.chat.sample",
									"Okay, I'll leave your inbox alone. Here's a sample in Invoice Inbox instead.",
								)}
							</BotBubble>
						)}
					</BotLine>
				)}
			</div>
			<div className="flex items-center gap-2 border-t px-4 py-3">
				<span
					className={cn(
						"min-w-0 flex-1 truncate rounded-xl border bg-background px-3.5 py-2.5 text-sm",
						sent && "text-muted-foreground",
					)}
				>
					{sent ? t("intro.chat.placeholder", "Message Post") : request}
				</span>
				{!sent && (
					<Button className="fl-intro-ring" onClick={send}>
						{t("intro.chat.send", "Send")}
						<SendIcon aria-hidden="true" />
					</Button>
				)}
			</div>
		</div>
	);
}

const INVOICES = [
	{ vendor: "Northwind GmbH", dueInDays: 5, amount: 1240 },
	{ vendor: "Lumen Studio", dueInDays: 11, amount: 380 },
	{ vendor: "Hale & Co", dueInDays: 16, amount: 2015 },
	{ vendor: "Brightline Ltd", dueInDays: 22, amount: 1485 },
] as const;

function useInvoiceFormat() {
	const { i18n } = useTranslation("guide");
	const language = i18n?.language || "en";
	return useMemo(() => {
		const money = new Intl.NumberFormat(language, {
			style: "currency",
			currency: "EUR",
			maximumFractionDigits: 0,
		});
		const day = new Intl.DateTimeFormat(language, {
			month: "short",
			day: "numeric",
		});
		const today = Date.now();
		return {
			money: (amount: number) => money.format(amount),
			due: (days: number) => day.format(new Date(today + days * 86_400_000)),
		};
	}, [language]);
}

function Stat({ label, value }: Readonly<{ label: string; value: string }>) {
	return (
		<div className="flex min-w-0 flex-[1_1_7rem] flex-col gap-0.5 rounded-xl border bg-background px-3 py-2.5">
			<span className="text-muted-foreground text-xs">{label}</span>
			<span className="font-semibold text-lg tabular-nums">{value}</span>
		</div>
	);
}

/** The app Post used: a finished tool, here a table of invoices. */
export function AppScene({
	onLookInside,
}: Readonly<{ onLookInside: () => void }>) {
	const { t } = useTranslation("guide");
	const format = useInvoiceFormat();
	const total = INVOICES.reduce((sum, invoice) => sum + invoice.amount, 0);
	const dueSoon = INVOICES.filter((invoice) => invoice.dueInDays <= 7).length;

	return (
		<>
			<div
				className={cn(
					"flex flex-col overflow-hidden rounded-2xl border bg-card shadow-sm",
					enter,
				)}
			>
				<div className="flex items-center gap-2.5 px-4 py-3.5">
					<span className="flex size-8 items-center justify-center rounded-lg bg-primary/10 text-primary">
						<MailIcon className="size-4" aria-hidden="true" />
					</span>
					<span className="font-semibold text-sm">
						{t("intro.app.name", "Invoice Inbox")}
					</span>
					<Chip tone="good" className="ml-auto">
						{t("intro.app.updatedBy", "Updated by Post")}
					</Chip>
				</div>
				<div className="flex flex-wrap gap-2.5 px-4 pb-3.5">
					<Stat
						label={t("intro.app.new", "New")}
						value={String(INVOICES.length)}
					/>
					<Stat
						label={t("intro.app.total", "Total")}
						value={format.money(total)}
					/>
					<Stat
						label={t("intro.app.dueSoon", "Due this week")}
						value={String(dueSoon)}
					/>
				</div>
				<div className="overflow-x-auto">
					<table className="w-full border-collapse text-sm">
						<thead>
							<tr className="bg-muted text-left text-muted-foreground text-xs uppercase tracking-wide">
								<th className="px-4 py-2 font-semibold">
									{t("intro.app.vendor", "Vendor")}
								</th>
								<th className="px-4 py-2 font-semibold">
									{t("intro.app.due", "Due")}
								</th>
								<th className="px-4 py-2 text-right font-semibold">
									{t("intro.app.amount", "Amount")}
								</th>
							</tr>
						</thead>
						<tbody>
							{INVOICES.map((invoice) => (
								<tr key={invoice.vendor} className="border-t">
									<td className="px-4 py-2.5">{invoice.vendor}</td>
									<td className="px-4 py-2.5 text-muted-foreground">
										{format.due(invoice.dueInDays)}
									</td>
									<td className="px-4 py-2.5 text-right font-semibold tabular-nums">
										{format.money(invoice.amount)}
									</td>
								</tr>
							))}
						</tbody>
					</table>
				</div>
			</div>
			<Button
				variant="outline"
				className="fl-intro-ring w-fit"
				onClick={onLookInside}
			>
				<EyeIcon aria-hidden="true" />
				{t("intro.app.lookInside", "Look inside the app")}
			</Button>
		</>
	);
}

interface IFlowNode {
	readonly id: string;
	readonly kind: string;
	readonly label: string;
	readonly event?: boolean;
	readonly left: string;
	readonly top: string;
}

function FlowNodeCard({
	node,
	order,
	style,
	className,
}: Readonly<{
	node: IFlowNode;
	order: number;
	style?: CSSProperties;
	className?: string;
}>) {
	return (
		<div
			className={cn(
				"fl-intro-node overflow-hidden rounded-lg border bg-card text-xs shadow-sm",
				className,
			)}
			style={{ "--node-order": order, ...style } as CSSProperties}
		>
			<div
				className={cn(
					"px-2.5 py-1 font-bold text-[10.5px] uppercase tracking-wide",
					node.event
						? "bg-primary text-primary-foreground"
						: "bg-muted text-muted-foreground",
				)}
			>
				{node.kind}
			</div>
			<div className="px-2.5 py-1.5 font-semibold">{node.label}</div>
		</div>
	);
}

/** What the app is made of: the flow, and where it ran. */
export function FlowScene({ host }: Readonly<{ host: IntroHost }>) {
	const { t } = useTranslation("guide");
	const [where, setWhere] = useState<IntroWhere>(host);
	useEffect(() => setWhere(host), [host]);

	const nodes: IFlowNode[] = [
		{
			id: "schedule",
			kind: t("intro.flow.kind.event", "Event"),
			label: t("intro.flow.node.schedule", "Every Monday 8:00"),
			event: true,
			left: "3%",
			top: "10%",
		},
		{
			id: "read",
			kind: t("intro.flow.kind.mail", "Mail"),
			label: t("intro.flow.node.read", "Read Finance inbox"),
			left: "36%",
			top: "10%",
		},
		{
			id: "extract",
			kind: t("intro.flow.kind.ai", "AI"),
			label: t("intro.flow.node.extract", "Extract invoice"),
			left: "69%",
			top: "10%",
		},
		{
			id: "store",
			kind: t("intro.flow.kind.data", "Data"),
			label: t("intro.flow.node.store", "Add to table"),
			left: "36%",
			top: "60%",
		},
		{
			id: "summary",
			kind: t("intro.flow.kind.mail", "Mail"),
			label: t("intro.flow.node.summary", "Send summary"),
			left: "69%",
			top: "60%",
		},
	];
	const places: { id: IntroWhere; label: string }[] = [
		{ id: "device", label: t("intro.where.device", "This device") },
		{ id: "server", label: t("intro.where.server", "Your servers") },
		{ id: "cloud", label: t("intro.where.cloud", "Cloud") },
	];
	const ran = {
		device: t("intro.flow.ran.device", "Ran on this device · 3.8 s · 5 steps"),
		server: t("intro.flow.ran.server", "Ran on your servers · 3.8 s · 5 steps"),
		cloud: t("intro.flow.ran.cloud", "Ran in the cloud · 3.8 s · 5 steps"),
	}[where];
	const boardLabel = t(
		"intro.flow.boardLabel",
		"The Invoice Inbox flow: every Monday it reads the Finance inbox, extracts each invoice, adds it to the table and sends a summary.",
	);

	return (
		<>
			<div
				role="img"
				aria-label={boardLabel}
				className={cn(
					"fl-intro-dots relative hidden h-72 overflow-hidden rounded-2xl border bg-muted/60 sm:block",
					enter,
				)}
			>
				<svg
					className="absolute inset-0 size-full"
					viewBox="0 0 100 100"
					preserveAspectRatio="none"
					aria-hidden="true"
				>
					{[
						"M31 20 L36 20",
						"M64 20 L69 20",
						"M83 30 C83 46, 50 44, 50 60",
						"M64 70 L69 70",
					].map((path) => (
						<path
							key={path}
							d={path}
							fill="none"
							stroke="var(--primary)"
							strokeWidth={2}
							vectorEffect="non-scaling-stroke"
						/>
					))}
				</svg>
				{nodes.map((node, index) => (
					<FlowNodeCard
						key={node.id}
						node={node}
						order={index}
						className="absolute w-[28%]"
						style={{ left: node.left, top: node.top }}
					/>
				))}
			</div>
			<ol
				aria-label={boardLabel}
				className="fl-intro-dots m-0 flex list-none flex-col gap-2 rounded-2xl border bg-muted/60 p-3 sm:hidden"
			>
				{nodes.map((node, index) => (
					<li key={node.id}>
						<FlowNodeCard node={node} order={index} />
					</li>
				))}
			</ol>
			<div
				className={cn(
					"flex flex-col gap-3 rounded-2xl border bg-card p-4 shadow-sm",
					enter,
				)}
			>
				<Chip tone="good">
					<CheckIcon className="size-3" aria-hidden="true" />
					{ran}
				</Chip>
				<div className="flex flex-col gap-2">
					<span className="font-semibold text-muted-foreground text-xs uppercase tracking-wide">
						{t("intro.where.label", "Where it runs")}
					</span>
					<div className="flex flex-wrap gap-2">
						{places.map((place) => (
							<Button
								key={place.id}
								size="sm"
								variant={where === place.id ? "default" : "outline"}
								aria-pressed={where === place.id}
								className="min-h-10 flex-1"
								onClick={() => setWhere(place.id)}
							>
								{place.label}
							</Button>
						))}
					</div>
				</div>
			</div>
		</>
	);
}

function Choice({
	inverted,
	eyebrow,
	icon,
	title,
	body,
	cta,
	onSelect,
}: Readonly<{
	inverted?: boolean;
	eyebrow?: string;
	icon: ReactNode;
	title: string;
	body: string;
	cta: string;
	onSelect: () => void;
}>) {
	return (
		<button
			type="button"
			onClick={onSelect}
			className={cn(
				"flex w-full items-start gap-4 rounded-2xl border p-5 text-left transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2",
				inverted
					? "border-foreground bg-foreground text-background hover:bg-foreground/90"
					: "bg-card hover:border-primary",
				enter,
			)}
		>
			<span className="flex-none">{icon}</span>
			<span className="flex min-w-0 flex-col gap-1.5">
				{eyebrow && (
					<span
						className={cn(
							"font-bold text-xs uppercase tracking-wide",
							inverted ? "text-background/70" : "text-muted-foreground",
						)}
					>
						{eyebrow}
					</span>
				)}
				<span className="font-semibold text-lg">{title}</span>
				<span
					className={cn(
						"text-sm leading-relaxed",
						inverted ? "text-background/80" : "text-muted-foreground",
					)}
				>
					{body}
				</span>
				<span
					className={cn(
						"pt-1 font-medium text-sm",
						inverted ? "text-background" : "text-primary",
					)}
				>
					{cta}
				</span>
			</span>
		</button>
	);
}

/** The welcome question: which tour to walk through, if any. */
export function ChoiceScene({
	seenInside,
	onAnswer,
}: Readonly<{
	seenInside: boolean;
	onAnswer: (tour: GuideTourId | null) => void;
}>) {
	const { t } = useTranslation("guide");

	return (
		<div className="flex flex-col gap-3">
			<Choice
				eyebrow={
					seenInside
						? t("intro.choose.stay", "Stay up here · You, bots, apps")
						: undefined
				}
				icon={
					<span className="flex gap-1" aria-hidden="true">
						<BotAvatar appearance={POST} size={32} decorative />
						<BotAvatar appearance={SCOUT} size={32} decorative />
					</span>
				}
				title={t("welcome.use.title", "Get things done")}
				body={t(
					"welcome.use.body",
					"Hand tasks to bots and use ready-made apps. No building.",
				)}
				cta={t("welcome.use.cta", "Show me bots and apps")}
				onSelect={() => onAnswer("use")}
			/>
			{seenInside && (
				<div
					className="flex items-center gap-3 font-semibold text-muted-foreground text-xs"
					aria-hidden="true"
				>
					<span className="h-px flex-1 bg-border" />
					{t("intro.choose.or", "or go inside")}
					<span className="h-px flex-1 bg-border" />
				</div>
			)}
			<Choice
				inverted
				eyebrow={
					seenInside ? t("intro.choose.inside", "Go inside · Flows") : undefined
				}
				icon={
					<span className="flex size-10 items-center justify-center rounded-xl bg-primary text-primary-foreground">
						<WorkflowIcon className="size-5" aria-hidden="true" />
					</span>
				}
				title={t("welcome.build.title", "Build my own")}
				body={t(
					"welcome.build.body",
					"Make apps and automations on the board. Turns on developer mode.",
				)}
				cta={t("welcome.build.cta", "Show me the board")}
				onSelect={() => onAnswer("build")}
			/>
			<div className="flex flex-wrap items-center justify-between gap-2 text-muted-foreground text-sm">
				<span>
					{t(
						"welcome.invite",
						"Joined through an invite? Your team's apps are already in My Apps.",
					)}
				</span>
				<Button variant="ghost" onClick={() => onAnswer(null)}>
					{t("welcome.skip", "I'll look around myself")}
				</Button>
			</div>
		</div>
	);
}
