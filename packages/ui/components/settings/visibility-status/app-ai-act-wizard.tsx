"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
	AlertTriangleIcon,
	CheckCircle2Icon,
	HelpCircleIcon,
	Loader2Icon,
	SaveIcon,
	SendIcon,
	ShieldCheckIcon,
	SparklesIcon,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { toast } from "sonner";
import { useInvoke } from "../../../hooks/use-invoke";
import { useBackend } from "../../../state/backend-state";
import {
	ConformityRecommendations,
	type Recommendation,
} from "../../ai-act/conformity-recommendations";
import {
	Alert,
	AlertDescription,
	AlertTitle,
	Badge,
	Button,
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
	Checkbox,
	Label,
	RadioGroup,
	RadioGroupItem,
	Separator,
	Skeleton,
	Textarea,
} from "../../ui";

// ---------------------------------------------------------------------------
// Types mirroring the backend EU AI Act schema (camelCase JSON).
// ---------------------------------------------------------------------------

type QuestionKind = "select" | "multi" | "yesno" | "text";

interface QuestionOption {
	value: string;
	label: string;
	help?: string;
}

interface Question {
	key: string;
	label: string;
	kind: QuestionKind;
	help?: string;
	options?: QuestionOption[];
	required?: boolean;
}

interface Screen {
	id: string;
	title: string;
	description: string;
	questions: Question[];
	highRiskOnly?: boolean;
}

interface QuestionnaireSchema {
	version: number;
	screens: Screen[];
}

interface Classification {
	riskCategory: "PROHIBITED" | "HIGH" | "LIMITED" | "MINIMAL" | "UNDETERMINED";
	conformityScore: number | null;
	conformityBand: "green" | "amber" | "red" | null;
	transparencyObligations: string[];
	blocked: boolean;
	rationale: string[];
}

interface QuestionnaireResponse {
	schema: QuestionnaireSchema;
	signals: Record<string, unknown>;
	answers: Record<string, unknown>;
	classification: Classification;
	recommendations: Recommendation[];
	responsibleName?: string | null;
	responsibleEmail?: string | null;
	hasAssessment: boolean;
}

interface SuggestedAnswer {
	key: string;
	value: unknown;
	rationale?: string;
	confidence?: number;
}

interface SuggestResponse {
	suggestion: {
		purpose?: string;
		suggestedAnswers?: SuggestedAnswer[];
		notes?: string[];
	};
	signals: Record<string, unknown>;
	model: string;
}

/** Per-question reasoning attached to an answer the assistant proposed. */
interface AnswerReasoning {
	rationale?: string;
	confidence?: number;
	model: string;
}

type Answers = Record<string, unknown>;

/** Questionnaire key for the one-sentence "what does your app do?" purpose. */
const PURPOSE_KEY = "purpose";

const OBLIGATION_LABELS: Record<string, string> = {
	disclose_ai_interaction: "Disclose users are interacting with AI (Art. 50.1)",
	label_generated_content: "Label AI-generated content (Art. 50.2)",
	disclose_emotion_biometric:
		"Inform people of emotion/biometric processing (Art. 50.3)",
	human_oversight: "Ensure human oversight (Annex III)",
	technical_documentation: "Maintain technical documentation & logging",
};

const RISK_META: Record<
	Classification["riskCategory"],
	{ label: string; className: string; description: string }
> = {
	PROHIBITED: {
		label: "Prohibited",
		className: "bg-red-600 text-white",
		description:
			"This app declares a prohibited AI practice (Art. 5) and cannot be published.",
	},
	HIGH: {
		label: "High Risk",
		className: "bg-orange-500 text-white",
		description:
			"High-risk system (Annex III). Human oversight and technical documentation are required.",
	},
	LIMITED: {
		label: "Limited Risk",
		className: "bg-yellow-500 text-black",
		description: "Transparency obligations under Art. 50 apply.",
	},
	MINIMAL: {
		label: "Minimal Risk",
		className: "bg-emerald-500 text-white",
		description: "No high-risk or transparency triggers detected.",
	},
	UNDETERMINED: {
		label: "Undetermined",
		className: "bg-slate-500 text-white",
		description:
			"One or more pivotal questions are unanswered or marked 'not sure'.",
	},
};

function bandColor(band: Classification["conformityBand"]): string {
	switch (band) {
		case "green":
			return "text-emerald-600";
		case "amber":
			return "text-yellow-600";
		case "red":
			return "text-red-600";
		default:
			return "text-muted-foreground";
	}
}

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

