"use client";

import { i18n as i18next, useTranslation } from "@flow-like/locales";
import { ExternalLink, Play } from "lucide-react";
import { useTheme } from "next-themes";
import { useEffect, useMemo, useRef, useState } from "react";
import { isTauri } from "../../../lib/platform";
import { cn } from "../../../lib/utils";

interface EmbedCodeBlockProps {
	content: string;
	className?: string;
}

interface EmbedConfig {
	url: string;
	autoplay?: boolean;
	start?: number;
}

type EmbedType =
	| "youtube"
	| "vimeo"
	| "twitter"
	| "reddit"
	| "github"
	| "stackoverflow"
	| "hackernews"
	| "linkedin"
	| "spotify"
	| "generic";

function parseEmbedContent(raw: string): EmbedConfig {
	const trimmed = raw.trim();

	if (
		(trimmed.startsWith("http://") ||
			trimmed.startsWith("https://") ||
			trimmed.startsWith("www.")) &&
		!trimmed.includes("\n")
	) {
		return { url: trimmed };
	}

	const config: EmbedConfig = { url: "" };
	for (const line of trimmed.split("\n")) {
		const colonIndex = line.indexOf(":");
		if (colonIndex === -1) {
			if (line.trim().startsWith("http")) config.url = line.trim();
			continue;
		}
		const key = line.slice(0, colonIndex).trim().toLowerCase();
		const value = line.slice(colonIndex + 1).trim();
		switch (key) {
			case "url":
				config.url = value;
				break;
			case "autoplay":
				config.autoplay = value === "true";
				break;
			case "start":
				config.start = Number.parseInt(value, 10);
				break;
		}
	}
	return config;
}

function matchesDomain(hostname: string, domain: string): boolean {
	return hostname === domain || hostname.endsWith(`.${domain}`);
}

function detectEmbedType(url: string): EmbedType {
	try {
		const hostname = new URL(url).hostname.toLowerCase();
		if (
			matchesDomain(hostname, "youtube.com") ||
			matchesDomain(hostname, "youtu.be") ||
			matchesDomain(hostname, "youtube-nocookie.com")
		)
			return "youtube";
		if (matchesDomain(hostname, "vimeo.com")) return "vimeo";
		if (
			matchesDomain(hostname, "twitter.com") ||
			matchesDomain(hostname, "x.com")
		)
			return "twitter";
		if (matchesDomain(hostname, "reddit.com")) return "reddit";
		if (matchesDomain(hostname, "github.com")) return "github";
		if (matchesDomain(hostname, "stackoverflow.com")) return "stackoverflow";
		if (hostname === "news.ycombinator.com") return "hackernews";
		if (matchesDomain(hostname, "linkedin.com")) return "linkedin";
		if (matchesDomain(hostname, "spotify.com")) return "spotify";
	} catch {
		// Invalid URL
	}
	return "generic";
}

function extractYouTubeId(url: string): string | null {
	try {
		const u = new URL(url);
		if (u.hostname.toLowerCase() === "youtu.be")
			return u.pathname.slice(1).split("/")[0] || null;
		return u.searchParams.get("v");
	} catch {
		return null;
	}
}

function extractVimeoId(url: string): string | null {
	try {
		const match = new URL(url).pathname.match(/\/(\d+)/);
		return match ? match[1] : null;
	} catch {
		return null;
	}
}

function extractSpotifyInfo(
	url: string,
): { embedUrl: string; type: string } | null {
	try {
		const u = new URL(url);
		const match = u.pathname.match(
			/^\/(track|album|playlist|episode|show)\/([a-zA-Z0-9]+)/,
		);
		if (match)
			return {
				embedUrl: `https://open.spotify.com/embed/${match[1]}/${match[2]}`,
				type: match[1],
			};
		return null;
	} catch {
		return null;
	}
}

function extractTweetId(url: string): string | null {
	try {
		const parts = new URL(url).pathname.split("/").filter(Boolean);
		if (parts[1] === "status" && parts[2]) return parts[2].split("?")[0];
		return null;
	} catch {
		return null;
	}
}

function buildRedditEmbedUrl(url: string, isDark: boolean): string | null {
	try {
		const u = new URL(url);
		const parts = u.pathname.split("/").filter(Boolean);
		if (parts[0] === "r" && parts[1] && parts[2] === "comments" && parts[3]) {
			const theme = isDark ? "dark" : "light";
			return `https://www.redditmedia.com${u.pathname}?ref_source=embed&ref=share&embed=true&theme=${theme}`;
		}
		return null;
	} catch {
		return null;
	}
}

function buildGitHubOgImage(url: string): string | null {
	try {
		const parts = new URL(url).pathname.split("/").filter(Boolean);
		if (parts.length < 2) return null;
		const [owner, repo, ...rest] = parts;
		const base = `https://opengraph.githubassets.com/1/${encodeURIComponent(owner)}/${encodeURIComponent(repo)}`;
		if (rest[0] === "issues" && rest[1])
			return `${base}/issues/${encodeURIComponent(rest[1])}`;
		if (rest[0] === "pull" && rest[1])
			return `${base}/pull/${encodeURIComponent(rest[1])}`;
		return base;
	} catch {
		return null;
	}
}

