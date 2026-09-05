"use client";

import { useQuery, useQueryClient } from "@tanstack/react-query";
import {
	ArrowUp,
	ArrowUpRight,
	BookOpen,
	Box,
	CalendarDays,
	CheckCircle2,
	ChevronDown,
	FileUp,
	Library,
	Link2,
	Megaphone,
	Moon,
	Plus,
	Sparkles,
	Sun,
} from "lucide-react";
import dynamic from "next/dynamic";
import Link from "next/link";
import { useRouter } from "next/navigation";
import { Fragment, useEffect, useState } from "react";
import { useAuth } from "react-oidc-context";
import { toast } from "sonner";
import { nowSystemTime } from "../../../lib/time/now";
import { cn } from "../../../lib/utils";
import { useBackend } from "../../../state/backend-state";
import { useGlobalChatStore } from "../../../state/global-chat/global-chat-store";
import { CreateFlowDialog } from "../../create-flow-dialog";
import { FlowPilotBubbleOrb } from "../../global-chat/flowpilot-bubble-orb";
import { useFlowPilotOrbState } from "../../global-chat/flowpilot-orb-state";
import { useHeroComposer } from "../../global-chat/hero-variants/use-hero-composer";
import { Button } from "../../ui/button";
import { Checkbox } from "../../ui/checkbox";
import { Textarea } from "../../ui/textarea";
import { homeGreetingForHour, homeGreetingName } from "../home-greeting";
import {
	type HomeContentProps,
	homeLinksRendering,
	safeHomeHref,
	stringList,
	textConfig,
} from "./config";
import { HomeEmpty, homeItemClass, homeRowClass, useHomeScope } from "./shared";

const Markdown = dynamic(
	() => import("../../ui/text-editor").then((module) => module.TextEditor),
	{ ssr: false },
);

