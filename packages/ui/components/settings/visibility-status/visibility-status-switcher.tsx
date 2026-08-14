"use client";

import { useTranslation } from "@flow-like/locales";
import {
	AlertTriangleIcon,
	ArrowRightIcon,
	ExternalLinkIcon,
	EyeIcon,
	InfoIcon,
	ShieldIcon,
	UsersIcon,
} from "lucide-react";
import type { ReactNode } from "react";
import { useCallback } from "react";
import { toast } from "sonner";
import type { IApp } from "../../../types";
import { IAppVisibility } from "../../../types";
import {
	AlertDialog,
	AlertDialogAction,
	AlertDialogCancel,
	AlertDialogContent,
	AlertDialogDescription,
	AlertDialogFooter,
	AlertDialogHeader,
	AlertDialogTitle,
	AlertDialogTrigger,
} from "../../ui/alert-dialog";
import { Button } from "../../ui/button";
import {
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
} from "../../ui/card";
import {
	type IVisibilityEntityNoun,
	VISIBILITY_META,
	getVisibilityTransitionWarning,
	getVisibilityTransitions,
} from "./visibility-meta";

/** Result shape of a reviewed change; anything falsy means "applied now". */
export interface VisibilityChangeOutcome {
	reviewRequested?: boolean;
}

export interface EntityVisibilitySwitcherProps {
	entityId: string;
	visibility: IAppVisibility;
	canEdit: boolean;
	entityNoun: IVisibilityEntityNoun;
	onVisibilityChange: (
		entityId: string,
		newVisibility: IAppVisibility,
	) => Promise<VisibilityChangeOutcome> | Promise<void>;
	availableTransitions?: IAppVisibility[];
	docsUrl?: string;
}

export function EntityVisibilitySwitcher({
	entityId,
	visibility,
	canEdit,
	entityNoun,
	onVisibilityChange,
	availableTransitions,
	docsUrl = "https://docs.flow-like.com/guides/Apps/visibility/",
}: Readonly<EntityVisibilitySwitcherProps>) {
	const { t } = useTranslation("settings");
	const currentConfig = VISIBILITY_META[visibility];
	const transitions =
		availableTransitions ?? getVisibilityTransitions(visibility);

	const confirmVisibilityChange = useCallback(
		async (newVisibility: IAppVisibility) => {
			if (visibility === newVisibility) return;
			const config = VISIBILITY_META[newVisibility];
			try {
				const outcome = await onVisibilityChange(entityId, newVisibility);
				if (outcome?.reviewRequested) {
					toast.success(t('submittedForReview', 'Submitted for review'), {
						description: t('yourEntitynounStaysTitleUntilTheReviewIsComplete', 'Your {{entityNoun}} stays {{title}} until the review is complete.', { entityNoun, title: VISIBILITY_META[visibility].title }),
					});
					return;
				}
				toast.success(t('visibilityChangedToTitle', 'Visibility changed to {{title}}', { title: config.title }), {
					icon: <config.Icon className="w-4 h-4" />,
				});
			} catch (error) {
				toast.error(
					error instanceof Error
						? error.message
						: t('couldNotChangeTheVisibility', 'Could not change the visibility'),
				);
			}
		},
		[entityId, entityNoun, onVisibilityChange, visibility],
	);

	if (!canEdit) {
		return null;
	}

	return (
		<Card>
			<CardHeader>
				<CardTitle className="flex items-center gap-2">
					<EyeIcon className="w-5 h-5" />
					{t('visibilityStatus', 'Visibility Status')}
				</CardTitle>
				<CardDescription>{t('controlWhoCanAccessYourEntitynounAndHowItapossShared', "Control who can access your {{entityNoun}} and how it's shared.", { entityNoun })}{" "}
					<a href={docsUrl} target="_blank" rel="noreferrer">
						<Button
							variant="link"
							className="h-auto p-0 text-xs text-muted-foreground hover:text-foreground"
						>
							{t('learnMoreAboutVisibilityStatuses', 'Learn more about visibility statuses')}
							<ExternalLinkIcon className="w-3 h-3 ml-1" />
						</Button>
					</a>
				</CardDescription>
			</CardHeader>
			<CardContent className="space-y-4">
				{/* Current Status */}
				<div className="flex items-center gap-3 p-4 bg-muted rounded-lg border">
					<div className={`w-3 h-3 rounded-full ${currentConfig.color}`} />
					<div>
						<div className="font-medium">{t('currentTitle', 'Current: {{title}}', { title: currentConfig.title })}</div>
						<div className="text-sm text-muted-foreground">
							{currentConfig.description}
						</div>
					</div>
				</div>

				{/* Available Transitions */}
				{transitions.length > 0 ? (
					<div className="space-y-3">
						<div className="text-sm font-medium text-muted-foreground">
							{t('availableTransitions', 'Available transitions:')}
						</div>
						<div className="grid gap-2">
							{transitions.map((target) => {
								const config = VISIBILITY_META[target];
								const warning = getVisibilityTransitionWarning(
									visibility,
									target,
									entityNoun,
								);

								return (
									<CustomVerificationDialog
										key={target}
										title={warning.title}
										description={warning.message}
										severity={warning.severity}
										confirmText="Change Visibility"
										onConfirm={() => confirmVisibilityChange(target)}
										content={
											<div className="flex items-center justify-center gap-2 p-3 bg-muted rounded-lg">
												<div
													className={`w-2 h-2 rounded-full ${currentConfig.color}`}
												/>
												<span className="text-sm font-medium">
													{currentConfig.title}
												</span>
												<ArrowRightIcon className="w-4 h-4 text-muted-foreground" />
												<div
													className={`w-2 h-2 rounded-full ${config.color}`}
												/>
												<span className="text-sm font-medium">
													{config.title}
												</span>
											</div>
										}
									>
										<Button
											variant="outline"
											className="w-full justify-between group hover:bg-muted/50 transition-colors h-fit"
										>
											<div className="flex items-center gap-3">
												<div
													className={`w-3 h-3 rounded-full ${config.color}`}
												/>
												<div className="text-left">
													<div className="font-medium">{config.title}</div>
													<div className="text-xs text-muted-foreground">
														{config.description}
													</div>
												</div>
											</div>
											<ArrowRightIcon className="w-4 h-4 opacity-0 group-hover:opacity-100 transition-opacity" />
										</Button>
									</CustomVerificationDialog>
								);
							})}
						</div>
					</div>
				) : (
					<div className="p-4 bg-muted/50 rounded-lg border-2 border-dashed border-muted-foreground/25">
						<div className="flex items-center gap-2 text-muted-foreground">
							<InfoIcon className="w-4 h-4" />
							<span className="text-sm">
								{visibility === IAppVisibility.Offline
									? `No transitions available from Offline status`
									: `No transitions available from current status`}
							</span>
						</div>
					</div>
				)}

				{/* Info about restrictions */}
				<div className="text-xs text-muted-foreground space-y-1 border-t pt-3">
					{entityNoun === "app" && (
						<div className="flex items-center gap-1">
							<ShieldIcon className="w-3 h-3" />
							<span>{t('offlineAppsCannotChangeVisibilityStatus', 'Offline apps cannot change visibility status')}</span>
						</div>
					)}
					<div className="flex items-center gap-1">
						<UsersIcon className="w-3 h-3" />
						<span>{t('publicTransitionsRequireCentralReview13Days', 'Public transitions require central review (1-3 days)')}</span>
					</div>
				</div>
			</CardContent>
		</Card>
	);
}