export interface AppAiActWizardProps {
	appId: string;
	/** Optional callback fired after a successful submission. */
	onSubmitted?: () => void;
	className?: string;
}

export function AppAiActWizard({
	appId,
	onSubmitted,
	className,
}: Readonly<AppAiActWizardProps>) {
	const backend = useBackend();
	const queryClient = useQueryClient();
	const profile = useInvoke(
		backend.userState.getSettingsProfile,
		backend.userState,
		[],
	);

	const [answers, setAnswers] = useState<Answers>({});
	const [livePreview, setLivePreview] = useState<Classification | null>(null);
	const [liveRecommendations, setLiveRecommendations] = useState<
		Recommendation[] | null
	>(null);
	const [reasoning, setReasoning] = useState<Record<string, AnswerReasoning>>(
		{},
	);
	const initialised = useRef(false);

	const questionnaire = useQuery<QuestionnaireResponse>({
		queryKey: ["ai-act", "questionnaire", appId],
		queryFn: async () => {
			if (!profile.data) throw new Error("Profile not loaded");
			return backend.apiState.get<QuestionnaireResponse>(
				profile.data.hub_profile,
				`apps/${appId}/ai-act/questionnaire`,
			);
		},
		enabled: !!profile.data && !!appId,
	});

	// Seed local state once the questionnaire loads.
	useEffect(() => {
		if (questionnaire.data && !initialised.current) {
			initialised.current = true;
			setAnswers((questionnaire.data.answers as Answers) ?? {});
			setLivePreview(questionnaire.data.classification);
			setLiveRecommendations(questionnaire.data.recommendations ?? []);
		}
	}, [questionnaire.data]);

	// Debounced live classification preview (authoritative, server-side).
	useEffect(() => {
		if (!profile.data || !initialised.current) return;
		const hubProfile = profile.data.hub_profile;
		const handle = setTimeout(async () => {
			try {
				const res = await backend.apiState.post<{
					classification: Classification;
					recommendations: Recommendation[];
				}>(hubProfile, `apps/${appId}/ai-act/classify`, {
					answers,
				});
				setLivePreview(res.classification);
				setLiveRecommendations(res.recommendations ?? []);
			} catch {
				// Preview failures are non-fatal; the authoritative result is
				// recomputed on submit.
			}
		}, 400);
		return () => clearTimeout(handle);
	}, [answers, appId, backend.apiState, profile.data]);

	const suggestMutation = useMutation({
		mutationFn: async () => {
			if (!profile.data) throw new Error("Profile not loaded");
			return backend.apiState.post<SuggestResponse>(
				profile.data.hub_profile,
				`apps/${appId}/ai-act/assessment/suggest`,
				{ profile: profile.data.hub_profile },
			);
		},
		onSuccess: (res) => {
			const suggested = res.suggestion?.suggestedAnswers ?? [];
			const model = res.model;
			const purpose = res.suggestion?.purpose?.trim();
			const hasPurposeAnswer = suggested.some((s) => s.key === PURPOSE_KEY);

			if (suggested.length === 0 && !purpose) {
				toast.info("The assistant had no answer suggestions for this app.");
				return;
			}

			setAnswers((prev) => {
				const next = { ...prev };
				for (const s of suggested) {
					next[s.key] = s.value;
				}
				// The agent's one-sentence purpose is returned as a top-level field;
				// fall back to it for the purpose question when not already covered.
				if (purpose && !hasPurposeAnswer) {
					next[PURPOSE_KEY] = purpose;
				}
				return next;
			});

			setReasoning(() => {
				const next: Record<string, AnswerReasoning> = {};
				for (const s of suggested) {
					next[s.key] = {
						rationale: s.rationale,
						confidence: s.confidence,
						model,
					};
				}
				if (purpose && !hasPurposeAnswer) {
					next[PURPOSE_KEY] = {
						rationale: "Derived as the one-sentence summary of the app.",
						model,
					};
				}
				return next;
			});

			const appliedCount =
				suggested.length + (purpose && !hasPurposeAnswer ? 1 : 0);
			toast.success(
				`Applied ${appliedCount} suggestion${appliedCount === 1 ? "" : "s"} from ${model}. Please review before submitting.`,
			);
		},
		onError: (err: Error) => {
			toast.error(err.message ?? "Failed to get suggestions");
		},
	});

	const saveMutation = useMutation({
		mutationFn: async (submit: boolean) => {
			if (!profile.data) throw new Error("Profile not loaded");
			return backend.apiState.put(
				profile.data.hub_profile,
				`apps/${appId}/ai-act/assessment`,
				{
					answers,
					submit,
				},
			);
		},
		onSuccess: async (_data, submit) => {
			await queryClient.invalidateQueries({
				queryKey: ["ai-act", "questionnaire", appId],
			});
			toast.success(
				submit
					? "Assessment submitted. It will be reviewed during publication."
					: "Draft saved.",
			);
			if (submit) onSubmitted?.();
		},
		onError: (err: Error) => {
			toast.error(err.message ?? "Failed to save assessment");
		},
	});

	const setAnswer = useCallback((key: string, value: unknown) => {
		setAnswers((prev) => ({ ...prev, [key]: value }));
		setReasoning((prev) => {
			if (!prev[key]) return prev;
			const next = { ...prev };
			delete next[key];
			return next;
		});
	}, []);

	const preview = livePreview ?? questionnaire.data?.classification ?? null;
	const recommendations =
		liveRecommendations ?? questionnaire.data?.recommendations ?? [];
	const responsibleName = questionnaire.data?.responsibleName ?? null;
	const responsibleEmail = questionnaire.data?.responsibleEmail ?? null;
	const showHighRisk =
		preview?.riskCategory === "HIGH" ||
		preview?.riskCategory === "UNDETERMINED";

	const visibleScreens = useMemo(() => {
		const screens = questionnaire.data?.schema?.screens ?? [];
		return screens.filter((s) => !s.highRiskOnly || showHighRisk);
	}, [questionnaire.data?.schema?.screens, showHighRisk]);

	// Validate required questions on visible screens.
	const missingRequired = useMemo(() => {
		const missing: string[] = [];
		for (const screen of visibleScreens) {
			for (const q of screen.questions) {
				if (!q.required) continue;
				const v = answers[q.key];
				const empty =
					v === undefined ||
					v === null ||
					v === "" ||
					(Array.isArray(v) && v.length === 0);
				if (empty) missing.push(q.label);
			}
		}
		return missing;
	}, [visibleScreens, answers]);

	if (questionnaire.isLoading) {
		return (
			<div className={className}>
				<Skeleton className="h-8 w-64 mb-4" />
				<Skeleton className="h-48 w-full mb-3" />
				<Skeleton className="h-48 w-full" />
			</div>
		);
	}

	if (questionnaire.isError) {
		return (
			<Alert variant="destructive" className={className}>
				<AlertTriangleIcon className="h-4 w-4" />
				<AlertTitle>Could not load the AI Act questionnaire</AlertTitle>
				<AlertDescription>
					{questionnaire.error?.message ?? "Unknown error"}
				</AlertDescription>
			</Alert>
		);
	}

	const riskMeta = preview ? RISK_META[preview.riskCategory] : null;
	const canSubmit =
		!!preview && !preview.blocked && missingRequired.length === 0;

	return (
		<div className={className}>
			<div className="flex items-start justify-between gap-4 mb-4">
				<div className="flex items-center gap-2">
					<ShieldCheckIcon className="h-5 w-5 text-primary" />
					<div>
						<h3 className="text-base font-semibold">
							EU AI Act Conformity Check
						</h3>
						<p className="text-sm text-muted-foreground">
							Answer the questionnaire to classify this app before publishing.
						</p>
					</div>
				</div>
				<Button
					variant="outline"
					size="sm"
					onClick={() => suggestMutation.mutate()}
					disabled={suggestMutation.isPending}
				>
					{suggestMutation.isPending ? (
						<Loader2Icon className="h-4 w-4 animate-spin" />
					) : (
						<SparklesIcon className="h-4 w-4" />
					)}
					Help me answer
				</Button>
			</div>

			{/* Live classification banner */}
			{riskMeta && (
				<Card className="mb-4">
					<CardContent className="py-4">
						<div className="flex flex-wrap items-center gap-3">
							<Badge className={riskMeta.className}>{riskMeta.label}</Badge>
							{typeof preview?.conformityScore === "number" && (
								<span
									className={`text-sm font-medium ${bandColor(preview.conformityBand)}`}
								>
									Conformity score: {preview.conformityScore}/100
								</span>
							)}
						</div>
						<p className="text-sm text-muted-foreground mt-2">
							{riskMeta.description}
						</p>
						{preview && preview.transparencyObligations.length > 0 && (
							<div className="mt-3">
								<p className="text-xs font-medium mb-1">
									Transparency obligations
								</p>
								<ul className="space-y-1">
									{preview.transparencyObligations.map((o) => (
										<li
											key={o}
											className="flex items-center gap-2 text-xs text-muted-foreground"
										>
											<CheckCircle2Icon className="h-3 w-3 text-primary shrink-0" />
											{OBLIGATION_LABELS[o] ?? o}
										</li>
									))}
								</ul>
							</div>
						)}
						{preview?.blocked && (
							<Alert variant="destructive" className="mt-3">
								<AlertTriangleIcon className="h-4 w-4" />
								<AlertTitle>Publication blocked</AlertTitle>
								<AlertDescription>
									Remove the prohibited practice declaration to proceed, or keep
									this app private.
								</AlertDescription>
							</Alert>
						)}
					</CardContent>
				</Card>
			)}

			{(suggestMutation.data?.suggestion?.notes?.length ?? 0) > 0 && (
				<Alert className="mb-4">
					<SparklesIcon className="h-4 w-4" />
					<AlertTitle>
						Assistant notes
						{suggestMutation.data?.model
							? ` · ${suggestMutation.data.model}`
							: ""}
					</AlertTitle>
					<AlertDescription>
						<ul className="list-disc pl-4 space-y-1">
							{suggestMutation.data?.suggestion?.notes?.map((note) => (
								<li key={note}>{note}</li>
							))}
						</ul>
					</AlertDescription>
				</Alert>
			)}

			{/* Screens */}
			<div className="space-y-4">
				{visibleScreens.map((screen) => (
					<Card key={screen.id}>
						<CardHeader>
							<CardTitle className="text-sm">{screen.title}</CardTitle>
							{screen.description && (
								<CardDescription>{screen.description}</CardDescription>
							)}
						</CardHeader>
						<CardContent className="space-y-5">
							{screen.questions.map((q, idx) => (
								<div key={q.key}>
									{idx > 0 && <Separator className="mb-5" />}
									<QuestionField
										question={q}
										value={answers[q.key]}
										onChange={(v) => setAnswer(q.key, v)}
										reasoning={reasoning[q.key]}
									/>
								</div>
							))}
						</CardContent>
					</Card>
				))}
			</div>

			{/* Responsible person — hard-linked to the app owner */}
			<Card className="mt-4">
				<CardContent className="flex items-center gap-3 py-4">
					<ShieldCheckIcon className="h-4 w-4 shrink-0 text-primary" />
					<div className="min-w-0">
						<p className="text-sm font-medium">
							Responsible person:{" "}
							{responsibleName ?? responsibleEmail ?? "App owner"}
						</p>
						<p className="text-xs text-muted-foreground">
							{responsibleEmail ? `${responsibleEmail} · ` : ""}
							Automatically set to the app owner (EU AI Act Art. 26) and shown
							to reviewers. This cannot be changed.
						</p>
					</div>
				</CardContent>
			</Card>

			{/* How to improve the conformity score */}
			{!preview?.blocked && (
				<ConformityRecommendations
					className="mt-4"
					recommendations={recommendations}
					score={preview?.conformityScore ?? null}
				/>
			)}

			{/* Footer actions */}
			<div className="flex flex-wrap items-center justify-between gap-3 mt-5">
				<div className="text-xs text-muted-foreground">
					{missingRequired.length > 0 ? (
						<span className="flex items-center gap-1 text-amber-600">
							<AlertTriangleIcon className="h-3 w-3" />
							{missingRequired.length} required question
							{missingRequired.length === 1 ? "" : "s"} remaining
						</span>
					) : (
						<span className="flex items-center gap-1">
							<HelpCircleIcon className="h-3 w-3" />
							The risk category and score are always recomputed on the server.
						</span>
					)}
				</div>
				<div className="flex items-center gap-2">
					<Button
						variant="outline"
						size="sm"
						onClick={() => saveMutation.mutate(false)}
						disabled={saveMutation.isPending}
					>
						<SaveIcon className="h-4 w-4" />
						Save draft
					</Button>
					<Button
						size="sm"
						onClick={() => saveMutation.mutate(true)}
						disabled={!canSubmit || saveMutation.isPending}
					>
						{saveMutation.isPending ? (
							<Loader2Icon className="h-4 w-4 animate-spin" />
						) : (
							<SendIcon className="h-4 w-4" />
						)}
						Submit assessment
					</Button>
				</div>
			</div>
		</div>
	);
}

