"use client";
import {
	AppRefEditor,
	type AppRefFormValue,
	Button,
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
	ChallengeEditor,
	type ChallengeFormValue,
	Input,
	Label,
	Switch,
	Tabs,
	TabsContent,
	TabsList,
	TabsTrigger,
	TextEditor,
	useBackend,
	useInvoke,
} from "@flow-like/flow-like-ui";
import { buildAssetPlateNode } from "@flow-like/flow-like-ui/lib/learn/asset-elements";
import type {
	Challenge,
	LessonAppRef,
	LessonAssetView,
} from "@flow-like/flow-like-ui/lib/learn/types";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { ArrowLeft, Plus, Save, Trash2 } from "lucide-react";
import Link from "next/link";
import { useRouter, useSearchParams } from "next/navigation";
import { Suspense, useEffect, useMemo, useState } from "react";
import { useAuth } from "react-oidc-context";
import { toast } from "sonner";
import { learnApi } from "../../../../lib/learn-api";

export default function LessonAdminPage() {
	return (
		<Suspense fallback={null}>
			<LessonAdminContent />
		</Suspense>
	);
}

function LessonAdminContent() {
	const auth = useAuth();
	const router = useRouter();
	const searchParams = useSearchParams();
	const courseId = searchParams.get("courseId") ?? "";
	const moduleId = searchParams.get("moduleId") ?? "";
	const lessonId = searchParams.get("lessonId") ?? "";
	const queryClient = useQueryClient();
	const backend = useBackend();
	const profileQuery = useInvoke(
		backend.userState.getSettingsProfile,
		backend.userState,
		[],
	);
	const profile = profileQuery.data?.hub_profile ?? null;
	const profileId = profile?.id ?? "no-profile";
	const getProfile = () => {
		if (!profile) {
			throw new Error("Profile is required for lesson authoring.");
		}
		return profile;
	};

	const lessonQuery = useQuery({
		queryKey: [
			"learn",
			"admin",
			"lesson",
			courseId,
			moduleId,
			lessonId,
			profileId,
		],
		enabled: Boolean(profile && courseId && moduleId && lessonId),
		queryFn: () =>
			learnApi.getLesson(getProfile(), auth, courseId, moduleId, lessonId),
	});

	const linksQuery = useQuery({
		queryKey: ["learn", "admin", "links", courseId, profileId],
		enabled: Boolean(profile && courseId),
		queryFn: () => learnApi.listAppLinks(getProfile(), auth, courseId),
	});

	const aliasOptions = useMemo(
		() =>
			(linksQuery.data ?? [])
				.map((l) => l.alias)
				.filter((a): a is string => Boolean(a)),
		[linksQuery.data],
	);

	const lesson = lessonQuery.data?.lesson ?? null;
	const challenges = lessonQuery.data?.challenges ?? [];
	const appRefs = lessonQuery.data?.app_refs ?? [];
	const assetMentionItems = useMemo(
		() =>
			(lessonQuery.data?.assets ?? []).map((asset: LessonAssetView) => ({
				key: asset.id,
				text: asset.name,
				onSelect: (editor: unknown) => {
					const e = editor as {
						tf: {
							insertNodes: (node: unknown) => void;
							move?: (opts: { unit: "offset" }) => void;
						};
					};
					e.tf.insertNodes(buildAssetPlateNode(asset));
					e.tf.move?.({ unit: "offset" });
				},
			})),
		[lessonQuery.data?.assets],
	);

	// Form state for the lesson body.
	const [title, setTitle] = useState("");
	const [language, setLanguage] = useState("en");
	const [content, setContent] = useState("");
	const [videoUrl, setVideoUrl] = useState("");
	const [estimatedMinutes, setEstimatedMinutes] = useState(5);
	const [position, setPosition] = useState(0);
	const [isOptional, setIsOptional] = useState(false);

	useEffect(() => {
		if (!lesson) return;
		setTitle(lesson.title);
		setLanguage(lesson.language);
		setContent(lesson.content);
		setVideoUrl(lesson.video_url ?? "");
		setEstimatedMinutes(lesson.estimated_minutes);
		setPosition(lesson.position);
		setIsOptional(lesson.is_optional);
	}, [lesson]);

	const saveLesson = useMutation({
		mutationFn: () =>
			learnApi.upsertLesson(getProfile(), auth, courseId, moduleId, lessonId, {
				title,
				language,
				content,
				video_url: videoUrl || null,
				estimated_minutes: estimatedMinutes,
				position,
				is_optional: isOptional,
			}),
		onSuccess: () => {
			queryClient.invalidateQueries({
				queryKey: ["learn", "admin", "lesson", courseId, moduleId, lessonId],
			});
			queryClient.invalidateQueries({
				queryKey: ["learn", "admin", "structure", courseId],
			});
			toast.success("Lesson saved");
		},
		onError: (err) => {
			console.error(err);
			toast.error("Could not save lesson");
		},
	});

	const deleteLesson = useMutation({
		mutationFn: () =>
			learnApi.deleteLesson(getProfile(), auth, courseId, moduleId, lessonId),
		onSuccess: () => {
			queryClient.invalidateQueries({
				queryKey: ["learn", "admin", "structure", courseId],
			});
			toast.success("Lesson deleted");
			router.push(
				`/learn/admin/course?courseId=${encodeURIComponent(courseId)}`,
			);
		},
	});

	if (!courseId || !moduleId || !lessonId) {
		return (
			<div className="flex-1 overflow-auto">
				<div className="mx-auto max-w-3xl p-6 md:p-10">
					<Card>
						<CardHeader>
							<CardTitle>Lesson missing</CardTitle>
							<CardDescription>
								Open a lesson from the course editor.
							</CardDescription>
						</CardHeader>
						<CardContent>
							<Button asChild variant="outline">
								<Link href="/learn/admin">
									<ArrowLeft className="mr-2 h-4 w-4" />
									Course admin
								</Link>
							</Button>
						</CardContent>
					</Card>
				</div>
			</div>
		);
	}

	return (
		<div className="flex-1 overflow-auto">
			<div className="mx-auto max-w-5xl p-6 md:p-8 lg:p-10 space-y-6">
				<div className="flex items-center gap-3">
					<Link
						href={`/learn/admin/course?courseId=${encodeURIComponent(courseId)}`}
						className="inline-flex items-center gap-1 text-sm text-muted-foreground hover:text-foreground"
					>
						<ArrowLeft className="h-3 w-3" />
						Course editor
					</Link>
					<h1 className="ml-auto text-2xl font-semibold truncate max-w-[60%]">
						{title || "Untitled lesson"}
					</h1>
					<Button
						variant="ghost"
						size="sm"
						onClick={() => {
							if (confirm("Delete this lesson?")) deleteLesson.mutate();
						}}
					>
						<Trash2 className="h-3.5 w-3.5 mr-1.5" />
						Delete
					</Button>
				</div>

				<Tabs defaultValue="content">
					<TabsList>
						<TabsTrigger value="content">Content</TabsTrigger>
						<TabsTrigger value="challenges">
							Challenges ({challenges.length})
						</TabsTrigger>
						<TabsTrigger value="refs">App refs ({appRefs.length})</TabsTrigger>
					</TabsList>

					<TabsContent value="content" className="mt-4">
						<Card>
							<CardHeader>
								<CardTitle className="text-base">Lesson body</CardTitle>
								<CardDescription>
									Markdown is rendered with GFM (tables, task lists, code).
								</CardDescription>
							</CardHeader>
							<CardContent>
								<form
									className="space-y-4"
									onSubmit={(e) => {
										e.preventDefault();
										saveLesson.mutate();
									}}
								>
									<div className="grid grid-cols-1 md:grid-cols-2 gap-3">
										<div className="space-y-1.5">
											<Label htmlFor="lesson-title">Title</Label>
											<Input
												id="lesson-title"
												value={title}
												onChange={(e) => setTitle(e.target.value)}
												required
											/>
										</div>
										<div className="space-y-1.5">
											<Label htmlFor="lesson-lang">Language</Label>
											<Input
												id="lesson-lang"
												value={language}
												onChange={(e) => setLanguage(e.target.value)}
											/>
										</div>
										<div className="space-y-1.5">
											<Label htmlFor="lesson-position">Position</Label>
											<Input
												id="lesson-position"
												type="number"
												value={position}
												onChange={(e) =>
													setPosition(Number(e.target.value) || 0)
												}
											/>
										</div>
										<div className="space-y-1.5">
											<Label htmlFor="lesson-minutes">Estimated minutes</Label>
											<Input
												id="lesson-minutes"
												type="number"
												value={estimatedMinutes}
												onChange={(e) =>
													setEstimatedMinutes(Number(e.target.value) || 0)
												}
											/>
										</div>
										<div className="space-y-1.5 md:col-span-2">
											<Label htmlFor="lesson-video">
												Video URL (YouTube, Vimeo, MP4 — optional)
											</Label>
											<Input
												id="lesson-video"
												value={videoUrl}
												onChange={(e) => setVideoUrl(e.target.value)}
												placeholder="https://www.youtube.com/watch?v=…"
											/>
										</div>
									</div>
									<div className="flex items-center justify-between rounded-md border p-3">
										<div>
											<p className="text-sm font-medium">Optional lesson</p>
											<p className="text-xs text-muted-foreground">
												Optional lessons don't need to be completed for
												certificates.
											</p>
										</div>
										<Switch
											checked={isOptional}
											onCheckedChange={setIsOptional}
										/>
									</div>
									<div className="space-y-2">
										<Label>Content</Label>
										<p className="text-xs text-muted-foreground">
											Rich text editor — formatting, code, math, mentions and
											focus-node links are supported.
										</p>
										<div className="rounded-md border bg-background">
											<TextEditor
												initialContent={content}
												onChange={setContent}
												editable
												isMarkdown
												mentionItems={assetMentionItems}
											/>
										</div>
									</div>
									<Button type="submit" disabled={saveLesson.isPending}>
										<Save className="h-4 w-4 mr-2" />
										Save lesson
									</Button>
								</form>
							</CardContent>
						</Card>
					</TabsContent>

					<TabsContent value="challenges" className="mt-4 space-y-3">
						<ChallengesPanel
							courseId={courseId}
							lessonId={lessonId}
							challenges={challenges}
							onChanged={() =>
								queryClient.invalidateQueries({
									queryKey: [
										"learn",
										"admin",
										"lesson",
										courseId,
										moduleId,
										lessonId,
									],
								})
							}
						/>
					</TabsContent>

					<TabsContent value="refs" className="mt-4 space-y-3">
						<AppRefsPanel
							courseId={courseId}
							lessonId={lessonId}
							refs={appRefs}
							aliasOptions={aliasOptions}
							onChanged={() =>
								queryClient.invalidateQueries({
									queryKey: [
										"learn",
										"admin",
										"lesson",
										courseId,
										moduleId,
										lessonId,
									],
								})
							}
						/>
					</TabsContent>
				</Tabs>
			</div>
		</div>
	);
}

