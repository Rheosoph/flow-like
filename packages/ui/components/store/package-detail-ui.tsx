"use client";

import { formatDistanceToNow } from "date-fns";
import {
	ArrowLeft,
	Check,
	Download,
	ExternalLink,
	Github,
	Globe,
	Package,
	RefreshCw,
	Shield,
	Tag,
	User,
} from "lucide-react";
import type { ReactNode } from "react";
import { Badge } from "../ui/badge";
import { Button } from "../ui/button";
import {
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
} from "../ui/card";
import { Skeleton } from "../ui/skeleton";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "../ui/tabs";

export interface PackageManifestUI {
	name: string;
	description: string;
	keywords: string[];
	authors: Array<{ name: string; email?: string; url?: string }>;
	license?: string;
	repository?: string;
	homepage?: string;
	permissions: {
		network: boolean;
		filesystem: boolean;
		process: boolean;
		ffi: boolean;
	};
}

export interface PackageVersionUI {
	version: string;
	publishedAt: string;
	releaseNotes?: string;
	yanked: boolean;
}

export interface PackageDataUI {
	manifest: PackageManifestUI;
	nodes: Array<{
		id: string;
		name: string;
		description: string;
		category: string;
	}>;
	versions: PackageVersionUI[];
	downloadCount: number;
	verified: boolean;
}

export interface PackageDetailUIProps {
	pkg: PackageDataUI;
	installedVersion?: string | null;
	isInstalling?: boolean;
	isUninstalling?: boolean;
	onBack: () => void;
	onInstall: (version?: string) => void;
	onUninstall: () => void;
	headerExtra?: ReactNode;
}

function PermissionBadge({
	label,
	enabled,
}: { label: string; enabled: boolean }) {
	return (
		<Badge variant={enabled ? "default" : "outline"} className="gap-1">
			{enabled ? <Check className="h-3 w-3" /> : null}
			{label}
		</Badge>
	);
}

function NodeCard({
	node,
}: {
	node: { id: string; name: string; description: string; category: string };
}) {
	return (
		<Card>
			<CardHeader className="pb-2">
				<div className="flex items-start justify-between">
					<CardTitle className="text-sm font-medium">{node.name}</CardTitle>
					<Badge variant="outline" className="text-xs">
						{node.category}
					</Badge>
				</div>
			</CardHeader>
			<CardContent>
				<p className="text-xs text-muted-foreground">{node.description}</p>
			</CardContent>
		</Card>
	);
}

function VersionRow({
	version,
	isLatest,
}: {
	version: PackageVersionUI;
	isLatest: boolean;
}) {
	return (
		<div className="flex items-center justify-between py-2 border-b last:border-0">
			<div className="flex items-center gap-2">
				<code className="text-sm font-mono">{version.version}</code>
				{isLatest && <Badge variant="secondary">Latest</Badge>}
				{version.yanked && <Badge variant="destructive">Yanked</Badge>}
			</div>
			<span className="text-sm text-muted-foreground">
				{formatDistanceToNow(new Date(version.publishedAt), {
					addSuffix: true,
				})}
			</span>
		</div>
	);
}