// ---------------------------------------------------------------------------
// Question field renderer
// ---------------------------------------------------------------------------

/** Inline reasoning chip shown under a question the assistant pre-filled. */
function AssistantReasoning({
	reasoning,
}: Readonly<{ reasoning: AnswerReasoning }>) {
	const confidencePct =
		typeof reasoning.confidence === "number"
			? Math.round(reasoning.confidence * 100)
			: null;
	const lowConfidence =
		typeof reasoning.confidence === "number" && reasoning.confidence < 0.5;

	return (
		<div className="mt-2 rounded-md border border-primary/20 bg-primary/5 px-2.5 py-1.5">
			<div className="flex flex-wrap items-center gap-2">
				<span className="flex items-center gap-1 text-xs font-medium text-primary">
					<SparklesIcon className="h-3 w-3" />
					Assistant
				</span>
				<Badge variant="secondary" className="text-[10px]">
					{reasoning.model}
				</Badge>
				{confidencePct !== null && (
					<Badge
						variant={lowConfidence ? "destructive" : "outline"}
						className="text-[10px]"
					>
						{confidencePct}% confidence
					</Badge>
				)}
			</div>
			{reasoning.rationale && (
				<p className="mt-1 text-xs text-muted-foreground">
					{reasoning.rationale}
				</p>
			)}
		</div>
	);
}