export function HomeFlowPilot({ widget, editing }: HomeContentProps) {
	const mode = textConfig(widget.config, "mode", "bar");
	const composer = useHeroComposer();
	const openOverlay = useGlobalChatStore((state) => state.openOverlay);
	const orbState = useFlowPilotOrbState();
	const expanded = mode === "card" || mode === "hero";
	const suggestions = expanded || widget.config.suggestions === true;
	const heading =
		widget.title &&
		!["FlowPilot hero", "FlowPilot composer"].includes(widget.title)
			? widget.title
			: mode === "hero"
				? "Bring your next idea to life"
				: "What would you like to build?";
	const orb = editing ? (
		<span className="flex size-14 shrink-0 items-center justify-center rounded-full border border-violet-400/25 bg-gradient-to-br from-violet-400/15 via-blue-400/10 to-pink-400/15 text-violet-400">
			<Sparkles className="size-5" />
		</span>
	) : (
		<FlowPilotBubbleOrb
			onClick={openOverlay}
			orbState={orbState}
			className={mode === "hero" ? "size-20" : "size-14"}
		/>
	);
	if (mode === "orb")
		return (
			<div className="flex min-h-24 items-center gap-5 p-4">
				{orb}
				<div className="min-w-0">
					<p className="text-sm font-semibold">
						{widget.title && widget.title !== "FlowPilot orb"
							? widget.title
							: "FlowPilot"}
					</p>
					<p className="mt-1 text-xs leading-relaxed text-muted-foreground">
						Your next idea starts here.
					</p>
				</div>
			</div>
		);
	const placeholder = textConfig(
		widget.config,
		"placeholder",
		"Ask FlowPilot to build, explore, or explain…",
	);
	return (
		<form
			onSubmit={(event) => {
				event.preventDefault();
				if (!editing) composer.submit(composer.value);
			}}
			className={cn(
				"flex min-w-0 flex-col gap-4",
				expanded ? "p-5" : "px-1 py-1",
				mode === "hero" &&
					"bg-gradient-to-br from-violet-400/[0.06] via-transparent to-blue-400/[0.04] p-6",
			)}
		>
			{expanded && (
				<div className="flex min-w-0 items-center gap-6">
					{orb}
					<div className="min-w-0">
						<p className="mb-1 text-[10px] font-medium uppercase tracking-[0.16em] text-muted-foreground">
							FlowPilot
						</p>
						<h3
							className={cn(
								"text-lg font-semibold tracking-tight",
								mode === "hero" && "text-2xl",
							)}
						>
							{heading}
						</h3>
						<p className="mt-1 text-xs leading-relaxed text-muted-foreground">
							Build an app, find a package, or work through an idea.
						</p>
					</div>
				</div>
			)}
			<div
				className={cn(
					"flex min-w-0 gap-3 rounded-2xl border border-border/70 bg-background/40 p-2.5 transition-shadow focus-within:border-primary/40 focus-within:ring-2 focus-within:ring-primary/10",
					expanded ? "items-end" : "items-center",
					widget.config.emphasis === true &&
						"border-violet-400/30 bg-violet-500/[0.04] p-3.5",
				)}
			>
				{!expanded && (
					<span className="flex size-9 shrink-0 items-center justify-center rounded-xl bg-primary/10 text-primary">
						<Sparkles className="size-4" />
					</span>
				)}
				{expanded ? (
					<Textarea
						value={composer.value}
						onChange={(event) => composer.setValue(event.target.value)}
						placeholder={placeholder}
						aria-label="Message FlowPilot"
						rows={mode === "hero" ? 3 : 2}
						className="max-h-40 min-h-16 min-w-0 resize-none border-0 bg-transparent p-1.5 text-sm shadow-none focus-visible:ring-0 dark:bg-transparent"
						onKeyDown={(event) => {
							if (event.key === "Enter" && !event.shiftKey) {
								event.preventDefault();
								if (!editing) composer.submit(composer.value);
							}
						}}
					/>
				) : (
					<input
						value={composer.value}
						onChange={(event) => composer.setValue(event.target.value)}
						placeholder={placeholder}
						aria-label="Message FlowPilot"
						className="h-9 min-w-0 flex-1 bg-transparent text-sm outline-none placeholder:text-muted-foreground sm:text-base"
					/>
				)}
				<Button
					type="submit"
					size="icon"
					className="size-9 shrink-0 rounded-xl"
					disabled={editing || !composer.canSend}
					aria-label="Send to FlowPilot"
				>
					<ArrowUp className="size-4" />
				</Button>
			</div>
			{suggestions && (
				<div className="flex flex-wrap gap-2">
					{["Create an app", "Find a package", "Explain a node"].map(
						(prompt) => (
							<Button
								key={prompt}
								type="button"
								size="sm"
								variant="ghost"
								className="h-7 rounded-full border border-border/60 bg-background/25 px-2.5 text-[11px] font-normal text-muted-foreground hover:text-foreground"
								disabled={editing}
								onClick={() => composer.submit(prompt)}
							>
								{prompt}
								<ArrowUpRight className="ml-1 size-3" />
							</Button>
						),
					)}
				</div>
			)}
		</form>
	);
}

export function HomeGreeting({ widget }: HomeContentProps) {
	const auth = useAuth();
	const backend = useBackend();
	const scope = useHomeScope();
	const account = useQuery({
		queryKey: [backend.userState.getInfo.name, ...scope],
		queryFn: () => backend.userState.getInfo(),
		enabled: auth.isAuthenticated && Boolean(auth.user?.access_token),
		staleTime: 60_000,
		retry: false,
	});
	const compact = textConfig(widget.config, "mode") === "masthead";
	const [now, setNow] = useState<Date | null>(null);
	useEffect(() => {
		setNow(new Date());
		const timer = setInterval(() => setNow(new Date()), 60_000);
		return () => clearInterval(timer);
	}, []);
	const hour = now?.getHours() ?? 12;
	const greeting = homeGreetingForHour(hour);
	const name = homeGreetingName(
		textConfig(widget.config, "name"),
		account.data,
		auth.user?.profile,
	);
	return (
		<div
			data-home-greeting
			className={cn(
				"flex items-center justify-between gap-4 px-1",
				compact ? "min-h-14 py-1" : "min-h-24 py-3",
			)}
		>
			<div className="min-w-0">
				<p className="text-[10px] font-medium uppercase tracking-[0.16em] text-muted-foreground">
					{now?.toLocaleDateString(undefined, {
						weekday: "long",
						day: "numeric",
						month: "long",
					}) ?? "Welcome"}
				</p>
				<h1
					className={cn(
						"mt-1 font-semibold tracking-tight",
						compact ? "text-xl sm:text-2xl" : "text-2xl md:text-3xl",
					)}
				>
					{greeting}
					{name ? `, ${name}` : ""}
				</h1>
				{textConfig(widget.config, "subtitle") && (
					<p className="mt-1 text-sm text-muted-foreground">
						{textConfig(widget.config, "subtitle")}
					</p>
				)}
			</div>
			{hour >= 18 || hour < 6 ? (
				<Moon className="size-6 shrink-0 text-primary/50" />
			) : (
				<Sun className="size-6 shrink-0 text-primary/50" />
			)}
		</div>
	);
}

