"use client";

import { Trans, useTranslation } from "@flow-like/locales";
import {
	AlertCircle,
	Check,
	Copy,
	Eye,
	EyeOff,
	Key,
	Loader2,
	Plus,
	RefreshCw,
	Shield,
	ShieldOff,
	Trash2,
} from "lucide-react";
import { useCallback, useMemo, useState } from "react";
import {
	type IListTokensResponse,
	type IRegisterSinkRequest,
	type IRegisterSinkResponse,
	type IRevokeSinkResponse,
	type ISinkTokenInfo,
	SINK_TYPES,
	SINK_TYPE_DESCRIPTIONS,
	SINK_TYPE_LABELS,
	type ServiceSinkType,
} from "../../../lib/schema/sink/sink-token";
import { Badge } from "../../ui/badge";
import { Button } from "../../ui/button";
import {
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
} from "../../ui/card";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogFooter,
	DialogHeader,
	DialogTitle,
} from "../../ui/dialog";
import { Input } from "../../ui/input";
import { Label } from "../../ui/label";
import {
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "../../ui/select";
import { Skeleton } from "../../ui/skeleton";
import {
	Table,
	TableBody,
	TableCell,
	TableHead,
	TableHeader,
	TableRow,
} from "../../ui/table";
import { Textarea } from "../../ui/textarea";
import {
	Tooltip,
	TooltipContent,
	TooltipProvider,
	TooltipTrigger,
} from "../../ui/tooltip";

export interface SinkTokensPageProps {
	data: IListTokensResponse | undefined;
	isLoading: boolean;
	error: Error | null;
	sinkTypeFilter: ServiceSinkType | undefined;
	includeRevoked: boolean;
	onSinkTypeFilterChange: (sinkType: ServiceSinkType | undefined) => void;
	onIncludeRevokedChange: (includeRevoked: boolean) => void;
	onRefresh: () => void;
	onRegisterToken: (
		request: IRegisterSinkRequest,
	) => Promise<IRegisterSinkResponse>;
	onRevokeToken: (jti: string) => Promise<IRevokeSinkResponse>;
}

export function SinkTokensPage({
	data,
	isLoading,
	error,
	sinkTypeFilter,
	includeRevoked,
	onSinkTypeFilterChange,
	onIncludeRevokedChange,
	onRefresh,
	onRegisterToken,
	onRevokeToken,
}: Readonly<SinkTokensPageProps>) {
	const { t } = useTranslation("settings");
	const [isCreateDialogOpen, setIsCreateDialogOpen] = useState(false);
	const [isRevokeDialogOpen, setIsRevokeDialogOpen] = useState(false);
	const [tokenToRevoke, setTokenToRevoke] = useState<ISinkTokenInfo | null>(
		null,
	);
	const [newToken, setNewToken] = useState<IRegisterSinkResponse | null>(null);
	const [showToken, setShowToken] = useState(false);
	const [copiedToken, setCopiedToken] = useState(false);

	// Form state
	const [selectedServiceSinkType, setSelectedServiceSinkType] =
		useState<ServiceSinkType>("cron");
	const [tokenName, setTokenName] = useState("");
	const [isSubmitting, setIsSubmitting] = useState(false);

	const activeTokens = useMemo(() => {
		if (!data?.tokens) return [];
		return data.tokens.filter((t) => !t.revoked);
	}, [data]);

	const revokedTokens = useMemo(() => {
		if (!data?.tokens) return [];
		return data.tokens.filter((t) => t.revoked);
	}, [data]);

	const handleCreateToken = useCallback(async () => {
		if (!selectedServiceSinkType) return;
		setIsSubmitting(true);
		try {
			const response = await onRegisterToken({
				sink_type: selectedServiceSinkType,
				name: tokenName || undefined,
			});
			setNewToken(response);
			setTokenName("");
			onRefresh();
		} catch {
			// Error handled by parent
		} finally {
			setIsSubmitting(false);
		}
	}, [selectedServiceSinkType, tokenName, onRegisterToken, onRefresh]);

	const handleRevokeToken = useCallback(async () => {
		if (!tokenToRevoke) return;
		setIsSubmitting(true);
		try {
			await onRevokeToken(tokenToRevoke.jti);
			setIsRevokeDialogOpen(false);
			setTokenToRevoke(null);
			onRefresh();
		} catch {
			// Error handled by parent
		} finally {
			setIsSubmitting(false);
		}
	}, [tokenToRevoke, onRevokeToken, onRefresh]);

	const handleCopyToken = useCallback(() => {
		if (newToken?.token) {
			navigator.clipboard.writeText(newToken.token);
			setCopiedToken(true);
			setTimeout(() => setCopiedToken(false), 2000);
		}
	}, [newToken]);

	const handleCloseCreateDialog = useCallback(() => {
		setIsCreateDialogOpen(false);
		setNewToken(null);
		setShowToken(false);
		setCopiedToken(false);
	}, []);

	const formatDate = (dateStr: string) => {
		return new Date(dateStr).toLocaleDateString(undefined, {
			year: "numeric",
			month: "short",
			day: "numeric",
			hour: "2-digit",
			minute: "2-digit",
		});
	};

	if (error) {
		return (
			<Card className="border-destructive">
				<CardHeader>
					<CardTitle className="flex items-center gap-2 text-destructive">
						<AlertCircle className="h-5 w-5" />
						{t('errorLoadingTokens', 'Error Loading Tokens')}
					</CardTitle>
					<CardDescription>{error.message}</CardDescription>
				</CardHeader>
				<CardContent>
					<Button onClick={onRefresh} variant="outline">
						<RefreshCw className="mr-2 h-4 w-4" />
						{t('retry', 'Retry')}
					</Button>
				</CardContent>
			</Card>
		);
	}

	return (
		<div className="space-y-6">
			{/* Header */}
			<div className="flex flex-col gap-4 sm:flex-row sm:items-center sm:justify-between">
				<div>
					<h1 className="text-2xl font-bold tracking-tight">
						{t('sinkServiceTokens', 'Sink Service Tokens')}
					</h1>
					<p className="text-muted-foreground">
						{t('manageAuthenticationTokensForExternalSinkServicesCronJobsWebhooksEtc', "Manage authentication tokens for external sink services (cron jobs, webhooks, etc.)")}
					</p>
				</div>
				<div className="flex gap-2">
					<Button onClick={onRefresh} variant="outline" size="sm">
						<RefreshCw className="mr-2 h-4 w-4" />
						{t('refresh', 'Refresh')}
					</Button>
					<Button onClick={() => setIsCreateDialogOpen(true)} size="sm">
						<Plus className="mr-2 h-4 w-4" />
						{t('createToken', 'Create Token')}
					</Button>
				</div>
			</div>

			{/* Filters */}
			<Card>
				<CardContent className="pt-6">
					<div className="flex flex-wrap gap-4">
						<div className="flex items-center gap-2">
							<Label htmlFor="sink-type-filter" className="whitespace-nowrap">
								{t('sinkType', 'Sink Type:')}
							</Label>
							<Select
								value={sinkTypeFilter ?? "all"}
								onValueChange={(v) =>
									onSinkTypeFilterChange(
										v === "all" ? undefined : (v as ServiceSinkType),
									)
								}
							>
								<SelectTrigger id="sink-type-filter" className="w-40">
									<SelectValue placeholder={t('allTypes', 'All types')} />
								</SelectTrigger>
								<SelectContent>
									<SelectItem value="all">{t('allTypes', 'All types')}</SelectItem>
									{SINK_TYPES.map((type) => (
										<SelectItem key={type} value={type}>
											{SINK_TYPE_LABELS[type]}
										</SelectItem>
									))}
								</SelectContent>
							</Select>
						</div>
						<div className="flex items-center gap-2">
							<input
								type="checkbox"
								id="include-revoked"
								checked={includeRevoked}
								onChange={(e) => onIncludeRevokedChange(e.target.checked)}
								className="h-4 w-4 rounded border-gray-300"
							/>
							<Label htmlFor="include-revoked">{t('showRevokedTokens', 'Show revoked tokens')}</Label>
						</div>
					</div>
				</CardContent>
			</Card>

			{/* Stats */}
			<div className="grid gap-4 md:grid-cols-3">
				<Card>
					<CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
						<CardTitle className="text-sm font-medium">{t('activeTokens', 'Active Tokens')}</CardTitle>
						<Shield className="h-4 w-4 text-green-500" />
					</CardHeader>
					<CardContent>
						{isLoading ? (
							<Skeleton className="h-8 w-16" />
						) : (
							<div className="text-2xl font-bold">{activeTokens.length}</div>
						)}
					</CardContent>
				</Card>
				<Card>
					<CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
						<CardTitle className="text-sm font-medium">
							{t('revokedTokens', 'Revoked Tokens')}
						</CardTitle>
						<ShieldOff className="h-4 w-4 text-destructive" />
					</CardHeader>
					<CardContent>
						{isLoading ? (
							<Skeleton className="h-8 w-16" />
						) : (
							<div className="text-2xl font-bold">{revokedTokens.length}</div>
						)}
					</CardContent>
				</Card>
				<Card>
					<CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
						<CardTitle className="text-sm font-medium">{t('totalTokens', 'Total Tokens')}</CardTitle>
						<Key className="h-4 w-4 text-muted-foreground" />
					</CardHeader>
					<CardContent>
						{isLoading ? (
							<Skeleton className="h-8 w-16" />
						) : (
							<div className="text-2xl font-bold">{data?.total ?? 0}</div>
						)}
					</CardContent>
				</Card>
			</div>

			{/* Tokens Table */}
			<Card>
				<CardHeader>
					<CardTitle>{t('tokens', 'Tokens')}</CardTitle>
					<CardDescription>
						{t('eachTokenIsScopedToASpecificSinkTypeAndCanBeIndividuallyRevoked', "Each token is scoped to a specific sink type and can be individually revoked.")}
					</CardDescription>
				</CardHeader>
				<CardContent>
					{isLoading ? (
						<div className="space-y-2">
							{[...Array(3)].map((_, i) => (
								<Skeleton key={i} className="h-12 w-full" />
							))}
						</div>
					) : !data?.tokens?.length ? (
						<div className="flex flex-col items-center justify-center py-12 text-center">
							<Key className="h-12 w-12 text-muted-foreground mb-4" />
							<h3 className="text-lg font-semibold">{t('noTokensFound', 'No tokens found')}</h3>
							<p className="text-sm text-muted-foreground mb-4">
								{t('createYourFirstSinkServiceTokenToGetStarted', 'Create your first sink service token to get started.')}
							</p>
							<Button onClick={() => setIsCreateDialogOpen(true)}>
								<Plus className="mr-2 h-4 w-4" />
								{t('createToken', 'Create Token')}
							</Button>
						</div>
					) : (
						<Table>
							<TableHeader>
								<TableRow>
									<TableHead>{t('nameJti', 'Name / JTI')}</TableHead>
									<TableHead>{t('sinkType2', 'Sink Type')}</TableHead>
									<TableHead>{t('status', 'Status')}</TableHead>
									<TableHead>{t('created', 'Created')}</TableHead>
									<TableHead className="text-right">{t('actions', 'Actions')}</TableHead>
								</TableRow>
							</TableHeader>
							<TableBody>
								{data.tokens.map((token) => (
									<TableRow
										key={token.jti}
										className={token.revoked ? "opacity-60" : ""}
									>
										<TableCell>
											<div className="flex flex-col">
												{token.name && (
													<span className="font-medium">{token.name}</span>
												)}
												<code className="text-xs text-muted-foreground font-mono">
													{token.jti}
												</code>
											</div>
										</TableCell>
										<TableCell>
											<Badge variant="outline">
												{SINK_TYPE_LABELS[token.sink_type as ServiceSinkType] ??
													token.sink_type}
											</Badge>
										</TableCell>
										<TableCell>
											{token.revoked ? (
												<TooltipProvider>
													<Tooltip>
														<TooltipTrigger>
															<Badge variant="destructive">{t('revoked', 'Revoked')}</Badge>
														</TooltipTrigger>
														<TooltipContent>
															<p>
																{t('revoked', 'Revoked')}{" "}
																{token.revoked_at
																	? formatDate(token.revoked_at)
																	: ""}
															</p>
															{token.revoked_by && (
																<p className="text-xs">{t('byRevoked_by', 'By: {{revoked_by}}', { revoked_by: token.revoked_by })}</p>
															)}
														</TooltipContent>
													</Tooltip>
												</TooltipProvider>
											) : (
												<Badge
													variant="outline"
													className="border-green-500 text-green-600"
												>
													{t('active', 'Active')}
												</Badge>
											)}
										</TableCell>
										<TableCell>
											<span className="text-sm text-muted-foreground">
												{formatDate(token.created_at)}
											</span>
										</TableCell>
										<TableCell className="text-right">
											{!token.revoked && (
												<Button
													variant="ghost"
													size="sm"
													className="text-destructive hover:text-destructive"
													onClick={() => {
														setTokenToRevoke(token);
														setIsRevokeDialogOpen(true);
													}}
												>
													<Trash2 className="h-4 w-4" />
													<span className="sr-only">{t('revoke', 'Revoke')}</span>
												</Button>
											)}
										</TableCell>
									</TableRow>
								))}
							</TableBody>
						</Table>
					)}
				</CardContent>
			</Card>

			{/* Create Token Dialog */}
			<Dialog open={isCreateDialogOpen} onOpenChange={handleCloseCreateDialog}>
				<DialogContent className="sm:max-w-lg">
					<DialogHeader>
						<DialogTitle>
							{newToken ? t('tokenCreatedSuccessfully', 'Token Created Successfully') : t('createSinkToken', 'Create Sink Token')}
						</DialogTitle>
						<DialogDescription>
							{newToken
								? t('copyAndSecurelyStoreThisTokenItWillNotBeShownAgain', 'Copy and securely store this token. It will not be shown again.')
								: `Create a new authentication token for a sink service.`}
						</DialogDescription>
					</DialogHeader>

					{newToken ? (
						<div className="space-y-4">
							<div className="rounded-md bg-green-50 dark:bg-green-950 p-4 border border-green-200 dark:border-green-800">
								<div className="flex items-start gap-3">
									<Check className="h-5 w-5 text-green-600 mt-0.5" />
									<div className="flex-1">
										<p className="text-sm font-medium text-green-800 dark:text-green-200">
											{t('tokenCreatedFor', 'Token created for')}{" "}
											{SINK_TYPE_LABELS[newToken.sink_type as ServiceSinkType]}
										</p>
										<p className="text-xs text-green-700 dark:text-green-300 mt-1">{t('jtiJti', 'JTI: {{jti}}', { jti: newToken.jti })}</p>
									</div>
								</div>
							</div>

							<div className="space-y-2">
								<Label>{t('tokenClickToReveal', 'Token (click to reveal)')}</Label>
								<div className="relative">
									<Textarea
										readOnly
										value={showToken ? newToken.token : "•".repeat(50)}
										className="font-mono text-xs pr-20 resize-none"
										rows={3}
										onClick={() => setShowToken(true)}
									/>
									<div className="absolute right-2 top-2 flex gap-1">
										<Button
											variant="ghost"
											size="icon"
											className="h-8 w-8"
											onClick={() => setShowToken(!showToken)}
										>
											{showToken ? (
												<EyeOff className="h-4 w-4" />
											) : (
												<Eye className="h-4 w-4" />
											)}
										</Button>
										<Button
											variant="ghost"
											size="icon"
											className="h-8 w-8"
											onClick={handleCopyToken}
										>
											{copiedToken ? (
												<Check className="h-4 w-4 text-green-500" />
											) : (
												<Copy className="h-4 w-4" />
											)}
										</Button>
									</div>
								</div>
								<p className="text-xs text-muted-foreground">
									{t('storeThisTokenSecurelyYouWonapostBeAbleToSeeItAgainAfterClosingThisDialog', "Store this token securely. You won't be able to see it again after closing this dialog.")}
								</p>
							</div>

							<DialogFooter>
								<Button onClick={handleCloseCreateDialog}>{t('done', 'Done')}</Button>
							</DialogFooter>
						</div>
					) : (
						<div className="space-y-4">
							<div className="space-y-2">
								<Label htmlFor="sink-type">{t('sinkType2', 'Sink Type')}</Label>
								<Select
									value={selectedServiceSinkType}
									onValueChange={(v) =>
										setSelectedServiceSinkType(v as ServiceSinkType)
									}
								>
									<SelectTrigger id="sink-type">
										<SelectValue />
									</SelectTrigger>
									<SelectContent>
										{SINK_TYPES.map((type) => (
											<SelectItem key={type} value={type}>
												<div className="flex flex-col">
													<span>{SINK_TYPE_LABELS[type]}</span>
													<span className="text-xs text-muted-foreground">
														{SINK_TYPE_DESCRIPTIONS[type]}
													</span>
												</div>
											</SelectItem>
										))}
									</SelectContent>
								</Select>
							</div>

							<div className="space-y-2">
								<Label htmlFor="token-name"><Trans i18nKey="nameSpanClassnametextmutedforegroundoptionalspan">Name <span className="text-muted-foreground">(optional)</span></Trans></Label>
								<Input
									id="token-name"
									placeholder={t('egProductionCronService', 'e.g., Production Cron Service')}
									value={tokenName}
									onChange={(e) => setTokenName(e.target.value)}
								/>
								<p className="text-xs text-muted-foreground">
									{t('aHumanreadableNameToIdentifyThisToken', 'A human-readable name to identify this token.')}
								</p>
							</div>

							<DialogFooter>
								<Button
									variant="outline"
									onClick={handleCloseCreateDialog}
									disabled={isSubmitting}
								>
									{t('cancel', 'Cancel')}
								</Button>
								<Button onClick={handleCreateToken} disabled={isSubmitting}>
									{isSubmitting ? (
										<>
											<Loader2 className="mr-2 h-4 w-4 animate-spin" />
											Creating...
										</>
									) : (
										<>
											<Plus className="mr-2 h-4 w-4" />
											{t('createToken', 'Create Token')}
										</>
									)}
								</Button>
							</DialogFooter>
						</div>
					)}
				</DialogContent>
			</Dialog>

			{/* Revoke Confirmation Dialog */}
			<Dialog open={isRevokeDialogOpen} onOpenChange={setIsRevokeDialogOpen}>
				<DialogContent>
					<DialogHeader>
						<DialogTitle>{t('revokeToken', 'Revoke Token')}</DialogTitle>
						<DialogDescription>
							{t('areYouSureYouWantToRevokeThisTokenThisActionCannotBeUndone', "Are you sure you want to revoke this token? This action cannot be undone.")}
						</DialogDescription>
					</DialogHeader>

					{tokenToRevoke && (
						<div className="rounded-md bg-destructive/10 p-4 border border-destructive/20">
							<div className="space-y-1">
								{tokenToRevoke.name && (
									<p className="font-medium">{tokenToRevoke.name}</p>
								)}
								<p className="text-sm text-muted-foreground font-mono">
									{tokenToRevoke.jti}
								</p>
								<p className="text-sm">
									{t('type', 'Type:')}{" "}
									{SINK_TYPE_LABELS[
										tokenToRevoke.sink_type as ServiceSinkType
									] ?? tokenToRevoke.sink_type}
								</p>
							</div>
						</div>
					)}

					<p className="text-sm text-muted-foreground">
						{t('anyServiceUsingThisTokenWillImmediatelyLoseAccessToTriggerEvents', "Any service using this token will immediately lose access to trigger events.")}
					</p>

					<DialogFooter>
						<Button
							variant="outline"
							onClick={() => setIsRevokeDialogOpen(false)}
							disabled={isSubmitting}
						>
							{t('cancel', 'Cancel')}
						</Button>
						<Button
							variant="destructive"
							onClick={handleRevokeToken}
							disabled={isSubmitting}
						>
							{isSubmitting ? (
								<>
									<Loader2 className="mr-2 h-4 w-4 animate-spin" />
									Revoking...
								</>
							) : (
								<>
									<Trash2 className="mr-2 h-4 w-4" />
									{t('revokeToken', 'Revoke Token')}
								</>
							)}
						</Button>
					</DialogFooter>
				</DialogContent>
			</Dialog>
		</div>
	);
}