function buildLinkedInEmbedUrl(url: string): string | null {
	try {
		const path = new URL(url).pathname;
		const urnMatch = path.match(/\/feed\/update\/(urn:li:\w+:\d+)/);
		if (urnMatch)
			return `https://www.linkedin.com/embed/feed/update/${urnMatch[1]}`;
		const activityMatch = path.match(/activity-(\d+)/);
		if (activityMatch)
			return `https://www.linkedin.com/embed/feed/update/urn:li:activity:${activityMatch[1]}`;
		return null;
	} catch {
		return null;
	}
}

// ── Platform icons as inline SVGs ─────────────────────────────

function XIcon({ className }: { className?: string }) {
	return (
		<svg className={className} viewBox="0 0 24 24" fill="currentColor">
			<path d="M18.244 2.25h3.308l-7.227 8.26 8.502 11.24H16.17l-5.214-6.817L4.99 21.75H1.68l7.73-8.835L1.254 2.25H8.08l4.713 6.231zm-1.161 17.52h1.833L7.084 4.126H5.117z" />
		</svg>
	);
}

function RedditIcon({ className }: { className?: string }) {
	return (
		<svg className={className} viewBox="0 0 24 24" fill="currentColor">
			<path d="M12 0A12 12 0 0 0 0 12a12 12 0 0 0 12 12 12 12 0 0 0 12-12A12 12 0 0 0 12 0zm5.01 4.744c.688 0 1.25.561 1.25 1.249a1.25 1.25 0 0 1-2.498.056l-2.597-.547-.8 3.747c1.824.07 3.48.632 4.674 1.488.308-.309.73-.491 1.207-.491.968 0 1.754.786 1.754 1.754 0 .716-.435 1.333-1.01 1.614a3.111 3.111 0 0 1 .042.52c0 2.694-3.13 4.87-7.004 4.87-3.874 0-7.004-2.176-7.004-4.87 0-.183.015-.366.043-.534A1.748 1.748 0 0 1 4.028 12c0-.968.786-1.754 1.754-1.754.463 0 .898.196 1.207.49 1.207-.883 2.878-1.43 4.744-1.487l.885-4.182a.342.342 0 0 1 .14-.197.35.35 0 0 1 .238-.042l2.906.617a1.214 1.214 0 0 1 1.108-.701zM9.25 12C8.561 12 8 12.562 8 13.25c0 .687.561 1.248 1.25 1.248.687 0 1.248-.561 1.248-1.249 0-.688-.561-1.249-1.249-1.249zm5.5 0c-.687 0-1.248.561-1.248 1.25 0 .687.561 1.248 1.249 1.248.688 0 1.249-.561 1.249-1.249 0-.687-.562-1.249-1.25-1.249zm-5.466 3.99a.327.327 0 0 0-.231.094.33.33 0 0 0 0 .463c.842.842 2.484.913 2.961.913.477 0 2.105-.056 2.961-.913a.361.361 0 0 0 .029-.463.33.33 0 0 0-.464 0c-.547.533-1.684.73-2.512.73-.828 0-1.979-.196-2.512-.73a.326.326 0 0 0-.232-.095z" />
		</svg>
	);
}

function GitHubIcon({ className }: { className?: string }) {
	return (
		<svg className={className} viewBox="0 0 24 24" fill="currentColor">
			<path d="M12 .297c-6.63 0-12 5.373-12 12 0 5.303 3.438 9.8 8.205 11.385.6.113.82-.258.82-.577 0-.285-.01-1.04-.015-2.04-3.338.724-4.042-1.61-4.042-1.61C4.422 18.07 3.633 17.7 3.633 17.7c-1.087-.744.084-.729.084-.729 1.205.084 1.838 1.236 1.838 1.236 1.07 1.835 2.809 1.305 3.495.998.108-.776.417-1.305.76-1.605-2.665-.3-5.466-1.332-5.466-5.93 0-1.31.465-2.38 1.235-3.22-.135-.303-.54-1.523.105-3.176 0 0 1.005-.322 3.3 1.23.96-.267 1.98-.399 3-.405 1.02.006 2.04.138 3 .405 2.28-1.552 3.285-1.23 3.285-1.23.645 1.653.24 2.873.12 3.176.765.84 1.23 1.91 1.23 3.22 0 4.61-2.805 5.625-5.475 5.92.42.36.81 1.096.81 2.22 0 1.606-.015 2.896-.015 3.286 0 .315.21.69.825.57C20.565 22.092 24 17.592 24 12.297c0-6.627-5.373-12-12-12" />
		</svg>
	);
}