export const HOME_QUICK_ACTIONS = {
	create: {
		title: "Create an app",
		description: "Start a new project",
		icon: Plus,
		href: "",
	},
	import: {
		title: "Import a flow",
		description: "Bring a flow or idea into your apps",
		icon: FileUp,
		href: "",
	},
	library: {
		title: "Your library",
		description: "Open your apps and projects",
		icon: Library,
		href: "/library",
	},
	packages: {
		title: "Package store",
		description: "Find nodes and integrations",
		icon: Box,
		href: "/store/packages",
	},
	explore: {
		title: "Explore apps",
		description: "Find something useful",
		icon: Sparkles,
		href: "/store/explore/apps",
	},
	learn: {
		title: "Learn",
		description: "Build your next skill",
		icon: BookOpen,
		href: "/learn",
	},
};

export function HomeQuickActions({ widget, editing }: HomeContentProps) {
	const backend = useBackend();
	const queryClient = useQueryClient();
	const auth = useAuth();
	const router = useRouter();
	const setDraft = useGlobalChatStore((state) => state.setDraft);
	const [creating, setCreating] = useState(false);
	const actions = stringList(widget.config, "actions");
	const toolbar = textConfig(widget.config, "layout") === "toolbar";
	const create = async (name: string, online: boolean) => {
		const [profile, bits] = await Promise.all([
			backend.userState.getSettingsProfile(),
			backend.bitState.getProfileBits(),
		]);
		const app = await backend.appState.createApp(
			{
				name,
				description: "",
				tags: [],
				created_at: nowSystemTime(),
				updated_at: nowSystemTime(),
				preview_media: [],
			},
			bits.filter((bit) => bit.type === "Embedding").map((bit) => bit.id),
			online,
		);
		await backend.userState.updateProfileApp(
			profile,
			{ app_id: app.id, favorite: false, pinned: false },
			"Upsert",
		);
		await queryClient.invalidateQueries();
		router.push(`/library/config?id=${encodeURIComponent(app.id)}`);
	};
	return (
		<>
			<div
				className={cn(
					"grid gap-3",
					toolbar
						? "grid-cols-2 sm:grid-cols-4"
						: "grid-cols-[repeat(auto-fit,minmax(min(100%,150px),1fr))]",
				)}
			>
				{actions.map((id) => {
					const action =
						HOME_QUICK_ACTIONS[id as keyof typeof HOME_QUICK_ACTIONS];
					if (!action) return null;
					const Icon = action.icon;
					const content = (
						<Fragment key={id}>
							<span
								className={cn(
									"flex size-8 shrink-0 items-center justify-center rounded-lg bg-primary/10 text-primary",
									!toolbar && "mb-3",
								)}
							>
								<Icon className="size-4" />
							</span>
							<span
								className={cn(
									"block font-medium",
									toolbar ? "text-xs" : "text-sm",
								)}
							>
								{action.title}
							</span>
							{!toolbar && (
								<span className="mt-1 block text-xs leading-relaxed text-muted-foreground">
									{action.description}
								</span>
							)}
						</Fragment>
					);
					return action.href ? (
						<Link
							key={id}
							href={action.href}
							className={cn(
								homeItemClass,
								toolbar
									? "flex items-center gap-2.5 p-2.5"
									: "block h-full p-3.5",
							)}
						>
							{content}
						</Link>
					) : (
						<button
							key={id}
							type="button"
							disabled={editing}
							className={cn(
								homeItemClass,
								"text-left disabled:opacity-60",
								toolbar
									? "flex items-center gap-2.5 p-2.5"
									: "block h-full p-3.5",
							)}
							onClick={() => {
								if (id === "create") setCreating(true);
								else {
									setDraft({
										prompt: "Help me import a flow from a file or URL.",
									});
									router.push("/chat");
								}
							}}
						>
							{content}
						</button>
					);
				})}
			</div>
			<CreateFlowDialog
				open={creating}
				onOpenChange={setCreating}
				onCreateProject={create}
				isAuthenticated={auth.isAuthenticated}
				defaultOnline={auth.isAuthenticated}
				toast={toast}
			/>
		</>
	);
}

