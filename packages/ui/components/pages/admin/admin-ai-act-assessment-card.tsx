"use client";

import { useMutation, useQuery } from "@tanstack/react-query";
import { Bot, ShieldAlert, Sparkles } from "lucide-react";
import { useMemo, useState } from "react";
import { toast } from "sonner";
import { useFeatures } from "../../../hooks/use-features";
import { useInvoke } from "../../../hooks/use-invoke";
import { useBackend } from "../../../state/backend-state";
import {
	Badge,
	Button,
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
	Separator,
	Skeleton,
} from "../../ui";

type RiskCategory =
	| "PROHIBITED"
	| "HIGH"
	| "LIMITED"
	| "MINIMAL"
	| "UNDETERMINED";

interface QuestionOption {
	value: string;
	label: string;
}

interface Question {
	key: string;
	label: string;
	kind: "select" | "multi" | "yesno" | "text" | "contact";
	options?: QuestionOption[];
}

interface Screen {
	id: string;
	title: string;
	description?: string;
	questions: Question[];
	highRiskOnly?: boolean;
}

interface QuestionnaireSchema {
	version: number;
	screens: Screen[];
}

interface Classification {
	riskCategory: RiskCategory;
	conformityScore: number | null;
	conformityBand: "green" | "amber" | "red" | null;
	transparencyObligations: string[];
	blocked: boolean;
	rationale: string[];
}

interface InventoryDetailResponse {
	appId: string;
	appName?: string | null;
	assessment?: { status?: string } | null;
	schema: QuestionnaireSchema;
	signals: Record<string, unknown>;
	answers: Record<string, unknown>;
	classification: Classification;
	hasAssessment: boolean;
}

interface SuggestedAnswer {
	key: string;
	value: unknown;
	rationale?: string;
	confidence?: number;
}

interface GovernanceSuggestion {
	purpose?: string;
	suggestedAnswers?: SuggestedAnswer[];
	notes?: string[];
}

interface AssistResponse {
	suggestion: GovernanceSuggestion;
	signals: Record<string, unknown>;
	model: string;
}

const RISK_META: Record<RiskCategory, { label: string; className: string }> = {
	PROHIBITED: { label: "Prohibited", className: "bg-red-600 text-white" },
	HIGH: { label: "High Risk", className: "bg-orange-500 text-white" },
	LIMITED: { label: "Limited Risk", className: "bg-yellow-500 text-black" },
	MINIMAL: { label: "Minimal Risk", className: "bg-emerald-500 text-white" },
	UNDETERMINED: {
		label: "Undetermined",
		className: "bg-muted text-foreground",
	},
};

function bandTextColor(band?: string | null): string {
	switch (band) {
		case "green":
			return "text-emerald-600";
		case "amber":
			return "text-amber-600";
		case "red":
			return "text-red-600";
		default:
			return "text-muted-foreground";
	}
}

function formatAnswer(question: Question | undefined, value: unknown): string {
	if (value === undefined || value === null || value === "") return "—";

	const labelFor = (raw: string) =>
		question?.options?.find((o) => o.value === raw)?.label ?? raw;

	if (Array.isArray(value)) {
		if (value.length === 0) return "—";
		return value.map((v) => labelFor(String(v))).join(", ");
	}

	if (question?.kind === "yesno") {
		const raw = String(value).toLowerCase();
		if (raw === "yes" || raw === "true") return "Yes";
		if (raw === "no" || raw === "false") return "No";
	}

	return labelFor(String(value));
}

function canonical(question: Question | undefined, value: unknown): string {
	if (value === undefined || value === null || value === "") return "";
	if (Array.isArray(value)) {
		return value
			.map((v) => formatAnswer(question, [v]).toLowerCase())
			.sort()
			.join("|");
	}
	return formatAnswer(question, value).toLowerCase();
}