function SpotifyIcon({ className }: { className?: string }) {
	return (
		<svg className={className} viewBox="0 0 24 24" fill="currentColor">
			<path d="M12 0C5.4 0 0 5.4 0 12s5.4 12 12 12 12-5.4 12-12S18.66 0 12 0zm5.521 17.34c-.24.359-.66.48-1.021.24-2.82-1.74-6.36-2.101-10.561-1.141-.418.122-.779-.179-.899-.539-.12-.421.18-.78.54-.9 4.56-1.021 8.52-.6 11.64 1.32.42.18.479.659.301 1.02zm1.44-3.3c-.301.42-.841.6-1.262.3-3.239-1.98-8.159-2.58-11.939-1.38-.479.12-1.02-.12-1.14-.6-.12-.48.12-1.021.6-1.141C9.6 9.9 15 10.561 18.72 12.84c.361.181.54.78.241 1.2zm.12-3.36C15.24 8.4 8.82 8.16 5.16 9.301c-.6.179-1.2-.181-1.38-.721-.18-.601.18-1.2.72-1.381 4.26-1.26 11.28-1.02 15.721 1.621.539.3.719 1.02.419 1.56-.299.421-1.02.599-1.559.3z" />
		</svg>
	);
}

function StackOverflowIcon({ className }: { className?: string }) {
	return (
		<svg className={className} viewBox="0 0 24 24" fill="currentColor">
			<path d="M15.725 0l-1.72 1.277 6.39 8.588 1.716-1.277L15.725 0zm-3.94 3.418l-1.369 1.644 8.225 6.85 1.369-1.644-8.225-6.85zm-3.15 4.465l-.905 1.94 9.702 4.517.904-1.94-9.701-4.517zm-1.85 4.86l-.44 2.093 10.473 2.201.44-2.092-10.473-2.203zM1.89 15.47V24h19.19v-8.53h-2.133v6.397H4.021v-6.396H1.89zm4.265 2.133v2.13h10.66v-2.13H6.154z" />
		</svg>
	);
}

function LinkedInIcon({ className }: { className?: string }) {
	return (
		<svg className={className} viewBox="0 0 24 24" fill="currentColor">
			<path d="M20.447 20.452h-3.554v-5.569c0-1.328-.027-3.037-1.852-3.037-1.853 0-2.136 1.445-2.136 2.939v5.667H9.351V9h3.414v1.561h.046c.477-.9 1.637-1.85 3.37-1.85 3.601 0 4.267 2.37 4.267 5.455v6.286zM5.337 7.433c-1.144 0-2.063-.926-2.063-2.065 0-1.138.92-2.063 2.063-2.063 1.14 0 2.064.925 2.064 2.063 0 1.139-.925 2.065-2.064 2.065zm1.782 13.019H3.555V9h3.564v11.452zM22.225 0H1.771C.792 0 0 .774 0 1.729v20.542C0 23.227.792 24 1.771 24h20.451C23.2 24 24 23.227 24 22.271V1.729C24 .774 23.2 0 22.222 0h.003z" />
		</svg>
	);
}

function HackerNewsIcon({ className }: { className?: string }) {
	return (
		<svg className={className} viewBox="0 0 24 24" fill="currentColor">
			<path d="M0 24V0h24v24H0zM6.951 5.896l4.112 7.708v5.064h1.583v-4.972l4.148-7.799h-1.749l-2.457 4.875c-.372.745-.688 1.434-.688 1.434s-.297-.708-.651-1.434L8.831 5.896h-1.88z" />
		</svg>
	);
}

// ── Platform configuration ─────────────────────────────

interface PlatformConfig {
	name: string;
	icon: React.ReactNode;
	accentColor: string;
	bgClass: string;
	borderClass: string;
	description: (url: string) => string;
}

function getPlatformConfig(
	type: EmbedType,
	url: string,
): PlatformConfig | null {
	const configs: Partial<Record<EmbedType, PlatformConfig>> = {
		twitter: {
			name: i18next.t('xTwitter', 'X (Twitter)'),
			icon: <XIcon className="size-5" />,
			accentColor: "text-foreground",
			bgClass: "bg-neutral-500/5 dark:bg-neutral-500/10",
			borderClass: "border-neutral-400/30",
			description: () => parseTwitterUrl(url),
		},
		reddit: {
			name: i18next.t('reddit', 'Reddit'),
			icon: <RedditIcon className="size-5" />,
			accentColor: "text-orange-500",
			bgClass: "bg-orange-500/5 dark:bg-orange-500/10",
			borderClass: "border-orange-500/20",
			description: () => parseRedditUrl(url),
		},
		github: {
			name: i18next.t('github', 'GitHub'),
			icon: <GitHubIcon className="size-5" />,
			accentColor: "text-foreground",
			bgClass: "bg-neutral-500/5 dark:bg-neutral-500/10",
			borderClass: "border-neutral-400/30",
			description: () => parseGitHubUrl(url),
		},
		stackoverflow: {
			name: i18next.t('stackOverflow', 'Stack Overflow'),
			icon: <StackOverflowIcon className="size-5" />,
			accentColor: "text-orange-400",
			bgClass: "bg-orange-500/5 dark:bg-orange-400/10",
			borderClass: "border-orange-400/20",
			description: () => i18next.t('questionOnStackOverflow', 'Question on Stack Overflow'),
		},
		hackernews: {
			name: i18next.t('hackerNews', 'Hacker News'),
			icon: <HackerNewsIcon className="size-5" />,
			accentColor: "text-orange-500",
			bgClass: "bg-orange-500/5 dark:bg-orange-500/10",
			borderClass: "border-orange-500/20",
			description: () => i18next.t('discussionOnHackerNews', 'Discussion on Hacker News'),
		},
		linkedin: {
			name: i18next.t('linkedin', 'LinkedIn'),
			icon: <LinkedInIcon className="size-5" />,
			accentColor: "text-blue-600",
			bgClass: "bg-blue-500/5 dark:bg-blue-500/10",
			borderClass: "border-blue-500/20",
			description: () => i18next.t('postOnLinkedin', 'Post on LinkedIn'),
		},
	};
	return configs[type] ?? null;
}