interface ChallengesPanelProps {
	readonly courseId: string;
	readonly lessonId: string;
	readonly challenges: ReadonlyArray<Challenge>;
	readonly onChanged: () => void;
}

function ChallengesPanel({
	courseId,
	lessonId,
	challenges,
	onChanged,
}: ChallengesPanelProps) {
	const auth = useAuth();
	const backend = useBackend();
	const profileQuery = useInvoke(
		backend.userState.getSettingsProfile,
		backend.userState,
		[],
	);
	const profile = profileQuery.data?.hub_profile ?? null;
	const getProfile = () => {
		if (!profile) {
			throw new Error("Profile is required for lesson authoring.");
		}
		return profile;
	};
	const [showNew, setShowNew] = useState(false);

	const upsert = useMutation({
		mutationFn: (args: { id: string | null; value: ChallengeFormValue }) => {
			const id =
				args.id ??
				`chal-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 5)}`;
			return learnApi.upsertChallenge(
				getProfile(),
				auth,
				courseId,
				lessonId,
				id,
				{
					kind: args.value.kind,
					prompt: args.value.prompt,
					explanation: args.value.explanation,
					payload: args.value.payload,
					points: args.value.points,
					position: args.value.position,
				},
			);
		},
		onSuccess: () => {
			setShowNew(false);
			onChanged();
		},
	});

	const remove = useMutation({
		mutationFn: (challengeId: string) =>
			learnApi.deleteChallenge(
				getProfile(),
				auth,
				courseId,
				lessonId,
				challengeId,
			),
		onSuccess: () => onChanged(),
	});

	return (
		<div className="space-y-3">
			{challenges.map((c) => (
				<ChallengeEditor
					key={c.id}
					initial={c}
					onSubmit={async (value) => {
						await upsert.mutateAsync({ id: c.id, value });
					}}
					onDelete={() => {
						if (confirm("Delete this challenge?")) remove.mutate(c.id);
					}}
					submitting={upsert.isPending}
				/>
			))}
			{showNew ? (
				<ChallengeEditor
					onSubmit={async (value) => {
						await upsert.mutateAsync({ id: null, value });
					}}
					onDelete={() => setShowNew(false)}
					submitting={upsert.isPending}
				/>
			) : (
				<Button variant="outline" onClick={() => setShowNew(true)}>
					<Plus className="h-4 w-4 mr-2" />
					Add challenge
				</Button>
			)}
		</div>
	);
}

