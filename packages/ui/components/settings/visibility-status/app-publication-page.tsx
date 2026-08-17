"use client";

import { i18n as i18next, useTranslation } from "@flow-like/locales";
import {
	ArrowLeft,
	CheckCircle2,
	Clock3,
	ExternalLink,
	FileText,
	MessageSquare,
	PauseCircle,
	Send,
	XCircle,
} from "lucide-react";
import type { ReactNode } from "react";
import { useFeatures } from "../../../hooks/use-features";
import {
	userAvatarUrl,
	userDisplayName,
	userInitials,
} from "../../../lib/user-display";
import {
	Avatar,
	AvatarFallback,
	AvatarImage,
	Badge,
	Button,
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
	RelativeTime,
	Separator,
	Skeleton,
} from "../../ui";
import { AppAiActWizard } from "./app-ai-act-wizard";
import type {
	AppPublicationLogItem,
	AppPublicationRequestItem,
} from "./app-publication-review-card";

function formatLabel(value: string) {
	return value.replaceAll("_", " ");
}

function statusConfig(status: string): {
	variant: "default" | "secondary" | "destructive";
	icon: ReactNode;
	color: string;
	label: string;
	description: string;
} {
	switch (status) {
		case "accepted":
			return {
				variant: "default",
				icon: <CheckCircle2 className="h-5 w-5 text-green-600" />,
				color: "border-green-500/30 bg-green-500/5",
				label: i18next.t('accepted', 'Accepted'),
				description:
					i18next.t('yourAppHasBeenApprovedAndTheVisibilityChangeIsNowActive', 'Your app has been approved and the visibility change is now active.'),
			};
		case "rejected":
			return {
				variant: "destructive",
				icon: <XCircle className="h-5 w-5 text-red-600" />,
				color: "border-red-500/30 bg-red-500/5",
				label: i18next.t('rejected', 'Rejected'),
				description:
					i18next.t('yourRequestWasNotApprovedReviewTheAuditorFeedbackBelowAndTryAgainAfterMakingTheSuggestedChanges', 'Your request was not approved. Review the auditor feedback below and try again after making the suggested changes.'),
			};
		case "on_hold":
			return {
				variant: "secondary",
				icon: <PauseCircle className="h-5 w-5 text-orange-500" />,
				color: "border-orange-500/30 bg-orange-500/5",
				label: i18next.t('onHold', 'On Hold'),
				description:
					i18next.t('yourRequestIsOnHoldAnAuditorMayNeedMoreInformationOrTimeToReviewCheckBackSoon', 'Your request is on hold. An auditor may need more information or time to review. Check back soon.'),
			};
		default:
			return {
				variant: "secondary",
				icon: <Clock3 className="h-5 w-5 text-blue-500" />,
				color: "border-blue-500/30 bg-blue-500/5",
				label: i18next.t('pendingReview', 'Pending Review'),
				description:
					i18next.t('yourRequestIsInTheReviewQueueThisTypicallyTakes13BusinessDays', 'Your request is in the review queue. This typically takes 1–3 business days.'),
			};
	}
}

function actorLabel(log: AppPublicationLogItem) {
	return userDisplayName(log.author, "System");
}

function StepIndicator({
	step,
	currentStep,
}: { step: number; currentStep: number }) {
	const completed = step < currentStep;
	const active = step === currentStep;

	return (
		<div className="flex flex-col items-center">
			<div
				className={`w-8 h-8 rounded-full flex items-center justify-center text-sm font-medium transition-colors ${
					completed
						? "bg-primary text-primary-foreground"
						: active
							? "bg-primary/20 text-primary border-2 border-primary"
							: "bg-muted text-muted-foreground"
				}`}
			>
				{completed ? <CheckCircle2 className="h-4 w-4" /> : step}
			</div>
		</div>
	);
}

function getReviewStep(status: string): number {
	switch (status) {
		case "accepted":
			return 4;
		case "rejected":
			return 3;
		case "on_hold":
			return 2;
		default:
			return 2;
	}
}