function parseTwitterUrl(url: string): string {
	try {
		const u = new URL(url);
		const parts = u.pathname.split("/").filter(Boolean);
		if (parts.length >= 1) {
			const user = parts[0];
			if (parts[1] === "status" && parts[2]) return i18next.t('postByUser', 'Post by @{{user}}', { user });
			return i18next.t('userOnX', '@{{user}} on X', { user });
		}
	} catch {
		/* noop */
	}
	return i18next.t('postOnX', 'Post on X');
}

function parseRedditUrl(url: string): string {
	try {
		const u = new URL(url);
		const parts = u.pathname.split("/").filter(Boolean);
		if (parts[0] === "r" && parts[1]) {
			const sub = parts[1];
			if (parts[2] === "comments") return i18next.t('rsubThread', 'r/{{sub}} thread', { sub });
			return `r/${sub}`;
		}
		if (parts[0] === "u" || parts[0] === "user") return `u/${parts[1]}`;
	} catch {
		/* noop */
	}
	return i18next.t('threadOnReddit', 'Thread on Reddit');
}

function parseGitHubUrl(url: string): string {
	try {
		const u = new URL(url);
		const parts = u.pathname.split("/").filter(Boolean);
		if (parts.length >= 2) {
			const owner = parts[0];
			const repo = parts[1];
			if (parts[2] === "issues" && parts[3])
				return `${owner}/${repo} #${parts[3]}`;
			if (parts[2] === "pull" && parts[3])
				return `${owner}/${repo} PR #${parts[3]}`;
			if (parts[2] === "tree" || parts[2] === "blob")
				return `${owner}/${repo}/${parts.slice(3).join("/")}`;
			return `${owner}/${repo}`;
		}
		if (parts.length === 1) return parts[0];
	} catch {
		/* noop */
	}
	return i18next.t('repositoryOnGithub', 'Repository on GitHub');
}

// ── Embed components ────────────────────────────────────

function YouTubeEmbed({ config }: { config: EmbedConfig }) {
	const { t } = useTranslation("common");
	const [loaded, setLoaded] = useState(false);
	const videoId = extractYouTubeId(config.url);

	if (!videoId) return <GenericEmbed config={config} />;

	const thumbnailUrl = `https://img.youtube.com/vi/${videoId}/maxresdefault.jpg`;
	const params = new URLSearchParams();
	if (config.autoplay) params.set("autoplay", "1");
	if (config.start) params.set("start", String(config.start));
	const iframeSrc = `https://www.youtube-nocookie.com/embed/${videoId}?${params.toString()}`;

	if (!loaded) {
		return (
			<div className="relative aspect-video w-full overflow-hidden rounded-lg bg-black">
				<img
					src={thumbnailUrl}
					alt={t('youtubeVideoThumbnail', 'YouTube video thumbnail')}
					className="absolute inset-0 h-full w-full object-cover"
					loading="lazy"
				/>
				<button
					type="button"
					className="absolute inset-0 flex items-center justify-center bg-black/30 hover:bg-black/20 transition-colors"
					onClick={() => setLoaded(true)}
					aria-label={t('playVideo', 'Play video')}
				>
					<div className="flex size-16 items-center justify-center rounded-full bg-red-600 text-white shadow-lg">
						<Play className="size-8 ml-1" fill="white" />
					</div>
				</button>
				<OpenExternalLink url={config.url} />
			</div>
		);
	}

	return (
		<div className="relative aspect-video w-full overflow-hidden rounded-lg">
			<iframe
				src={iframeSrc}
				title={t('youtubeVideo', 'YouTube video')}
				className="absolute inset-0 h-full w-full border-0"
				allow="accelerometer; autoplay; clipboard-write; encrypted-media; gyroscope; picture-in-picture"
				allowFullScreen
				loading="lazy"
			/>
			<OpenExternalLink url={config.url} />
		</div>
	);
}

