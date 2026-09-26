"use client";

import {
	Badge,
	Tooltip,
	TooltipContent,
	TooltipTrigger,
	cn,
} from "@flow-like/flow-like-ui";
import type { PackageManifest } from "@flow-like/flow-like-ui/lib/schema/wasm";
import { i18n as i18next, useTranslation } from "@flow-like/locales";
import {
	Clock,
	Code2,
	Globe,
	HardDrive,
	Lock,
	Shield,
	Sparkles,
	Zap,
} from "lucide-react";

function getPermissionMeta(permission: string): {
	label: string;
	description: string;
	className: string;
	icon: typeof Globe;
} {
	switch (permission) {
		case "network:http":
			return {
				label: "HTTP",
				description: i18next.t(
					"allowsOutboundHttpRequestsDuringDebugExecution",
					"Allows outbound HTTP requests during debug execution.",
				),
				className: "text-amber-600 border-amber-500/30",
				icon: Globe,
			};
		case "network:websocket":
			return {
				label: i18next.t("websocket", "WebSocket"),
				description: i18next.t(
					"allowsWebsocketConnectionsFromTheNodeSandbox",
					"Allows WebSocket connections from the node sandbox.",
				),
				className: "text-amber-600 border-amber-500/30",
				icon: Globe,
			};
		case "network:tcp":
			return {
				label: "TCP",
				description: i18next.t(
					"allowsOutboundTcpSocketsForThisNode",
					"Allows outbound TCP sockets for this node.",
				),
				className: "text-amber-600 border-amber-500/30",
				icon: Globe,
			};
		case "network:udp":
			return {
				label: "UDP",
				description: i18next.t(
					"allowsOutboundUdpSocketsForThisNode",
					"Allows outbound UDP sockets for this node.",
				),
				className: "text-amber-600 border-amber-500/30",
				icon: Globe,
			};
		case "network:dns":
			return {
				label: "DNS",
				description: i18next.t(
					"allowsHostnameResolutionDuringExecution",
					"Allows hostname resolution during execution.",
				),
				className: "text-amber-600 border-amber-500/30",
				icon: Globe,
			};
		case "storage:read":
			return {
				label: i18next.t("storageRead", "Storage Read"),
				description: i18next.t(
					"allowsReadingNodeOrUserStorageThroughFlowlikeApis",
					"Allows reading node or user storage through Flow-Like APIs.",
				),
				className: "text-blue-600 border-blue-500/30",
				icon: HardDrive,
			};
		case "storage:write":
			return {
				label: i18next.t("storageWrite", "Storage Write"),
				description: i18next.t(
					"allowsWritingNodeOrUserStorageThroughFlowlikeApis",
					"Allows writing node or user storage through Flow-Like APIs.",
				),
				className: "text-blue-600 border-blue-500/30",
				icon: HardDrive,
			};
		case "variables":
			return {
				label: i18next.t("variables", "Variables"),
				description: i18next.t(
					"allowsReadingAndWritingFlowVariables",
					"Allows reading and writing flow variables.",
				),
				className: "text-slate-600 border-slate-500/30",
				icon: HardDrive,
			};
		case "cache":
			return {
				label: i18next.t("cache", "Cache"),
				description: i18next.t(
					"allowsAccessToExecutionCacheEntries",
					"Allows access to execution cache entries.",
				),
				className: "text-slate-600 border-slate-500/30",
				icon: HardDrive,
			};
		case "streaming":
			return {
				label: i18next.t("streaming", "Streaming"),
				description: i18next.t(
					"allowsIncrementalOutputStreamingWhileTheNodeRuns",
					"Allows incremental output streaming while the node runs.",
				),
				className: "text-emerald-600 border-emerald-500/30",
				icon: Zap,
			};
		case "models":
			return {
				label: i18next.t("models", "Models"),
				description: i18next.t(
					"allowsInvokingLlmOrVlmModelProvidersFromTheHost",
					"Allows invoking LLM or VLM model providers from the host.",
				),
				className: "text-fuchsia-600 border-fuchsia-500/30",
				icon: Sparkles,
			};
		case "a2ui":
			return {
				label: "A2UI",
				description: i18next.t(
					"allowsAgenttouiRenderingFeatures",
					"Allows agent-to-UI rendering features.",
				),
				className: "text-sky-600 border-sky-500/30",
				icon: Shield,
			};
		case "oauth":
			return {
				label: i18next.t("oauth", "OAuth"),
				description: i18next.t(
					"allowsAccessToOauthbackedCredentials",
					"Allows access to OAuth-backed credentials.",
				),
				className: "text-orange-600 border-orange-500/30",
				icon: Lock,
			};
		case "functions":
			return {
				label: i18next.t("functions", "Functions"),
				description: i18next.t(
					"allowsInvokingOtherFunctionsOrSubflows",
					"Allows invoking other functions or sub-flows.",
				),
				className: "text-violet-600 border-violet-500/30",
				icon: Code2,
			};
		default:
			return {
				label: permission,
				description: i18next.t(
					"customNodeCapabilityDeclaredByTheWasmNode",
					"Custom node capability declared by the WASM node.",
				),
				className: "text-muted-foreground border-border/30",
				icon: Shield,
			};
	}
}