interface QuestionFieldProps {
	question: Question;
	value: unknown;
	onChange: (value: unknown) => void;
	reasoning?: AnswerReasoning;
}

function QuestionField({
	question,
	value,
	onChange,
	reasoning,
}: Readonly<QuestionFieldProps>) {
	const label = (
		<div className="mb-2">
			<Label className="text-sm font-medium">
				{question.label}
				{question.required && <span className="text-destructive"> *</span>}
			</Label>
			{question.help && (
				<p className="text-xs text-muted-foreground mt-0.5">{question.help}</p>
			)}
			{reasoning && <AssistantReasoning reasoning={reasoning} />}
		</div>
	);

	if (question.kind === "yesno") {
		const current = typeof value === "string" ? value : "";
		const options = question.options?.length
			? question.options
			: [
					{ value: "yes", label: "Yes" },
					{ value: "no", label: "No" },
				];
		return (
			<div>
				{label}
				<RadioGroup
					value={current}
					onValueChange={onChange}
					className="flex flex-col gap-2"
				>
					{options.map((opt) => (
						<div key={opt.value} className="flex items-center gap-2">
							<RadioGroupItem
								value={opt.value}
								id={`${question.key}-${opt.value}`}
							/>
							<Label
								htmlFor={`${question.key}-${opt.value}`}
								className="text-sm font-normal"
							>
								{opt.label}
							</Label>
						</div>
					))}
				</RadioGroup>
			</div>
		);
	}

	if (question.kind === "select") {
		const current = typeof value === "string" ? value : "";
		return (
			<div>
				{label}
				<RadioGroup
					value={current}
					onValueChange={onChange}
					className="flex flex-col gap-2"
				>
					{(question.options ?? []).map((opt) => (
						<div key={opt.value} className="flex items-start gap-2">
							<RadioGroupItem
								value={opt.value}
								id={`${question.key}-${opt.value}`}
								className="mt-0.5"
							/>
							<Label
								htmlFor={`${question.key}-${opt.value}`}
								className="text-sm font-normal"
							>
								{opt.label}
								{opt.help && (
									<span className="block text-xs text-muted-foreground">
										{opt.help}
									</span>
								)}
							</Label>
						</div>
					))}
				</RadioGroup>
			</div>
		);
	}

	if (question.kind === "multi") {
		const current = Array.isArray(value) ? (value as string[]) : [];
		const toggle = (optValue: string, checked: boolean) => {
			const set = new Set(current);
			if (checked) set.add(optValue);
			else set.delete(optValue);
			onChange(Array.from(set));
		};
		return (
			<div>
				{label}
				<div className="flex flex-col gap-2">
					{(question.options ?? []).map((opt) => (
						<div key={opt.value} className="flex items-start gap-2">
							<Checkbox
								id={`${question.key}-${opt.value}`}
								checked={current.includes(opt.value)}
								onCheckedChange={(c) => toggle(opt.value, c === true)}
								className="mt-0.5"
							/>
							<Label
								htmlFor={`${question.key}-${opt.value}`}
								className="text-sm font-normal"
							>
								{opt.label}
								{opt.help && (
									<span className="block text-xs text-muted-foreground">
										{opt.help}
									</span>
								)}
							</Label>
						</div>
					))}
				</div>
			</div>
		);
	}

	// text
	const current = typeof value === "string" ? value : "";
	return (
		<div>
			{label}
			<Textarea
				value={current}
				onChange={(e) => onChange(e.target.value)}
				rows={3}
			/>
		</div>
	);
}