function VimeoEmbed({ config }: { config: EmbedConfig }) {
	const { t } = useTranslation("common");
	const [loaded, setLoaded] = useState(false);
	const videoId = extractVimeoId(config.url);

	if (!videoId) return <GenericEmbed config={config} />;

	if (!loaded) {
		return (
			<div className="relative aspect-video w-full overflow-hidden rounded-lg bg-muted">
				<button
					type="button"
					className="absolute inset-0 flex items-center justify-center hover:bg-muted/80 transition-colors"
					onClick={() => setLoaded(true)}
					aria-label={t('playVimeoVideo', 'Play Vimeo video')}
				>
					<div className="flex size-16 items-center justify-center rounded-full bg-blue-500 text-white shadow-lg">
						<Play className="size-8 ml-1" fill="white" />
					</div>
				</button>
				<OpenExternalLink url={config.url} />
			</div>
		);
	}

	return (
		<div className="relative aspect-video w-full overflow-hidden rounded-lg">
			<iframe
				src={`https://player.vimeo.com/video/${videoId}`}
				title={t('vimeoVideo', 'Vimeo video')}
				className="absolute inset-0 h-full w-full border-0"
				allow="autoplay; fullscreen; picture-in-picture"
				allowFullScreen
				loading="lazy"
			/>
			<OpenExternalLink url={config.url} />
		</div>
	);
}

function SpotifyEmbed({ config }: { config: EmbedConfig }) {
	const { t } = useTranslation("common");
	const info = extractSpotifyInfo(config.url);

	if (!info) {
		return (
			<BrandedLinkCard
				config={config}
				name="Spotify"
				icon={<SpotifyIcon className="size-5" />}
				accentColor="text-green-500"
				bgClass="bg-green-500/5 dark:bg-green-500/10"
				borderClass="border-green-500/20"
				description={t('contentOnSpotify', 'Content on Spotify')}
			/>
		);
	}

	const heights: Record<string, number> = {
		track: 152,
		album: 352,
		playlist: 352,
		episode: 232,
		show: 352,
	};

	return (
		<div className="my-2 overflow-hidden rounded-xl">
			<iframe
				src={`${info.embedUrl}?theme=0`}
				title={t('spotifyEmbed', 'Spotify embed')}
				className="w-full border-0 rounded-xl"
				style={{ height: heights[info.type] || 152 }}
				allow="autoplay; clipboard-write; encrypted-media; fullscreen; picture-in-picture"
				loading="lazy"
			/>
		</div>
	);
}

function TwitterEmbed({ config }: { config: EmbedConfig }) {
	const { t } = useTranslation("common");
	const { resolvedTheme } = useTheme();
	const tweetId = extractTweetId(config.url);
	const iframeRef = useRef<HTMLIFrameElement>(null);
	const [height, setHeight] = useState(350);

	if (!tweetId) return <PlatformEmbed config={config} type="twitter" />;

	const theme = resolvedTheme === "dark" ? "dark" : "light";

	return (
		<div
			className={cn(
				"my-2 overflow-hidden rounded-xl border",
				resolvedTheme === "dark"
					? "border-neutral-700 bg-neutral-900"
					: "border-neutral-200 bg-white",
			)}
		>
			<div className="overflow-y-auto" style={{ maxHeight: 500 }}>
				<iframe
					ref={iframeRef}
					src={`https://platform.twitter.com/embed/Tweet.html?dnt=true&id=${tweetId}&theme=${theme}`}
					title={t('xPost', 'X post')}
					className="w-full border-0"
					style={{ height }}
					loading="lazy"
					sandbox="allow-scripts allow-same-origin allow-popups"
					onLoad={() => {
						setTimeout(() => {
							if (height === 350) setHeight(450);
						}, 1500);
					}}
				/>
			</div>
			<div className="flex items-center justify-between px-4 py-2 border-t border-border/30">
				<div className="flex items-center gap-2">
					<XIcon className="size-4" />
					<span className="text-xs text-muted-foreground">
						{parseTwitterUrl(config.url)}
					</span>
				</div>
				<a
					href={config.url}
					target="_blank"
					rel="noopener noreferrer"
					className="text-xs text-muted-foreground hover:text-foreground flex items-center gap-1 no-underline"
				>
					<ExternalLink className="size-3" /> {t('open', 'Open')}
				</a>
			</div>
		</div>
	);
}