export function NodePermissionBadges({
	permissions,
	showEmpty = false,
}: {
	permissions: string[];
	showEmpty?: boolean;
}) {
	const { t } = useTranslation("common");
	if (permissions.length === 0) {
		if (!showEmpty) return null;
		return (
			<Badge variant="outline" className="text-xs text-muted-foreground/70">
				{t("noExtraPermissions", "No extra permissions")}
			</Badge>
		);
	}

	return (
		<div className="flex flex-wrap gap-1.5">
			{permissions.map((permission) => {
				const meta = getPermissionMeta(permission);
				const Icon = meta.icon;
				return (
					<Badge
						key={permission}
						variant="outline"
						className={cn("gap-1 text-xs", meta.className)}
					>
						<Icon className="h-3 w-3" />
						{meta.label}
					</Badge>
				);
			})}
		</div>
	);
}

export function PermissionsBadges({ manifest }: { manifest: PackageManifest }) {
	const { t } = useTranslation("common");
	const p = manifest.permissions;
	return (
		<div className="flex flex-wrap gap-1.5">
			<Tooltip>
				<TooltipTrigger>
					<Badge variant="outline" className="gap-1 text-xs">
						<HardDrive className="h-3 w-3" />
						{p.memory}
					</Badge>
				</TooltipTrigger>
				<TooltipContent>{t("memoryTier2", "Memory tier")}</TooltipContent>
			</Tooltip>
			<Tooltip>
				<TooltipTrigger>
					<Badge variant="outline" className="gap-1 text-xs">
						<Clock className="h-3 w-3" />
						{p.timeout}
					</Badge>
				</TooltipTrigger>
				<TooltipContent>{t("timeoutTier2", "Timeout tier")}</TooltipContent>
			</Tooltip>
			{p.network?.httpEnabled && (
				<Badge
					variant="outline"
					className="gap-1 text-xs text-amber-600 border-amber-500/30"
				>
					<Globe className="h-3 w-3" />
					{"HTTP"}
				</Badge>
			)}
			{(p.filesystem?.nodeStorage || p.filesystem?.userStorage) && (
				<Badge
					variant="outline"
					className="gap-1 text-xs text-blue-600 border-blue-500/30"
				>
					<HardDrive className="h-3 w-3" />
					{t("storage", "Storage")}
				</Badge>
			)}
			{p.streaming && (
				<Badge variant="outline" className="gap-1 text-xs">
					<Zap className="h-3 w-3" />
					{t("streaming", "Streaming")}
				</Badge>
			)}
			{p.models && (
				<Badge
					variant="outline"
					className="gap-1 text-xs text-purple-600 border-purple-500/30"
				>
					<Sparkles className="h-3 w-3" />
					{t("models", "Models")}
				</Badge>
			)}
			{p.variables && (
				<Badge variant="outline" className="gap-1 text-xs">
					{t("variables", "Variables")}
				</Badge>
			)}
			{p.cache && (
				<Badge variant="outline" className="gap-1 text-xs">
					{t("cache", "Cache")}
				</Badge>
			)}
			{p.a2ui && (
				<Badge variant="outline" className="gap-1 text-xs">
					{"A2UI"}
				</Badge>
			)}
			{p.oauthScopes?.length > 0 && (
				<Tooltip>
					<TooltipTrigger>
						<Badge
							variant="outline"
							className="gap-1 text-xs text-orange-600 border-orange-500/30"
						>
							<Lock className="h-3 w-3" />
							{t("oauthLength", "OAuth ({{length}})", {
								length: p.oauthScopes.length,
							})}
						</Badge>
					</TooltipTrigger>
					<TooltipContent>
						{p.oauthScopes
							.map((s) => `${s.provider}: ${s.scopes.join(", ")}`)
							.join("\n")}
					</TooltipContent>
				</Tooltip>
			)}
		</div>
	);
}