export interface HomeLinkItem {
	id?: string;
	title: string;
	description?: string;
	href: string;
}
export interface HomeInformationItem {
	id?: string;
	title: string;
	body?: string;
	href?: string;
	label?: string;
	checked?: boolean;
}

export function homeLinks(config: Record<string, unknown>): HomeLinkItem[] {
	return Array.isArray(config.links)
		? config.links
				.filter((item): item is HomeLinkItem =>
					Boolean(
						item &&
							typeof item === "object" &&
							typeof item.title === "string" &&
							typeof item.href === "string",
					),
				)
				.map((item, index) => ({ ...item, id: item.id || `link-${index}` }))
		: [];
}

export function informationItems(
	config: Record<string, unknown>,
): HomeInformationItem[] {
	return Array.isArray(config.items)
		? config.items
				.filter((item): item is HomeInformationItem =>
					Boolean(
						item && typeof item === "object" && typeof item.title === "string",
					),
				)
				.map((item, index) => ({ ...item, id: item.id || `item-${index}` }))
		: [];
}

export function HomeQuickLinks({ widget }: HomeContentProps) {
	const links = homeLinks(widget.config);
	const rendering = homeLinksRendering(
		widget.config,
		widget.appearance.variant,
	);
	if (!links.length)
		return <HomeEmpty>Add useful links in widget settings.</HomeEmpty>;
	return (
		<div
			className={cn(
				"min-w-0",
				rendering === "grid"
					? "grid content-start grid-cols-[repeat(auto-fit,minmax(min(100%,180px),1fr))] gap-2"
					: "divide-y divide-border/50",
			)}
		>
			{links.map((link) => {
				const href = safeHomeHref(link.href);
				const content = (
					<Fragment key={link.id}>
						<Link2 className="size-4 shrink-0 text-primary" />
						<div className="min-w-0 flex-1">
							<p className="truncate text-sm font-medium">{link.title}</p>
							{link.description && (
								<p className="mt-1 line-clamp-2 text-xs text-muted-foreground">
									{link.description}
								</p>
							)}
						</div>
						<ArrowUpRight className="size-4 shrink-0 text-muted-foreground" />
					</Fragment>
				);
				return href ? (
					<Link
						key={link.id}
						href={href}
						target={href.startsWith("/") ? undefined : "_blank"}
						rel={href.startsWith("/") ? undefined : "noopener noreferrer"}
						className={rendering === "grid" ? homeItemClass : homeRowClass}
					>
						{content}
					</Link>
				) : (
					<div
						key={link.id}
						className={cn(
							rendering === "grid" ? homeItemClass : homeRowClass,
							"opacity-50",
						)}
					>
						{content}
					</div>
				);
			})}
		</div>
	);
}