function RedditEmbed({ config }: { config: EmbedConfig }) {
	const { t } = useTranslation("common");
	const { resolvedTheme } = useTheme();
	const isDark = resolvedTheme === "dark";
	const embedUrl = buildRedditEmbedUrl(config.url, isDark);
	const iframeRef = useRef<HTMLIFrameElement>(null);
	const [height, setHeight] = useState(240);

	useEffect(() => {
		const handler = (event: MessageEvent) => {
			try {
				const data =
					typeof event.data === "string" ? JSON.parse(event.data) : event.data;
				if (
					data?.type === "resize.embed" &&
					typeof data.data === "number" &&
					data.data > 0
				) {
					setHeight(Math.min(data.data, 600));
				}
			} catch {
				/* ignore non-JSON messages */
			}
		};
		window.addEventListener("message", handler);
		return () => window.removeEventListener("message", handler);
	}, []);

	if (!embedUrl) return <PlatformEmbed config={config} type="reddit" />;

	return (
		<div className="my-2 overflow-hidden rounded-xl border border-orange-500/20 bg-orange-500/5 dark:bg-orange-500/10">
			<iframe
				ref={iframeRef}
				src={embedUrl}
				title={t('redditThread', 'Reddit thread')}
				className="w-full border-0"
				style={{ height }}
				loading="lazy"
				sandbox="allow-scripts allow-same-origin allow-popups"
			/>
			<div className="flex items-center justify-between px-4 py-2 border-t border-orange-500/10">
				<div className="flex items-center gap-2">
					<RedditIcon className="size-4 text-orange-500" />
					<span className="text-xs text-muted-foreground">
						{parseRedditUrl(config.url)}
					</span>
				</div>
				<a
					href={config.url}
					target="_blank"
					rel="noopener noreferrer"
					className="text-xs text-muted-foreground hover:text-foreground flex items-center gap-1 no-underline"
				>
					<ExternalLink className="size-3" /> {t('open', 'Open')}
				</a>
			</div>
		</div>
	);
}

function GitHubEmbed({ config }: { config: EmbedConfig }) {
	const ogImage = buildGitHubOgImage(config.url);
	const description = parseGitHubUrl(config.url);

	if (!ogImage) return <PlatformEmbed config={config} type="github" />;

	return (
		<a
			href={config.url}
			target="_blank"
			rel="noopener noreferrer"
			className="my-2 block overflow-hidden rounded-xl border border-neutral-400/30 bg-neutral-500/5 dark:bg-neutral-500/10 hover:opacity-90 transition-opacity no-underline group"
		>
			<img
				src={ogImage}
				alt={description}
				className="w-full aspect-2/1 object-cover"
				loading="lazy"
			/>
			<div className="flex items-center gap-2 px-4 py-3">
				<GitHubIcon className="size-4 shrink-0 text-foreground" />
				<span className="text-sm font-medium text-foreground truncate">
					{description}
				</span>
				<ExternalLink className="size-3.5 shrink-0 text-muted-foreground group-hover:text-foreground transition-colors ml-auto" />
			</div>
		</a>
	);
}

function LinkedInEmbed({ config }: { config: EmbedConfig }) {
	const { t } = useTranslation("common");
	const embedUrl = buildLinkedInEmbedUrl(config.url);

	if (!embedUrl) return <PlatformEmbed config={config} type="linkedin" />;

	return (
		<div className="my-2 overflow-hidden rounded-xl border border-blue-500/20 bg-blue-500/5 dark:bg-blue-500/10">
			<iframe
				src={embedUrl}
				title={t('linkedinPost', 'LinkedIn post')}
				className="w-full border-0"
				style={{ minHeight: 300 }}
				loading="lazy"
				sandbox="allow-scripts allow-same-origin allow-popups"
			/>
			<div className="flex items-center justify-between px-4 py-2 border-t border-blue-500/10">
				<div className="flex items-center gap-2">
					<LinkedInIcon className="size-4 text-blue-600" />
					<span className="text-xs text-muted-foreground">
						{t('postOnLinkedin', 'Post on LinkedIn')}
					</span>
				</div>
				<a
					href={config.url}
					target="_blank"
					rel="noopener noreferrer"
					className="text-xs text-muted-foreground hover:text-foreground flex items-center gap-1 no-underline"
				>
					<ExternalLink className="size-3" /> {t('open', 'Open')}
				</a>
			</div>
		</div>
	);
}

function BrandedLinkCard({
	config,
	name,
	icon,
	accentColor,
	bgClass,
	borderClass,
	description,
}: {
	config: EmbedConfig;
	name: string;
	icon: React.ReactNode;
	accentColor: string;
	bgClass: string;
	borderClass: string;
	description: string;
}) {
	return (
		<a
			href={config.url}
			target="_blank"
			rel="noopener noreferrer"
			className={cn(
				"my-2 flex items-center gap-3 rounded-lg border p-4 transition-colors hover:opacity-80 no-underline group",
				bgClass,
				borderClass,
			)}
		>
			<div className={cn("shrink-0", accentColor)}>{icon}</div>
			<div className="min-w-0 flex-1">
				<div className="flex items-center gap-1.5">
					<span className="text-sm font-semibold text-foreground">{name}</span>
				</div>
				<p className="text-xs text-muted-foreground mt-0.5 truncate">
					{description}
				</p>
				<p className="text-xs text-muted-foreground/60 mt-0.5 truncate">
					{config.url}
				</p>
			</div>
			<ExternalLink className="size-4 shrink-0 text-muted-foreground group-hover:text-foreground transition-colors" />
		</a>
	);
}

function PlatformEmbed({
	config,
	type,
}: { config: EmbedConfig; type: EmbedType }) {
	const platform = getPlatformConfig(type, config.url);
	if (!platform) return <GenericEmbed config={config} />;

	return (
		<BrandedLinkCard
			config={config}
			name={platform.name}
			icon={platform.icon}
			accentColor={platform.accentColor}
			bgClass={platform.bgClass}
			borderClass={platform.borderClass}
			description={platform.description(config.url)}
		/>
	);
}