export interface VisibilityStatusSwitcherProps {
	localApp: IApp;
	canEdit: boolean;
	onVisibilityChange: (
		appId: string,
		newVisibility: IAppVisibility,
	) => Promise<void>;
	docsUrl?: string;
}

export function VisibilityStatusSwitcher({
	localApp,
	canEdit,
	onVisibilityChange,
	docsUrl,
}: Readonly<VisibilityStatusSwitcherProps>) {
	return (
		<EntityVisibilitySwitcher
			entityId={localApp.id}
			visibility={localApp.visibility ?? IAppVisibility.Offline}
			canEdit={canEdit}
			entityNoun="app"
			onVisibilityChange={onVisibilityChange}
			docsUrl={docsUrl}
		/>
	);
}

interface CustomVerificationDialogProps {
	children: ReactNode;
	title: string;
	description: string;
	severity: "warning" | "danger" | "info";
	confirmText?: string;
	cancelText?: string;
	onConfirm: () => void | Promise<void>;
	content?: ReactNode;
}

function CustomVerificationDialog({
	children,
	title,
	description,
	severity,
	confirmText = "Confirm",
	cancelText = "Cancel",
	onConfirm,
	content,
}: Readonly<CustomVerificationDialogProps>) {
	const getSeverityConfig = () => {
		switch (severity) {
			case "danger":
				return {
					icon: <AlertTriangleIcon className="h-5 w-5 text-destructive" />,
					iconBg: "bg-destructive/10",
					buttonVariant: "destructive" as const,
				};
			case "warning":
				return {
					icon: <AlertTriangleIcon className="h-5 w-5 text-orange-500" />,
					iconBg: "bg-orange-50 dark:bg-orange-950",
					buttonVariant: "default" as const,
				};
			default:
				return {
					icon: <InfoIcon className="h-5 w-5 text-blue-500" />,
					iconBg: "bg-blue-50 dark:bg-blue-950",
					buttonVariant: "default" as const,
				};
		}
	};

	const config = getSeverityConfig();

	return (
		<AlertDialog>
			<AlertDialogTrigger asChild>{children}</AlertDialogTrigger>
			<AlertDialogContent className="sm:max-w-[425px]">
				<AlertDialogHeader>
					<div className="flex items-center gap-3">
						<div className={`p-2 rounded-full ${config.iconBg}`}>
							{config.icon}
						</div>
						<AlertDialogTitle className="text-left">{title}</AlertDialogTitle>
					</div>
					<AlertDialogDescription className="text-left text-muted-foreground">
						{description}
					</AlertDialogDescription>
				</AlertDialogHeader>
				{content && <div className="py-4">{content}</div>}
				<AlertDialogFooter className="flex-col sm:flex-row gap-2">
					<AlertDialogCancel asChild>
						<Button variant="outline" className="w-full sm:w-auto">
							{cancelText}
						</Button>
					</AlertDialogCancel>
					<AlertDialogAction asChild>
						<Button
							variant={config.buttonVariant}
							onClick={onConfirm}
							className="w-full sm:w-auto"
						>
							{confirmText}
						</Button>
					</AlertDialogAction>
				</AlertDialogFooter>
			</AlertDialogContent>
		</AlertDialog>
	);
}