export function PermissionsDetail({ manifest }: { manifest: PackageManifest }) {
	const { t } = useTranslation("common");
	const p = manifest.permissions;
	return (
		<div className="grid grid-cols-1 sm:grid-cols-2 gap-4 text-sm">
			<div className="space-y-2">
				<h4 className="font-medium flex items-center gap-1.5">
					<HardDrive className="h-3.5 w-3.5" /> {t("resources", "Resources")}
				</h4>
				<div className="space-y-1 text-muted-foreground/70">
					<div className="flex justify-between">
						<span>{t("memory", "Memory")}</span>
						<span className="font-mono">{p.memory}</span>
					</div>
					<div className="flex justify-between">
						<span>{t("timeout", "Timeout")}</span>
						<span className="font-mono">{p.timeout}</span>
					</div>
				</div>
			</div>
			<div className="space-y-2">
				<h4 className="font-medium flex items-center gap-1.5">
					<Globe className="h-3.5 w-3.5" /> {t("network", "Network")}
				</h4>
				<div className="space-y-1 text-muted-foreground/70">
					<div className="flex justify-between">
						<span>{"HTTP"}</span>
						<span>{p.network?.httpEnabled ? "Yes" : "No"}</span>
					</div>
					{p.network?.allowedHosts?.length > 0 && (
						<div>
							<span className="text-xs">
								{t("allowedHosts", "Allowed hosts:")}
							</span>
							<div className="flex flex-wrap gap-1 mt-0.5">
								{p.network.allowedHosts.map((h) => (
									<Badge key={h} variant="outline" className="text-[10px]">
										{h}
									</Badge>
								))}
							</div>
						</div>
					)}
				</div>
			</div>
			<div className="space-y-2">
				<h4 className="font-medium flex items-center gap-1.5">
					<HardDrive className="h-3.5 w-3.5" /> {t("filesystem", "Filesystem")}
				</h4>
				<div className="space-y-1 text-muted-foreground/70">
					{(
						[
							[t("nodeStorage", "Node Storage"), p.filesystem?.nodeStorage],
							["User Storage", p.filesystem?.userStorage],
							[t("uploadDir", "Upload Dir"), p.filesystem?.uploadDir],
							["Cache Dir", p.filesystem?.cacheDir],
						] as const
					).map(([label, enabled]) => (
						<div key={label} className="flex justify-between">
							<span>{label}</span>
							<span>{enabled ? "Yes" : "No"}</span>
						</div>
					))}
				</div>
			</div>
			<div className="space-y-2">
				<h4 className="font-medium flex items-center gap-1.5">
					<Zap className="h-3.5 w-3.5" /> {t("capabilities", "Capabilities")}
				</h4>
				<div className="space-y-1 text-muted-foreground/70">
					{(
						[
							["Variables", p.variables],
							["Cache", p.cache],
							["Streaming", p.streaming],
							["A2UI", p.a2ui],
							["Models/LLM", p.models],
						] as const
					).map(([label, enabled]) => (
						<div key={label} className="flex justify-between">
							<span>{label}</span>
							<span>{enabled ? "Yes" : "No"}</span>
						</div>
					))}
				</div>
			</div>
		</div>
	);
}

export function NodePermissionsDetail({
	permissions,
}: {
	permissions: string[];
}) {
	const { t } = useTranslation("common");
	if (permissions.length === 0) {
		return (
			<div className="rounded-xl border border-border/20 bg-muted/5 p-4">
				<p className="text-sm text-muted-foreground/70">
					{t(
						"thisNodeDoesPureComputationInTheDebugSandboxAndDoesNotRequestAnyExtraHostCapabilities",
						"This node does pure computation in the debug sandbox and does not request any extra host capabilities.",
					)}
				</p>
			</div>
		);
	}

	return (
		<div className="grid grid-cols-1 sm:grid-cols-2 gap-3">
			{permissions.map((permission) => {
				const meta = getPermissionMeta(permission);
				const Icon = meta.icon;
				return (
					<div
						key={permission}
						className="rounded-xl border border-border/20 bg-muted/5 p-4 space-y-2"
					>
						<div className="flex items-center gap-2">
							<Icon className="h-4 w-4 text-muted-foreground/70" />
							<span className="text-sm font-medium">{meta.label}</span>
						</div>
						<p className="text-xs text-muted-foreground/60">
							{meta.description}
						</p>
					</div>
				);
			})}
		</div>
	);
}

export function NodePermissionsSummary({
	permissions,
	title = "Permissions",
	description,
	className,
}: {
	permissions: string[];
	title?: string;
	description?: string;
	className?: string;
}) {
	return (
		<div
			className={cn(
				"rounded-xl border border-border/20 bg-muted/5 p-3",
				className,
			)}
		>
			<div className="flex items-center justify-between gap-3">
				<div className="min-w-0">
					<div className="flex items-center gap-2">
						<Shield className="h-3.5 w-3.5 text-muted-foreground/60" />
						<span className="text-xs font-medium uppercase tracking-widest text-muted-foreground/60">
							{title}
						</span>
					</div>
					{description && (
						<p className="mt-1 text-xs text-muted-foreground/60">
							{description}
						</p>
					)}
				</div>
				<Badge variant="outline" className="text-[10px] shrink-0">
					{permissions.length === 0
						? "Pure"
						: `${permissions.length} ${permissions.length === 1 ? "permission" : "permissions"}`}
				</Badge>
			</div>
			<div className="mt-3">
				<NodePermissionBadges permissions={permissions} showEmpty />
			</div>
		</div>
	);
}