export function AdminAiActAssessmentCard({
	appId,
}: Readonly<{ appId: string }>) {
	const backend = useBackend();
	const features = useFeatures();
	const profile = useInvoke(
		backend.userState.getProfile,
		backend.userState,
		[],
	);

	const [suggestion, setSuggestion] = useState<GovernanceSuggestion | null>(
		null,
	);
	const [model, setModel] = useState<string | null>(null);

	const enabled = !!profile.data && features.data?.ai_act === true && !!appId;

	const detail = useQuery<InventoryDetailResponse>({
		queryKey: ["admin", "ai-act", "inventory", appId],
		queryFn: () => {
			if (!profile.data) throw new Error("Profile not loaded");
			return backend.apiState.get<InventoryDetailResponse>(
				profile.data,
				`admin/ai-act/inventory/${encodeURIComponent(appId)}`,
			);
		},
		enabled,
	});

	const validate = useMutation({
		mutationFn: async () => {
			if (!profile.data) throw new Error("Profile not loaded");
			return backend.apiState.post<AssistResponse>(
				profile.data,
				`admin/ai-act/assist/${encodeURIComponent(appId)}`,
				{ profile: profile.data },
			);
		},
		onSuccess: (res) => {
			setSuggestion(res.suggestion ?? null);
			setModel(res.model ?? null);
			const count = res.suggestion?.suggestedAnswers?.length ?? 0;
			toast.success(
				count > 0
					? `FlowPilot reviewed ${count} answer${count === 1 ? "" : "s"}.`
					: "FlowPilot completed with no answer findings.",
			);
		},
		onError: (err: Error) =>
			toast.error(err.message ?? "FlowPilot validation failed"),
	});

	const data = detail.data;

	const questionByKey = useMemo(() => {
		const map = new Map<string, Question>();
		for (const screen of data?.schema.screens ?? []) {
			for (const question of screen.questions) {
				map.set(question.key, question);
			}
		}
		return map;
	}, [data?.schema]);

	if (!features.data?.ai_act) return null;

	if (detail.isLoading) {
		return (
			<Card>
				<CardHeader>
					<CardTitle className="text-base flex items-center gap-2">
						<ShieldAlert className="h-4 w-4" />
						EU AI Act Conformity
					</CardTitle>
				</CardHeader>
				<CardContent>
					<Skeleton className="h-32 w-full" />
				</CardContent>
			</Card>
		);
	}

	if (!data) return null;

	const { classification, schema, answers, hasAssessment } = data;
	const risk = RISK_META[classification.riskCategory] ?? RISK_META.UNDETERMINED;
	const statusLabel = hasAssessment
		? (data.assessment?.status ?? "DRAFT")
		: "NOT SUBMITTED";

	return (
		<Card>
			<CardHeader>
				<div className="flex items-start justify-between gap-3">
					<div className="space-y-1.5">
						<CardTitle className="text-base flex items-center gap-2">
							<ShieldAlert className="h-4 w-4" />
							EU AI Act Conformity
						</CardTitle>
						<CardDescription>
							{hasAssessment
								? "The owner's submitted assessment and the authoritative live classification."
								: "The owner has not started an assessment. The questionnaire below shows auto-derived answers and the current classification."}
						</CardDescription>
					</div>
					<Button
						variant="outline"
						size="sm"
						disabled={validate.isPending || !profile.data}
						onClick={() => validate.mutate()}
					>
						<Sparkles
							className={`mr-2 h-4 w-4 ${validate.isPending ? "animate-pulse" : ""}`}
						/>
						Validate with FlowPilot
					</Button>
				</div>
			</CardHeader>
			<CardContent className="space-y-4">
				<div className="flex flex-wrap items-center gap-3">
					<Badge className={risk.className}>{risk.label}</Badge>
					<Badge variant="secondary">{statusLabel}</Badge>
					{typeof classification.conformityScore === "number" && (
						<span
							className={`text-sm font-semibold ${bandTextColor(classification.conformityBand)}`}
						>
							{classification.conformityScore}/100
						</span>
					)}
					{classification.blocked && (
						<Badge variant="destructive">Blocked</Badge>
					)}
				</div>

				{classification.rationale.length > 0 && (
					<ul className="list-disc pl-5 text-sm text-muted-foreground space-y-1">
						{classification.rationale.map((line) => (
							<li key={line}>{line}</li>
						))}
					</ul>
				)}

				<Separator />

				<div className="space-y-4">
					{schema.screens
						.filter(
							(screen) =>
								!screen.highRiskOnly ||
								classification.riskCategory === "HIGH" ||
								classification.riskCategory === "PROHIBITED",
						)
						.map((screen) => (
							<div key={screen.id} className="space-y-2">
								<h4 className="text-sm font-semibold">{screen.title}</h4>
								<dl className="grid gap-x-4 gap-y-2 sm:grid-cols-2">
									{screen.questions.map((question) => (
										<div key={question.key} className="space-y-0.5">
											<dt className="text-xs text-muted-foreground">
												{question.label}
											</dt>
											<dd className="text-sm font-medium">
												{formatAnswer(question, answers[question.key])}
											</dd>
										</div>
									))}
								</dl>
							</div>
						))}
				</div>

				{suggestion && (
					<FlowPilotValidation
						suggestion={suggestion}
						questionByKey={questionByKey}
						answers={answers}
						model={model}
					/>
				)}
			</CardContent>
		</Card>
	);
}