interface AppPublicationPageProps {
	requests: AppPublicationRequestItem[];
	isLoading?: boolean;
	error?: string | null;
	onBack?: () => void;
	docsUrl?: string;
	/** When provided and the platform has the AI Act feature on, the EU AI
	 * Act conformity wizard is shown so the owner can complete the assessment
	 * required before publishing. */
	appId?: string | null;
}

export function AppPublicationPage({
	requests,
	isLoading,
	error,
	onBack,
	docsUrl = "https://docs.flow-like.com/guides/Apps/visibility/",
	appId,
}: Readonly<AppPublicationPageProps>) {
	const { t } = useTranslation("settings");
	const features = useFeatures();
	const showWizard = !!appId && features.data?.ai_act === true;

	const header = (
		<div className="flex items-center gap-3">
			{onBack && (
				<Button variant="ghost" size="sm" onClick={onBack}>
					<ArrowLeft className="h-4 w-4 mr-1" />
					{t('back', 'Back')}
				</Button>
			)}
			<h2 className="text-lg font-semibold">{t('publicationReview', 'Publication Review')}</h2>
		</div>
	);

	const wizard = showWizard && appId ? <AppAiActWizard appId={appId} /> : null;

	const activeRequests = requests.filter(
		(r) => r.status === "pending" || r.status === "on_hold",
	);
	const pastRequests = requests.filter(
		(r) => r.status === "accepted" || r.status === "rejected",
	);

	if (isLoading) {
		return (
			<div className="w-full max-w-4xl mx-auto p-2 md:p-6 pt-0 space-y-6">
				<Skeleton className="h-8 w-48" />
				<Skeleton className="h-48 w-full" />
				<Skeleton className="h-32 w-full" />
			</div>
		);
	}

	if (error) {
		return (
			<div className="w-full max-w-4xl mx-auto p-2 md:p-6 pt-0 space-y-6">
				{header}
				{wizard}
				<Card className="border-destructive/30 bg-destructive/5">
					<CardContent className="pt-6">
						<p className="text-sm text-destructive">{error}</p>
					</CardContent>
				</Card>
			</div>
		);
	}

	if (requests.length === 0) {
		return (
			<div className="w-full max-w-4xl mx-auto p-2 md:p-6 pt-0 space-y-6">
				{header}
				{wizard}
				<Card>
					<CardContent className="pt-6">
						<div className="text-center py-8 space-y-3">
							<Send className="h-10 w-10 mx-auto text-muted-foreground/50" />
							<p className="text-sm text-muted-foreground">
								{t('noPublicationRequestsYetChangeYourAppapossVisibilityToPublicToSubmitItForReview', "No publication requests yet. Change your app's visibility to Public to submit it for review.")}
							</p>
							<a href={docsUrl} target="_blank" rel="noreferrer">
								<Button variant="link" size="sm" className="gap-1">
									{t('learnAboutThePublicationProcess', 'Learn about the publication process')}
									<ExternalLink className="h-3 w-3" />
								</Button>
							</a>
						</div>
					</CardContent>
				</Card>
			</div>
		);
	}

	return (
		<div className="w-full max-w-4xl mx-auto p-2 md:p-6 pt-0 space-y-6 flex flex-col flex-grow max-h-full min-h-0 overflow-auto md:overflow-visible">
			{header}
			{wizard}
			{/* Active Requests */}
			{activeRequests.map((request) => {
				const config = statusConfig(request.status);
				const step = getReviewStep(request.status);

				return (
					<div key={request.id} className="space-y-4">
						{/* Status Card */}
						<Card className={config.color}>
							<CardHeader>
								<div className="flex items-start justify-between">
									<div className="flex items-center gap-3">
										{config.icon}
										<div>
											<CardTitle className="text-base">
												{config.label}
											</CardTitle>
											<CardDescription className="mt-1">
												{config.description}
											</CardDescription>
										</div>
									</div>
									<Badge variant="outline">
										{t('target', 'Target:')} {formatLabel(request.targetVisibility)}
									</Badge>
								</div>
							</CardHeader>
							<CardContent>
								{/* Progress Stepper */}
								<div className="flex items-center gap-0 w-full">
									<StepIndicator step={1} currentStep={step} />
									<div
										className={`flex-1 h-0.5 ${step > 1 ? "bg-primary" : "bg-muted"}`}
									/>
									<StepIndicator step={2} currentStep={step} />
									<div
										className={`flex-1 h-0.5 ${step > 2 ? "bg-primary" : "bg-muted"}`}
									/>
									<StepIndicator step={3} currentStep={step} />
									<div
										className={`flex-1 h-0.5 ${step > 3 ? "bg-primary" : "bg-muted"}`}
									/>
									<StepIndicator step={4} currentStep={step} />
								</div>
								<div className="flex justify-between mt-2">
									<span className="text-xs text-muted-foreground">
										{t('submitted', 'Submitted')}
									</span>
									<span className="text-xs text-muted-foreground">
										{t('inReview', 'In Review')}
									</span>
									<span className="text-xs text-muted-foreground">
										{t('decision', 'Decision')}
									</span>
									<span className="text-xs text-muted-foreground">
										{t('published', 'Published')}
									</span>
								</div>

								<Separator className="my-4" />

								<div className="grid grid-cols-2 gap-4 text-sm">
									<div>
										<span className="text-muted-foreground">{t('submitted', 'Submitted')}</span>
										<div className="font-medium">
											<RelativeTime
												value={request.createdAt}
												fallback={request.createdAt || "Unknown"}
											/>
										</div>
									</div>
									<div>
										<span className="text-muted-foreground">{t('lastUpdated', 'Last Updated')}</span>
										<div className="font-medium">
											<RelativeTime
												value={request.updatedAt}
												fallback={request.updatedAt || "Unknown"}
											/>
										</div>
									</div>
								</div>
							</CardContent>
						</Card>

						{/* Review Timeline */}
						<Card>
							<CardHeader>
								<CardTitle className="text-base flex items-center gap-2">
									<MessageSquare className="h-4 w-4" />
									{t('reviewActivity', 'Review Activity')}
								</CardTitle>
								<CardDescription>
									{`Communication and status updates from auditors`}
								</CardDescription>
							</CardHeader>
							<CardContent>
								{request.logs.length === 0 ? (
									<div className="text-center py-6">
										<Clock3 className="h-8 w-8 mx-auto text-muted-foreground/40 mb-2" />
										<p className="text-sm text-muted-foreground">
											{t('noReviewActivityYetYourRequestIsInTheQueue', 'No review activity yet. Your request is in the queue.')}
										</p>
									</div>
								) : (
									<div className="relative">
										{/* Timeline line */}
										<div className="absolute left-4 top-0 bottom-0 w-px bg-border" />
										<div className="space-y-4">
											{request.logs.map((log) => {
												const label = actorLabel(log);

												return (
													<div
														key={log.id}
														className="relative flex items-start gap-4 pl-0"
													>
														<div className="relative z-10 bg-background">
															<Avatar className="h-8 w-8 border">
																<AvatarImage
																	src={userAvatarUrl(log.author)}
																	alt={label}
																/>
																<AvatarFallback className="text-xs">
																	{userInitials(label)}
																</AvatarFallback>
															</Avatar>
														</div>
														<div className="min-w-0 flex-1 pb-4">
															<div className="flex flex-wrap items-center gap-2 text-sm">
																<span className="font-medium">{label}</span>
																<span className="text-muted-foreground">
																	<RelativeTime
																		value={log.createdAt}
																		fallback={log.createdAt || "Unknown"}
																	/>
																</span>
																{log.visibility && (
																	<Badge variant="outline" className="text-xs">
																		{formatLabel(log.visibility)}
																	</Badge>
																)}
															</div>
															{log.message ? (
																<p className="mt-1 text-sm text-muted-foreground bg-muted/50 rounded-lg p-3">
																	{log.message}
																</p>
															) : (
																<p className="mt-1 text-sm text-muted-foreground italic">
																	{t('noCommentProvided', 'No comment provided.')}
																</p>
															)}
														</div>
													</div>
												);
											})}
										</div>
									</div>
								)}
							</CardContent>
						</Card>

						{/* What to Expect */}
						<Card>
							<CardHeader>
								<CardTitle className="text-base flex items-center gap-2">
									<FileText className="h-4 w-4" />
									{t('whatToExpect', 'What to Expect')}
								</CardTitle>
							</CardHeader>
							<CardContent>
								<ul className="space-y-2 text-sm text-muted-foreground">
									<li className="flex items-start gap-2">
										<CheckCircle2 className="h-4 w-4 mt-0.5 text-green-600 shrink-0" />
										{t('reviewsTypicallyTake13BusinessDays', 'Reviews typically take 1–3 business days.')}
									</li>
									<li className="flex items-start gap-2">
										<CheckCircle2 className="h-4 w-4 mt-0.5 text-green-600 shrink-0" />
										{t('anAuditorWillCheckYourAppMetadataDescriptionAndContent', "An auditor will check your app metadata, description, and content.")}
									</li>
									<li className="flex items-start gap-2">
										<CheckCircle2 className="h-4 w-4 mt-0.5 text-green-600 shrink-0" />
										{t('youaposllReceiveAnEmailNotificationWhenADecisionIsMade', "You'll receive an email notification when a decision is made.")}
									</li>
									<li className="flex items-start gap-2">
										<CheckCircle2 className="h-4 w-4 mt-0.5 text-green-600 shrink-0" />
										{t('ifRejectedYouCanAddressTheFeedbackAndResubmit', 'If rejected, you can address the feedback and resubmit.')}
									</li>
								</ul>
								<div className="mt-4">
									<a href={docsUrl} target="_blank" rel="noreferrer">
										<Button variant="outline" size="sm" className="gap-1">
											{t('readThePublicationGuidelines', 'Read the publication guidelines')}
											<ExternalLink className="h-3 w-3" />
										</Button>
									</a>
								</div>
							</CardContent>
						</Card>
					</div>
				);
			})}

			{/* Past Requests */}
			{pastRequests.length > 0 && (
				<Card>
					<CardHeader>
						<CardTitle className="text-base">{t('previousRequests', 'Previous Requests')}</CardTitle>
						<CardDescription>
							{t('historyOfPastPublicationReviewDecisions', 'History of past publication review decisions')}
						</CardDescription>
					</CardHeader>
					<CardContent className="space-y-3">
						{pastRequests.map((request) => {
							const config = statusConfig(request.status);

							return (
								<div
									key={request.id}
									className="rounded-lg border bg-background/80 p-4"
								>
									<div className="flex flex-wrap items-center gap-2">
										<Badge variant={config.variant}>
											<span className="flex items-center gap-1">
												{statusConfig(request.status).icon}
												{config.label}
											</span>
										</Badge>
										<Badge variant="outline">
											{t('target', 'Target:')} {formatLabel(request.targetVisibility)}
										</Badge>
										<span className="text-xs text-muted-foreground">
											<RelativeTime
												value={request.updatedAt}
												fallback={request.updatedAt || "Unknown"}
											/>
										</span>
									</div>

									{request.logs.length > 0 && (
										<div className="mt-3 space-y-2">
											{request.logs.map((log) => {
												const label = actorLabel(log);

												return (
													<div key={log.id} className="flex items-start gap-3">
														<Avatar className="h-6 w-6">
															<AvatarImage
																src={userAvatarUrl(log.author)}
																alt={label}
															/>
															<AvatarFallback className="text-xs">
																{userInitials(label)}
															</AvatarFallback>
														</Avatar>
														<div className="min-w-0 flex-1">
															<div className="flex flex-wrap items-center gap-1 text-xs">
																<span className="font-medium">{label}</span>
																<span className="text-muted-foreground">
																	<RelativeTime
																		value={log.createdAt}
																		fallback={log.createdAt || "Unknown"}
																	/>
																</span>
															</div>
															{log.message && (
																<p className="text-xs text-muted-foreground mt-0.5">
																	{log.message}
																</p>
															)}
														</div>
													</div>
												);
											})}
										</div>
									)}
								</div>
							);
						})}
					</CardContent>
				</Card>
			)}
		</div>
	);
}