export function HomeInformation({
	widget,
	editing,
	onUpdate,
}: HomeContentProps) {
	const mode = textConfig(widget.config, "mode", "markdown");
	const body = textConfig(widget.config, "body");
	const items = informationItems(widget.config);
	const [now, setNow] = useState<number | null>(null);
	useEffect(() => {
		if (mode !== "countdown") return;
		setNow(Date.now());
		const timer = setInterval(() => setNow(Date.now()), 60_000);
		return () => clearInterval(timer);
	}, [mode]);
	if (mode === "countdown") {
		const date = new Date(textConfig(widget.config, "date"));
		if (!Number.isFinite(date.getTime()))
			return (
				<HomeEmpty icon={<CalendarDays className="size-7 opacity-50" />}>
					Choose a milestone date in widget settings.
				</HomeEmpty>
			);
		const days =
			now === null ? null : Math.ceil((date.getTime() - now) / 86_400_000);
		return (
			<div className="flex min-h-32 flex-col justify-center gap-1">
				<p className="mb-2 text-[10px] font-medium uppercase tracking-wider text-muted-foreground">
					{days !== null && days < 0 ? "Since the milestone" : "Coming up"}
				</p>
				<p className="text-5xl font-semibold tabular-nums tracking-tight">
					{days === null ? "…" : days === 0 ? "Today" : Math.abs(days)}
					<span className="ml-2 text-sm font-normal text-muted-foreground">
						{days === 0
							? ""
							: `${Math.abs(days ?? 0) === 1 ? "day" : "days"} ${days !== null && days < 0 ? "ago" : "to go"}`}
					</span>
				</p>
				<p className="mt-3 text-sm text-muted-foreground">
					{date.toLocaleDateString(undefined, { dateStyle: "long" })}
				</p>
				{body && <p className="mt-2 text-sm">{body}</p>}
			</div>
		);
	}
	if (["feed", "steps", "resources", "facts"].includes(mode)) {
		const resourceList =
			mode === "resources" && widget.config.layout === "list";
		if (!items.length)
			return <HomeEmpty>Add items in widget settings.</HomeEmpty>;
		return (
			<div
				className={cn(
					"min-w-0",
					["resources", "facts"].includes(mode) && !resourceList
						? "grid content-start grid-cols-[repeat(auto-fit,minmax(min(100%,180px),1fr))] gap-3"
						: "",
				)}
			>
				{items.map((item, index) => {
					const href = item.href ? safeHomeHref(item.href) : undefined;
					return (
						<article
							key={item.id}
							className={cn(
								"min-w-0",
								["resources", "facts"].includes(mode) && !resourceList
									? "rounded-xl bg-muted/35 p-3.5"
									: "border-b border-border/50 py-3 first:pt-0 last:border-0 last:pb-0",
								mode === "steps" && "flex items-start gap-3",
							)}
						>
							{mode === "steps" && (
								<span className="flex size-7 shrink-0 items-center justify-center rounded-full bg-primary/10 text-xs font-semibold text-primary">
									{index + 1}
								</span>
							)}
							<div className="min-w-0 flex-1">
								{item.label && (
									<p className="mb-2 text-[10px] font-medium uppercase tracking-wider text-primary">
										{item.label}
									</p>
								)}
								<h3
									className={cn(
										"text-sm font-semibold",
										mode === "facts" && "text-2xl tracking-tight",
									)}
								>
									{resourceList && href ? (
										<Link
											href={href}
											target={href.startsWith("/") ? undefined : "_blank"}
											rel={
												href.startsWith("/") ? undefined : "noopener noreferrer"
											}
											className="flex items-center justify-between gap-3 rounded-sm hover:text-primary focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary"
										>
											{item.title}
											<ArrowUpRight className="size-3.5 shrink-0 text-muted-foreground" />
										</Link>
									) : (
										item.title
									)}
								</h3>
								{item.body && (
									<div className="mt-1.5 text-xs leading-relaxed text-muted-foreground [&_p]:text-xs [&_p]:leading-relaxed">
										<Markdown initialContent={item.body} isMarkdown minimal />
									</div>
								)}
								{href && !resourceList && (
									<Link
										href={href}
										target={href.startsWith("/") ? undefined : "_blank"}
										rel={
											href.startsWith("/") ? undefined : "noopener noreferrer"
										}
										className="mt-3 inline-flex items-center gap-1 text-xs font-medium text-primary hover:underline"
									>
										Open <ArrowUpRight className="size-3" />
										<span className="sr-only">{item.title}</span>
									</Link>
								)}
							</div>
						</article>
					);
				})}
			</div>
		);
	}
	if (mode === "faq")
		return (
			<div className="divide-y divide-border/50">
				{items.length ? (
					items.map((item) => (
						<details
							key={item.id}
							className="group/faq py-3 first:pt-0 last:pb-0"
						>
							<summary className="flex cursor-pointer list-none items-center justify-between gap-3 rounded text-sm font-medium focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring [&::-webkit-details-marker]:hidden">
								{item.title}
								<ChevronDown className="size-4 shrink-0 text-muted-foreground transition-transform group-open/faq:rotate-180" />
							</summary>
							<div className="pt-2 text-sm text-muted-foreground">
								<Markdown initialContent={item.body ?? ""} isMarkdown minimal />
							</div>
						</details>
					))
				) : (
					<HomeEmpty>Add questions and answers in widget settings.</HomeEmpty>
				)}
			</div>
		);
	if (mode === "checklist") {
		const completed = items.filter((item) => item.checked).length;
		if (!items.length)
			return (
				<HomeEmpty icon={<CheckCircle2 />}>
					Add a first step in widget settings.
				</HomeEmpty>
			);
		return (
			<div className="min-w-0">
				<div className="mb-4 flex items-center justify-between text-xs text-muted-foreground">
					<span>
						{completed} of {items.length} complete
					</span>
					<CheckCircle2 className="size-4" />
				</div>
				<div className="mb-4 h-1.5 overflow-hidden rounded-full bg-muted">
					<div
						className="h-full rounded-full bg-[var(--home-accent,var(--primary))] transition-[width] duration-300 motion-reduce:transition-none"
						style={{
							width: `${items.length ? (completed / items.length) * 100 : 0}%`,
						}}
					/>
				</div>
				{items.map((item, index) => (
					<label
						key={item.id}
						htmlFor={`${widget.id}-check-${item.id}`}
						className="flex cursor-pointer items-start gap-3 rounded-lg px-1 py-2.5 transition-colors hover:bg-muted/40 has-disabled:cursor-default"
					>
						<Checkbox
							id={`${widget.id}-check-${item.id}`}
							aria-label={item.title}
							checked={Boolean(item.checked)}
							disabled={editing || !onUpdate}
							onCheckedChange={(checked) =>
								onUpdate?.({
									...widget.config,
									items: items.map((entry, entryIndex) =>
										entryIndex === index
											? { ...entry, checked: checked === true }
											: entry,
									),
								})
							}
							className="mt-0.5"
						/>
						<span
							className={cn(
								"min-w-0 text-sm leading-relaxed",
								item.checked && "text-muted-foreground line-through",
							)}
						>
							{item.title}
						</span>
					</label>
				))}
			</div>
		);
	}
	const prominent = ["banner", "announcement", "story", "quote"].includes(mode);
	const action = safeHomeHref(textConfig(widget.config, "actionHref"));
	const imageHref = safeHomeHref(textConfig(widget.config, "imageUrl"));
	const imageUrl =
		imageHref && /^(https?:|\/)/.test(imageHref) ? imageHref : undefined;
	return (
		<div className={cn("min-w-0", prominent && "flex flex-col gap-1")}>
			{imageUrl && (
				<HomeInformationImage
					src={imageUrl}
					alt={textConfig(widget.config, "imageAlt")}
				/>
			)}
			{textConfig(widget.config, "eyebrow") && (
				<p className="mb-3 text-[10px] font-medium uppercase tracking-[0.16em] text-primary">
					{textConfig(widget.config, "eyebrow")}
				</p>
			)}
			{mode === "announcement" && (
				<Megaphone className="mb-3 size-6 text-primary" />
			)}
			{body ? (
				<div
					className={cn(
						mode === "banner" &&
							"[&_p]:text-lg [&_p]:font-medium [&_p]:leading-relaxed",
						mode === "quote" && "border-l-2 border-primary/50 pl-4 italic",
					)}
				>
					<Markdown initialContent={body} isMarkdown minimal />
				</div>
			) : !imageUrl ? (
				<HomeEmpty>
					{mode === "image"
						? "Choose an image URL and add a description in widget settings."
						: "Add a note, links, or instructions in widget settings. Markdown formatting is supported."}
				</HomeEmpty>
			) : null}
			{mode === "quote" && textConfig(widget.config, "attribution") && (
				<p className="mt-4 text-xs text-muted-foreground">
					{textConfig(widget.config, "attribution")}
				</p>
			)}
			{action && (
				<Link
					href={action}
					target={action.startsWith("/") ? undefined : "_blank"}
					rel={action.startsWith("/") ? undefined : "noopener noreferrer"}
					className="mt-5 inline-flex w-fit items-center gap-2 rounded-lg bg-primary px-4 py-2 text-sm font-medium text-primary-foreground hover:opacity-90"
				>
					{textConfig(widget.config, "actionLabel", "Learn more")}
					<ArrowUpRight className="size-4" />
				</Link>
			)}
		</div>
	);
}

function HomeInformationImage({ src, alt }: { src: string; alt: string }) {
	const [failedSrc, setFailedSrc] = useState<string | null>(null);
	return failedSrc === src ? (
		<div className="mb-4 rounded-xl border border-dashed p-6 text-center text-xs text-muted-foreground">
			This image could not be loaded. Check its URL in widget settings.
		</div>
	) : (
		<img
			src={src}
			alt={alt}
			loading="lazy"
			referrerPolicy="no-referrer"
			onError={() => setFailedSrc(src)}
			className="mb-3 max-h-64 w-full shrink-0 rounded-xl object-cover"
		/>
	);
}