function FlowPilotValidation({
	suggestion,
	questionByKey,
	answers,
	model,
}: Readonly<{
	suggestion: GovernanceSuggestion;
	questionByKey: Map<string, Question>;
	answers: Record<string, unknown>;
	model: string | null;
}>) {
	const suggestedAnswers = suggestion.suggestedAnswers ?? [];

	return (
		<div className="rounded-lg border border-dashed bg-muted/30 p-4 space-y-3">
			<div className="flex flex-wrap items-center gap-2">
				<Bot className="h-4 w-4 text-primary" />
				<h4 className="text-sm font-semibold">FlowPilot validation</h4>
				{model && <Badge variant="secondary">{model}</Badge>}
			</div>

			{suggestion.purpose && (
				<p className="text-sm text-muted-foreground">
					<span className="font-medium text-foreground">Derived purpose:</span>{" "}
					{suggestion.purpose}
				</p>
			)}

			{suggestedAnswers.length === 0 ? (
				<p className="text-sm text-muted-foreground">
					FlowPilot found nothing to flag against the submitted answers.
				</p>
			) : (
				<ul className="space-y-3">
					{suggestedAnswers.map((s) => {
						const question = questionByKey.get(s.key);
						const ownerValue = answers[s.key];
						const matches =
							canonical(question, ownerValue) === canonical(question, s.value);
						const confidencePct =
							typeof s.confidence === "number"
								? Math.round(s.confidence * 100)
								: null;
						const lowConfidence =
							typeof s.confidence === "number" && s.confidence < 0.5;

						return (
							<li
								key={s.key}
								className="rounded-md border bg-background p-3 space-y-1.5"
							>
								<div className="flex flex-wrap items-center justify-between gap-2">
									<span className="text-sm font-medium">
										{question?.label ?? s.key}
									</span>
									<div className="flex items-center gap-2">
										{confidencePct !== null && (
											<Badge
												variant={lowConfidence ? "destructive" : "secondary"}
											>
												{confidencePct}% confidence
											</Badge>
										)}
										<Badge
											className={
												matches
													? "bg-emerald-500 text-white"
													: "bg-amber-500 text-black"
											}
										>
											{matches ? "Matches" : "Differs"}
										</Badge>
									</div>
								</div>
								<div className="grid gap-x-4 gap-y-1 text-sm sm:grid-cols-2">
									<div>
										<span className="text-xs text-muted-foreground">
											Submitted
										</span>
										<div className="font-medium">
											{formatAnswer(question, ownerValue)}
										</div>
									</div>
									<div>
										<span className="text-xs text-muted-foreground">
											FlowPilot suggests
										</span>
										<div className="font-medium">
											{formatAnswer(question, s.value)}
										</div>
									</div>
								</div>
								{s.rationale && (
									<p className="text-xs text-muted-foreground">{s.rationale}</p>
								)}
							</li>
						);
					})}
				</ul>
			)}

			{(suggestion.notes?.length ?? 0) > 0 && (
				<div className="space-y-1">
					<span className="text-xs font-medium text-muted-foreground">
						Reviewer notes
					</span>
					<ul className="list-disc pl-5 text-xs text-muted-foreground space-y-1">
						{suggestion.notes?.map((note) => (
							<li key={note}>{note}</li>
						))}
					</ul>
				</div>
			)}
		</div>
	);
}