interface OgData {
	title?: string;
	description?: string;
	image?: string;
}

export function parseOgFromHtml(html: string): OgData | null {
	const head = html.split(/<\/head>/i)[0] ?? html;

	const getMetaContent = (property: string): string | undefined => {
		// Escape special regex chars in property name
		const esc = property.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
		const patterns = [
			// Quoted: property="og:*" ... content="value"
			new RegExp(
				`<meta[^>]*property=["']${esc}["'][^>]*content=["']([^"']*)["']`,
				"i",
			),
			// Quoted reversed: content="value" ... property="og:*"
			new RegExp(
				`<meta[^>]*content=["']([^"']*)["'][^>]*property=["']${esc}["']`,
				"i",
			),
			// Quoted name=: name="description" ... content="value"
			new RegExp(
				`<meta[^>]*name=["']${esc}["'][^>]*content=["']([^"']*)["']`,
				"i",
			),
			// Quoted name= reversed
			new RegExp(
				`<meta[^>]*content=["']([^"']*)["'][^>]*name=["']${esc}["']`,
				"i",
			),
			// Unquoted property, quoted content: property=og:title content="value"
			new RegExp(
				`<meta[^>]*property=${esc}[\\s][^>]*content=["']([^"']*)["']`,
				"i",
			),
			// Unquoted property, quoted content reversed: content="value" property=og:title
			new RegExp(
				`<meta[^>]*content=["']([^"']*)["'][^>]*property=${esc}[\\s>]`,
				"i",
			),
			// Unquoted property, unquoted content: content=value property=og:title
			new RegExp(
				`<meta[^>]*content=([^\\s"'>]+)[^>]*property=${esc}[\\s>]`,
				"i",
			),
			// Unquoted both reversed: property=og:title content=value
			new RegExp(
				`<meta[^>]*property=${esc}[\\s][^>]*content=([^\\s"'>]+)`,
				"i",
			),
		];
		for (const pattern of patterns) {
			const match = head.match(pattern);
			if (match?.[1]) return match[1];
		}
		return undefined;
	};

	const title =
		getMetaContent("og:title") ??
		head.match(/<title[^>]*>([^<]*)<\/title>/i)?.[1]?.trim();
	const description =
		getMetaContent("og:description") ?? getMetaContent("description");
	const image = getMetaContent("og:image");

	if (!title && !description && !image) return null;
	return { title, description, image };
}

function useOgMetadata(url: string): { data: OgData | null; loading: boolean } {
	const [data, setData] = useState<OgData | null>(null);
	const [loading, setLoading] = useState(true);

	useEffect(() => {
		let cancelled = false;
		const controller = new AbortController();

		(async () => {
			try {
				let ogData: OgData | null = null;

				if (isTauri()) {
					const { fetch: tauriFetch } = await import("@tauri-apps/plugin-http");
					const res = await tauriFetch(url, {
						method: "GET",
						headers: {
							"User-Agent": "Mozilla/5.0 (compatible; FlowLikeBot/1.0)",
							Accept: "text/html",
						},
					});
					if (cancelled) return;
					const html = await res.text();
					if (cancelled) return;
					ogData = parseOgFromHtml(html);
				} else {
					const apiBase =
						process.env.NEXT_PUBLIC_API_URL ?? "https://api.flow-like.com";
					const res = await fetch(
						`${apiBase}/api/v1/og?url=${encodeURIComponent(url)}`,
						{ signal: controller.signal },
					);
					if (cancelled) return;
					const json = await res.json();
					if (cancelled) return;
					ogData = {
						title: json.title || undefined,
						description: json.description || undefined,
						image: json.image || undefined,
					};
				}

				if (ogData) setData(ogData);
			} catch {
				// Network error or aborted — keep fallback
			} finally {
				if (!cancelled) setLoading(false);
			}
		})();

		return () => {
			cancelled = true;
			controller.abort();
		};
	}, [url]);

	return { data, loading };
}