interface AppRefsPanelProps {
	readonly courseId: string;
	readonly lessonId: string;
	readonly refs: ReadonlyArray<LessonAppRef>;
	readonly aliasOptions: ReadonlyArray<string>;
	readonly onChanged: () => void;
}

function AppRefsPanel({
	courseId,
	lessonId,
	refs,
	aliasOptions,
	onChanged,
}: AppRefsPanelProps) {
	const auth = useAuth();
	const backend = useBackend();
	const profileQuery = useInvoke(
		backend.userState.getSettingsProfile,
		backend.userState,
		[],
	);
	const profile = profileQuery.data?.hub_profile ?? null;
	const getProfile = () => {
		if (!profile) {
			throw new Error("Profile is required for lesson authoring.");
		}
		return profile;
	};
	const [showNew, setShowNew] = useState(false);

	const upsert = useMutation({
		mutationFn: (args: { id: string | null; value: AppRefFormValue }) => {
			const id =
				args.id ??
				`ref-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 5)}`;
			return learnApi.upsertAppRef(getProfile(), auth, courseId, lessonId, id, {
				kind: args.value.kind,
				app_alias: args.value.app_alias,
				app_id: args.value.app_id,
				label: args.value.label,
				target: args.value.target,
			});
		},
		onSuccess: () => {
			setShowNew(false);
			onChanged();
		},
	});

	const remove = useMutation({
		mutationFn: (refId: string) =>
			learnApi.deleteAppRef(getProfile(), auth, courseId, lessonId, refId),
		onSuccess: () => onChanged(),
	});

	return (
		<div className="space-y-3">
			{refs.map((r) => (
				<AppRefEditor
					key={r.id}
					initial={r}
					aliasOptions={aliasOptions}
					onSubmit={async (value) => {
						await upsert.mutateAsync({ id: r.id, value });
					}}
					onDelete={() => {
						if (confirm("Delete this app reference?")) remove.mutate(r.id);
					}}
					submitting={upsert.isPending}
				/>
			))}
			{showNew ? (
				<AppRefEditor
					aliasOptions={aliasOptions}
					onSubmit={async (value) => {
						await upsert.mutateAsync({ id: null, value });
					}}
					onDelete={() => setShowNew(false)}
					submitting={upsert.isPending}
				/>
			) : (
				<Button variant="outline" onClick={() => setShowNew(true)}>
					<Plus className="h-4 w-4 mr-2" />
					Add app reference
				</Button>
			)}
		</div>
	);
}