export function PackageDetailUI({
	pkg,
	installedVersion,
	isInstalling = false,
	isUninstalling = false,
	onBack,
	onInstall,
	onUninstall,
	headerExtra,
}: PackageDetailUIProps) {
	const manifest = pkg.manifest;
	const latestVersion =
		pkg.versions.find((v) => !v.yanked)?.version ?? pkg.versions[0]?.version;
	const isInstalled = !!installedVersion;
	const hasUpdate = isInstalled && installedVersion !== latestVersion;

	return (
		<main className="flex-col flex flex-grow max-h-full p-6 overflow-auto min-h-0 w-full">
			<div className="mx-auto w-full max-w-5xl space-y-6">
				{/* Back Button */}
				<Button variant="ghost" onClick={onBack} className="gap-2">
					<ArrowLeft className="h-4 w-4" />
					Back
				</Button>

				{/* Header Card */}
				<Card>
					<CardHeader>
						<div className="flex flex-col md:flex-row md:items-start md:justify-between gap-4">
							<div className="flex items-start gap-4">
								<div className="p-3 rounded-lg bg-muted">
									<Package className="h-8 w-8" />
								</div>
								<div>
									<div className="flex items-center gap-2 flex-wrap">
										<CardTitle className="text-2xl">{manifest.name}</CardTitle>
										{pkg.verified && (
											<Badge variant="secondary" className="gap-1">
												<Shield className="h-3 w-3" />
												Verified
											</Badge>
										)}
									</div>
									<CardDescription className="mt-1">
										{manifest.description}
									</CardDescription>
									<div className="flex items-center gap-4 mt-2 text-sm text-muted-foreground">
										<span className="flex items-center gap-1">
											<Tag className="h-4 w-4" />v{latestVersion}
										</span>
										<span className="flex items-center gap-1">
											<Download className="h-4 w-4" />
											{pkg.downloadCount.toLocaleString()} downloads
										</span>
									</div>
								</div>
							</div>

							<div className="flex flex-col gap-2">
								{isInstalled ? (
									<>
										<div className="flex items-center gap-2 text-sm text-muted-foreground">
											<Check className="h-4 w-4 text-green-500" />
											Installed v{installedVersion}
										</div>
										{hasUpdate && (
											<Button
												onClick={() => onInstall(latestVersion)}
												disabled={isInstalling}
											>
												{isInstalling ? (
													<RefreshCw className="mr-2 h-4 w-4 animate-spin" />
												) : (
													<RefreshCw className="mr-2 h-4 w-4" />
												)}
												Update to v{latestVersion}
											</Button>
										)}
										<Button
											variant="destructive"
											onClick={onUninstall}
											disabled={isUninstalling}
										>
											Uninstall
										</Button>
									</>
								) : (
									<Button
										onClick={() => onInstall(undefined)}
										disabled={isInstalling}
									>
										{isInstalling ? (
											<RefreshCw className="mr-2 h-4 w-4 animate-spin" />
										) : (
											<Download className="mr-2 h-4 w-4" />
										)}
										Install
									</Button>
								)}
								{headerExtra}
							</div>
						</div>
					</CardHeader>
				</Card>

				{/* Main Content */}
				<Tabs defaultValue="overview" className="w-full">
					<TabsList>
						<TabsTrigger value="overview">Overview</TabsTrigger>
						<TabsTrigger value="nodes">
							Nodes ({pkg.nodes.length})
						</TabsTrigger>
						<TabsTrigger value="permissions">Permissions</TabsTrigger>
						<TabsTrigger value="versions">
							Versions ({pkg.versions.length})
						</TabsTrigger>
					</TabsList>

					<TabsContent value="overview" className="space-y-4">
						<div className="grid grid-cols-1 md:grid-cols-3 gap-4">
							{/* Info Card */}
							<Card className="md:col-span-2">
								<CardHeader>
									<CardTitle className="text-base">
										Package Information
									</CardTitle>
								</CardHeader>
								<CardContent className="space-y-4">
									{/* Keywords */}
									{manifest.keywords.length > 0 && (
										<div>
											<h4 className="text-sm font-medium mb-2">Keywords</h4>
											<div className="flex flex-wrap gap-1">
												{manifest.keywords.map((kw) => (
													<Badge key={kw} variant="outline">
														{kw}
													</Badge>
												))}
											</div>
										</div>
									)}

									{/* Authors */}
									{manifest.authors.length > 0 && (
										<div>
											<h4 className="text-sm font-medium mb-2">Authors</h4>
											<div className="flex flex-wrap gap-2">
												{manifest.authors.map((author, idx) => (
													<div
														key={idx}
														className="flex items-center gap-1 text-sm"
													>
														<User className="h-4 w-4 text-muted-foreground" />
														{author.url ? (
															<a
																href={author.url}
																target="_blank"
																rel="noopener noreferrer"
																className="hover:underline"
															>
																{author.name}
															</a>
														) : (
															<span>{author.name}</span>
														)}
													</div>
												))}
											</div>
										</div>
									)}

									{/* License */}
									{manifest.license && (
										<div>
											<h4 className="text-sm font-medium mb-2">License</h4>
											<Badge variant="outline">{manifest.license}</Badge>
										</div>
									)}
								</CardContent>
							</Card>

							{/* Links Card */}
							<Card>
								<CardHeader>
									<CardTitle className="text-base">Links</CardTitle>
								</CardHeader>
								<CardContent className="space-y-3">
									{manifest.repository && (
										<a
											href={manifest.repository}
											target="_blank"
											rel="noopener noreferrer"
											className="flex items-center gap-2 text-sm hover:underline"
										>
											<Github className="h-4 w-4" />
											Repository
											<ExternalLink className="h-3 w-3" />
										</a>
									)}
									{manifest.homepage && (
										<a
											href={manifest.homepage}
											target="_blank"
											rel="noopener noreferrer"
											className="flex items-center gap-2 text-sm hover:underline"
										>
											<Globe className="h-4 w-4" />
											Homepage
											<ExternalLink className="h-3 w-3" />
										</a>
									)}
									{!manifest.repository && !manifest.homepage && (
										<p className="text-sm text-muted-foreground">
											No links available
										</p>
									)}
								</CardContent>
							</Card>
						</div>
					</TabsContent>

					<TabsContent value="nodes">
						{pkg.nodes.length === 0 ? (
							<Card>
								<CardContent className="pt-6">
									<p className="text-sm text-muted-foreground">
										This package doesn&apos;t export any nodes.
									</p>
								</CardContent>
							</Card>
						) : (
							<div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-4">
								{pkg.nodes.map((node) => (
									<NodeCard key={node.id} node={node} />
								))}
							</div>
						)}
					</TabsContent>

					<TabsContent value="permissions">
						<Card>
							<CardHeader>
								<CardTitle className="text-base flex items-center gap-2">
									<Shield className="h-5 w-5" />
									Package Permissions
								</CardTitle>
								<CardDescription>
									This package requests the following permissions
								</CardDescription>
							</CardHeader>
							<CardContent>
								<div className="flex flex-wrap gap-2">
									<PermissionBadge
										label="Network"
										enabled={manifest.permissions.network}
									/>
									<PermissionBadge
										label="Filesystem"
										enabled={manifest.permissions.filesystem}
									/>
									<PermissionBadge
										label="Process"
										enabled={manifest.permissions.process}
									/>
									<PermissionBadge
										label="FFI"
										enabled={manifest.permissions.ffi}
									/>
								</div>
							</CardContent>
						</Card>
					</TabsContent>

					<TabsContent value="versions">
						<Card>
							<CardHeader>
								<CardTitle className="text-base">Version History</CardTitle>
							</CardHeader>
							<CardContent>
								{pkg.versions.length === 0 ? (
									<p className="text-sm text-muted-foreground">
										No versions available
									</p>
								) : (
									<div>
										{pkg.versions.map((version, idx) => (
											<VersionRow
												key={version.version}
												version={version}
												isLatest={idx === 0 && !version.yanked}
											/>
										))}
									</div>
								)}
							</CardContent>
						</Card>
					</TabsContent>
				</Tabs>
			</div>
		</main>
	);
}

export function PackageDetailSkeleton() {
	return (
		<main className="flex-col flex flex-grow max-h-full p-6 overflow-auto min-h-0 w-full">
			<div className="mx-auto w-full max-w-5xl space-y-6">
				<div className="flex items-center gap-4">
					<Skeleton className="h-9 w-24" />
				</div>
				<Skeleton className="h-32 w-full" />
				<Skeleton className="h-64 w-full" />
			</div>
		</main>
	);
}