function GenericEmbed({ config }: { config: EmbedConfig }) {
	const faviconUrl = getFaviconUrl(config.url);
	const domainName = getDomainDisplayName(config.url);
	const { data: og, loading } = useOgMetadata(config.url);

	if (og?.image) {
		return (
			<a
				href={config.url}
				target="_blank"
				rel="noopener noreferrer"
				className="my-2 block overflow-hidden rounded-xl border border-border/50 bg-muted/20 hover:bg-muted/30 transition-colors no-underline group"
			>
				<img
					src={og.image}
					alt={og.title || domainName}
					className="w-full aspect-2/1 object-cover"
					loading="lazy"
				/>
				<div className="px-4 py-3 space-y-1">
					<div className="flex items-center gap-2">
						{faviconUrl && (
							<img
								src={faviconUrl}
								alt=""
								className="size-4 shrink-0 rounded-sm"
								loading="lazy"
							/>
						)}
						<span className="text-xs text-muted-foreground">{domainName}</span>
						<ExternalLink className="size-3 shrink-0 text-muted-foreground group-hover:text-foreground transition-colors ml-auto" />
					</div>
					{og.title && (
						<p className="text-sm font-medium text-foreground line-clamp-1">
							{og.title}
						</p>
					)}
					{og.description && (
						<p className="text-xs text-muted-foreground line-clamp-2">
							{og.description}
						</p>
					)}
				</div>
			</a>
		);
	}

	if (!loading && og?.title) {
		return (
			<a
				href={config.url}
				target="_blank"
				rel="noopener noreferrer"
				className="my-2 flex items-center gap-3 rounded-lg border border-border/50 bg-muted/20 p-4 hover:bg-muted/30 transition-colors no-underline group"
			>
				{faviconUrl && (
					<img
						src={faviconUrl}
						alt=""
						className="size-6 shrink-0 rounded-sm"
						loading="lazy"
					/>
				)}
				<div className="min-w-0 flex-1">
					<p className="text-sm font-medium text-foreground line-clamp-1">
						{og.title}
					</p>
					{og.description && (
						<p className="text-xs text-muted-foreground mt-0.5 line-clamp-2">
							{og.description}
						</p>
					)}
					<p className="text-xs text-muted-foreground/60 mt-0.5 truncate">
						{domainName}
					</p>
				</div>
				<ExternalLink className="size-4 shrink-0 text-muted-foreground group-hover:text-foreground transition-colors" />
			</a>
		);
	}

	return (
		<a
			href={config.url}
			target="_blank"
			rel="noopener noreferrer"
			className="my-2 flex items-center gap-3 rounded-lg border border-border/50 bg-muted/20 p-4 hover:bg-muted/30 transition-colors no-underline group"
		>
			{faviconUrl && (
				<img
					src={faviconUrl}
					alt=""
					className="size-6 shrink-0 rounded-sm"
					loading="lazy"
				/>
			)}
			<div className="min-w-0 flex-1">
				{domainName && (
					<p className="text-sm font-medium text-foreground">{domainName}</p>
				)}
				<p className="text-xs text-muted-foreground mt-0.5 truncate">
					{config.url}
				</p>
			</div>
			<ExternalLink className="size-4 shrink-0 text-muted-foreground group-hover:text-foreground transition-colors" />
		</a>
	);
}

function getDomainDisplayName(url: string): string {
	try {
		return new URL(url).hostname.replace("www.", "");
	} catch {
		return "Link";
	}
}

function getFaviconUrl(url: string): string {
	try {
		return `https://www.google.com/s2/favicons?domain=${encodeURIComponent(new URL(url).hostname)}&sz=32`;
	} catch {
		return "";
	}
}

function OpenExternalLink({ url }: { url: string }) {
	const { t } = useTranslation("common");
	return (
		<a
			href={url}
			target="_blank"
			rel="noopener noreferrer"
			className="absolute top-2 right-2 z-10 flex items-center gap-1 rounded bg-black/60 px-2 py-1 text-xs text-white hover:bg-black/80 transition-colors"
			aria-label={t('openInBrowser', 'Open in browser')}
		>
			<ExternalLink className="size-3" />
			<span>{t('open', 'Open')}</span>
		</a>
	);
}

export function EmbedCodeBlock({ content, className }: EmbedCodeBlockProps) {
	const { t } = useTranslation("common");
	const config = useMemo(() => parseEmbedContent(content), [content]);
	const embedType = useMemo(
		() => (config.url ? detectEmbedType(config.url) : "generic"),
		[config.url],
	);

	if (!config.url) {
		return (
			<div className={cn("p-4 text-sm text-muted-foreground", className)}>
				{t('noUrlProvidedForEmbedBlock', 'No URL provided for embed block.')}
			</div>
		);
	}

	switch (embedType) {
		case "youtube":
			return (
				<div className={className}>
					<YouTubeEmbed config={config} />
				</div>
			);
		case "vimeo":
			return (
				<div className={className}>
					<VimeoEmbed config={config} />
				</div>
			);
		case "spotify":
			return (
				<div className={className}>
					<SpotifyEmbed config={config} />
				</div>
			);
		case "twitter":
			return (
				<div className={className}>
					<TwitterEmbed config={config} />
				</div>
			);
		case "reddit":
			return (
				<div className={className}>
					<RedditEmbed config={config} />
				</div>
			);
		case "github":
			return (
				<div className={className}>
					<GitHubEmbed config={config} />
				</div>
			);
		case "linkedin":
			return (
				<div className={className}>
					<LinkedInEmbed config={config} />
				</div>
			);
		case "stackoverflow":
		case "hackernews":
			return (
				<div className={className}>
					<PlatformEmbed config={config} type={embedType} />
				</div>
			);
		default:
			return (
				<div className={className}>
					<GenericEmbed config={config} />
				</div>
			);
	}
}

export default EmbedCodeBlock;
