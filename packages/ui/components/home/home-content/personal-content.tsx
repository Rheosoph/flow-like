"use client";

import { useQueryClient } from "@tanstack/react-query";
import {
	ArrowUp,
	ArrowUpRight,
	BookOpen,
	Box,
	CalendarDays,
	CheckCircle2,
	FileUp,
	Library,
	Link2,
	Megaphone,
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
import { Textarea } from "../../ui/textarea";
import {
	type HomeContentProps,
	safeHomeHref,
	stringList,
	textConfig,
} from "./config";
import { HomeEmpty, homeItemClass } from "./shared";

const Hero = dynamic(
	() =>
		import("../../global-chat/hero-variants/hero-bubble").then(
			(module) => module.HeroSearchBarBubble,
		),
	{ ssr: false },
);
const Markdown = dynamic(
	() => import("../../ui/text-editor").then((module) => module.TextEditor),
	{ ssr: false },
);

export function HomeFlowPilot({ widget, editing }: HomeContentProps) {
	const mode = textConfig(widget.config, "mode", "bar");
	const composer = useHeroComposer();
	const openOverlay = useGlobalChatStore((state) => state.openOverlay);
	const orbState = useFlowPilotOrbState();
	if (mode === "hero")
		return (
			<div className="h-full overflow-auto">
				<Hero />
			</div>
		);
	if (mode === "orb")
		return (
			<div className="flex h-full items-center justify-center gap-4 p-4">
				<FlowPilotBubbleOrb
					onClick={openOverlay}
					disabled={editing}
					orbState={orbState}
				/>
				<div>
					<p className="font-semibold">{widget.title || "Ask FlowPilot"}</p>
					<p className="mt-1 text-xs text-muted-foreground">
						An idea starts here.
					</p>
				</div>
			</div>
		);
	return (
		<form
			onSubmit={(event) => {
				event.preventDefault();
				if (!editing) composer.submit(composer.value);
			}}
			className={cn(
				"flex h-full flex-col justify-center gap-3 p-4",
				mode === "card" && "p-5",
			)}
		>
			{mode === "card" && (
				<div className="mb-1">
					<Sparkles className="mb-3 size-6 text-primary" />
					<h3 className="text-xl font-semibold tracking-tight">
						{widget.title || "What would you like to do?"}
					</h3>
					<p className="mt-1 text-sm text-muted-foreground">
						Build an app, find a package, or work through an idea.
					</p>
				</div>
			)}
			<div className="flex items-end gap-3 rounded-2xl border border-primary/25 bg-background/60 p-2.5 shadow-sm">
				{mode !== "card" && (
					<Sparkles className="mb-2 size-5 shrink-0 text-primary" />
				)}
				<Textarea
					value={composer.value}
					onChange={(event) => composer.setValue(event.target.value)}
					placeholder={textConfig(
						widget.config,
						"placeholder",
						"Ask FlowPilot to build, explore, or explain…",
					)}
					aria-label="Message FlowPilot"
					rows={mode === "card" ? 3 : 1}
					className="max-h-40 min-h-9 resize-none border-0 bg-transparent p-1.5 text-sm shadow-none focus-visible:ring-0"
					onKeyDown={(event) => {
						if (event.key === "Enter" && !event.shiftKey) {
							event.preventDefault();
							if (!editing) composer.submit(composer.value);
						}
					}}
				/>
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
			{mode === "card" && (
				<div className="flex flex-wrap gap-1.5">
					{["Create an app", "Find a package", "Explain a node"].map(
						(prompt) => (
							<Button
								key={prompt}
								type="button"
								size="sm"
								variant="outline"
								className="h-7 rounded-full text-[11px]"
								disabled={editing}
								onClick={() => composer.submit(prompt)}
							>
								{prompt}
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
	const [now, setNow] = useState<Date | null>(null);
	useEffect(() => {
		setNow(new Date());
		const timer = setInterval(() => setNow(new Date()), 60_000);
		return () => clearInterval(timer);
	}, []);
	const hour = now?.getHours() ?? 12;
	const greeting =
		hour < 12 ? "Good morning" : hour < 18 ? "Good afternoon" : "Good evening";
	const name =
		textConfig(widget.config, "name") ||
		auth.user?.profile.given_name ||
		auth.user?.profile.name?.split(" ")[0];
	return (
		<div className="flex h-full items-center justify-between gap-4 px-1 py-2">
			<div>
				<p className="text-[10px] font-medium uppercase tracking-[0.16em] text-muted-foreground">
					{now?.toLocaleDateString(undefined, {
						weekday: "long",
						day: "numeric",
						month: "long",
					}) ?? "Welcome"}
				</p>
				<h1 className="mt-1 text-2xl font-semibold tracking-tight md:text-3xl">
					{greeting}
					{name ? `, ${name}` : ""}
				</h1>
				{textConfig(widget.config, "subtitle") && (
					<p className="mt-1 text-sm text-muted-foreground">
						{textConfig(widget.config, "subtitle")}
					</p>
				)}
			</div>
			<Sun className="hidden size-7 text-primary/60 sm:block" />
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
		title: "Import with FlowPilot",
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
			<div className="grid h-full content-center grid-cols-[repeat(auto-fit,minmax(min(100%,160px),1fr))] gap-3">
				{actions.map((id) => {
					const action =
						HOME_QUICK_ACTIONS[id as keyof typeof HOME_QUICK_ACTIONS];
					if (!action) return null;
					const Icon = action.icon;
					const content = (
						<Fragment key={id}>
							<Icon className="mb-3 size-5 text-primary" />
							<span className="block text-sm font-semibold">
								{action.title}
							</span>
							<span className="mt-1 block text-xs text-muted-foreground">
								{action.description}
							</span>
						</Fragment>
					);
					return action.href ? (
						<Link
							key={id}
							href={action.href}
							className={cn(homeItemClass, "block p-4")}
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
								"block p-4 text-left disabled:opacity-60",
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
	if (!links.length)
		return <HomeEmpty>Add useful links in widget settings.</HomeEmpty>;
	return (
		<div
			className={cn(
				"h-full overflow-auto p-3",
				widget.appearance.variant === "grid"
					? "grid content-start grid-cols-[repeat(auto-fit,minmax(min(100%,180px),1fr))] gap-2"
					: "space-y-2",
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
						className={homeItemClass}
					>
						{content}
					</Link>
				) : (
					<div key={link.id} className={cn(homeItemClass, "opacity-50")}>
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
			<div className="flex h-full flex-col justify-center p-5">
				<CalendarDays className="mb-4 size-6 text-primary" />
				<p className="text-4xl font-semibold tracking-tight">
					{days === null ? "…" : Math.abs(days)}
					<span className="ml-2 text-sm font-normal text-muted-foreground">
						{days !== null && days < 0
							? "days ago"
							: days === 0
								? "days. Today is the day."
								: "days to go"}
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
		if (!items.length)
			return <HomeEmpty>Add items in widget settings.</HomeEmpty>;
		return (
			<div
				className={cn(
					"h-full overflow-auto p-4",
					["resources", "facts"].includes(mode)
						? "grid content-start grid-cols-[repeat(auto-fit,minmax(min(100%,180px),1fr))] gap-3"
						: "space-y-3",
				)}
			>
				{items.map((item, index) => {
					const href = item.href ? safeHomeHref(item.href) : undefined;
					return (
						<article
							key={item.id}
							className={cn(
								"rounded-xl border border-border/60 bg-background/30 p-4",
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
									{item.title}
								</h3>
								{item.body && (
									<div className="mt-2 text-sm text-muted-foreground">
										<Markdown initialContent={item.body} isMarkdown minimal />
									</div>
								)}
								{href && (
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
			<div className="h-full space-y-2 overflow-auto p-4">
				{items.length ? (
					items.map((item, index) => (
						<details
							key={item.id}
							className="rounded-xl border bg-background/30 p-3"
						>
							<summary className="cursor-pointer text-sm font-medium">
								{item.title}
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
		return (
			<div className="h-full overflow-auto p-4">
				<div className="mb-4 flex items-center justify-between text-xs text-muted-foreground">
					<span>
						{completed} of {items.length} complete
					</span>
					<CheckCircle2 className="size-4" />
				</div>
				<div className="mb-4 h-1.5 overflow-hidden rounded-full bg-muted">
					<div
						className="h-full rounded-full bg-primary transition-all"
						style={{
							width: `${items.length ? (completed / items.length) * 100 : 0}%`,
						}}
					/>
				</div>
				{items.map((item, index) => (
					<label
						key={item.id}
						className="flex cursor-pointer items-start gap-3 border-b border-border/50 py-3 last:border-0"
					>
						<input
							type="checkbox"
							checked={Boolean(item.checked)}
							disabled={editing || !onUpdate}
							onChange={(event) =>
								onUpdate?.({
									...widget.config,
									items: items.map((entry, entryIndex) =>
										entryIndex === index
											? { ...entry, checked: event.target.checked }
											: entry,
									),
								})
							}
							className="mt-0.5 size-4 accent-primary"
						/>
						<span
							className={cn(
								"text-sm",
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
		<div
			className={cn(
				"h-full overflow-auto p-4",
				prominent && "flex flex-col p-6",
				mode === "banner" &&
					"[&_p]:text-lg [&_p]:font-medium [&_p]:leading-relaxed",
			)}
		>
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
						mode === "quote" && "border-l-2 border-primary/50 pl-4 italic",
					)}
				>
					<Markdown initialContent={body} isMarkdown minimal />
				</div>
			) : (
				<HomeEmpty>
					{mode === "image"
						? "Choose an image URL and add a description in widget settings."
						: "Add a note, links, or instructions in widget settings. Markdown formatting is supported."}
				</HomeEmpty>
			)}
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
			className="mb-4 max-h-64 w-full shrink-0 rounded-xl object-cover"
		/>
	);
}
