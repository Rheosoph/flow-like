"use client";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
	Badge,
	Button,
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
	Progress,
	useBackend,
	useInvoke,
} from "@tm9657/flow-like-ui";
import type { ModuleWithLessons } from "@tm9657/flow-like-ui/lib/learn/types";
import {
	ArrowLeft,
	Award,
	BookOpen,
	CheckCircle2,
	ChevronRight,
	Circle,
	Clock,
	PlayCircle,
} from "lucide-react";
import Link from "next/link";
import { useRouter, useSearchParams } from "next/navigation";
import { Suspense, useMemo } from "react";
import { useAuth } from "react-oidc-context";
import { learnApi } from "../../../lib/learn-api";

export default function CourseDetailPage() {
	return (
		<Suspense fallback={null}>
			<CourseDetailContent />
		</Suspense>
	);
}

function CourseDetailContent() {
	const router = useRouter();
	const searchParams = useSearchParams();
	const courseId = searchParams.get("courseId") ?? "";
	const auth = useAuth();
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
			throw new Error("Profile is required for learning API calls.");
		}
		return profile;
	};

	const structureQuery = useQuery({
		queryKey: ["learn", "structure", courseId, profileId],
		enabled: Boolean(profile && courseId),
		queryFn: () => learnApi.getCourseStructure(getProfile(), auth, courseId),
	});

	const enrollMutation = useMutation({
		mutationFn: () => learnApi.enroll(getProfile(), auth, courseId),
		onSuccess: () => {
			queryClient.invalidateQueries({
				queryKey: ["learn", "enrollments", "me"],
			});
		},
	});

	const enrollmentsQuery = useQuery({
		queryKey: ["learn", "enrollments", "me", profileId, auth.user?.profile.sub],
		enabled: Boolean(profile && auth.user),
		queryFn: () => learnApi.myEnrollments(getProfile(), auth),
	});

	const isEnrolled = useMemo(
		() =>
			Boolean(courseId) &&
			(enrollmentsQuery.data ?? []).some((e) => e.course_id === courseId),
		[enrollmentsQuery.data, courseId],
	);

	const progressQuery = useQuery({
		queryKey: ["learn", "progress", "me", courseId, profileId],
		enabled: Boolean(profile && auth.user && courseId && isEnrolled),
		queryFn: () => learnApi.myCourseProgress(getProfile(), auth, courseId),
	});

	const completedLessonIds = useMemo(
		() =>
			new Set(
				(progressQuery.data ?? [])
					.filter((p) => p.status === "COMPLETED")
					.map((p) => p.lesson_id),
			),
		[progressQuery.data],
	);

	const lessonCounts = useMemo(
		() =>
			(structureQuery.data?.modules ?? []).reduce(
				(counts, m) => ({
					all: counts.all + m.lessons.length,
					required:
						counts.required + m.lessons.filter((l) => !l.is_optional).length,
				}),
				{ all: 0, required: 0 },
			),
		[structureQuery.data],
	);
	const completedRequiredLessons = useMemo(
		() =>
			(structureQuery.data?.modules ?? []).reduce(
				(sum, m) =>
					sum +
					m.lessons.filter(
						(l) => !l.is_optional && completedLessonIds.has(l.id),
					).length,
				0,
			),
		[completedLessonIds, structureQuery.data],
	);

	const progressPct =
		lessonCounts.required === 0
			? lessonCounts.all > 0
				? 100
				: 0
			: Math.round((completedRequiredLessons / lessonCounts.required) * 100);

	const certificateMutation = useMutation({
		mutationFn: () => learnApi.issueCertificate(getProfile(), auth, courseId),
		onSuccess: () => {
			queryClient.invalidateQueries({
				queryKey: ["learn", "certificates", "me"],
			});
		},
	});

	const course = structureQuery.data?.course;
	const modules = structureQuery.data?.modules ?? [];

	if (!courseId) {
		return (
			<div className="flex-1 overflow-auto">
				<div className="mx-auto max-w-3xl p-6 md:p-10">
					<Card>
						<CardHeader>
							<CardTitle>Course missing</CardTitle>
							<CardDescription>
								Open a course from the university catalog.
							</CardDescription>
						</CardHeader>
						<CardContent>
							<Button asChild variant="outline">
								<Link href="/learn">
									<ArrowLeft className="mr-2 h-4 w-4" />
									All courses
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
			{course?.banner_url && (
				<div
					className="h-48 bg-cover bg-center border-b"
					style={{ backgroundImage: `url(${course.banner_url})` }}
				/>
			)}
			<div className="mx-auto max-w-5xl p-6 md:p-8 lg:p-10 space-y-6">
				<Link
					href="/learn"
					className="inline-flex items-center gap-1 text-sm text-muted-foreground hover:text-foreground"
				>
					<ArrowLeft className="h-3 w-3" />
					All courses
				</Link>

				<header className="space-y-3">
					<div className="flex items-center gap-2 flex-wrap">
						<Badge>{course?.difficulty ?? "BEGINNER"}</Badge>
						<Badge variant="outline">{course?.category ?? "GENERAL"}</Badge>
						{course?.estimated_minutes ? (
							<Badge variant="outline" className="gap-1">
								<Clock className="h-3 w-3" />
								{course.estimated_minutes} min
							</Badge>
						) : null}
					</div>
					<h1 className="text-3xl font-semibold">{course?.name ?? courseId}</h1>
					{course?.description && (
						<p className="text-muted-foreground">{course.description}</p>
					)}
					<div className="flex items-center gap-3 flex-wrap">
						{!isEnrolled ? (
							<Button
								disabled={enrollMutation.isPending || !profile}
								onClick={() => enrollMutation.mutate()}
							>
								<PlayCircle className="h-4 w-4 mr-2" />
								Start course
							</Button>
						) : (
							<>
								<Badge variant="secondary" className="gap-1">
									<BookOpen className="h-3 w-3" />
									Enrolled
								</Badge>
								<div className="flex items-center gap-2 min-w-50">
									<Progress value={progressPct} className="w-40" />
									<span className="text-xs text-muted-foreground">
										{progressPct}%
									</span>
								</div>
								{progressPct === 100 && (
									<Button
										variant="default"
										onClick={() => certificateMutation.mutate()}
										disabled={certificateMutation.isPending}
									>
										<Award className="h-4 w-4 mr-2" />
										Claim certificate
									</Button>
								)}
							</>
						)}
					</div>
				</header>

				{course?.long_description && (
					<Card>
						<CardHeader>
							<CardTitle>About this course</CardTitle>
						</CardHeader>
						<CardContent className="prose prose-sm dark:prose-invert max-w-none whitespace-pre-wrap">
							{course.long_description}
						</CardContent>
					</Card>
				)}

				{modules.length === 0 ? (
					<Card>
						<CardHeader>
							<CardTitle>No content yet</CardTitle>
							<CardDescription>
								This course has no modules yet. Check back later or seed via the
								API.
							</CardDescription>
						</CardHeader>
					</Card>
				) : (
					<div className="space-y-3">
						{modules.map((m) => (
							<ModuleSection
								key={m.id}
								module={m}
								completedLessonIds={completedLessonIds}
								onOpen={(lessonId) =>
									router.push(
										`/learn/lesson?learnCourseId=${encodeURIComponent(courseId)}&learnModuleId=${encodeURIComponent(m.id)}&learnLessonId=${encodeURIComponent(lessonId)}`,
									)
								}
							/>
						))}
					</div>
				)}
			</div>
		</div>
	);
}

interface ModuleSectionProps {
	readonly module: ModuleWithLessons;
	readonly completedLessonIds: ReadonlySet<string>;
	readonly onOpen: (lessonId: string) => void;
}

function ModuleSection({
	module: m,
	completedLessonIds,
	onOpen,
}: ModuleSectionProps) {
	return (
		<Card>
			<CardHeader>
				<CardTitle>{m.title}</CardTitle>
				{m.description && <CardDescription>{m.description}</CardDescription>}
			</CardHeader>
			<CardContent className="p-0">
				<ul>
					{m.lessons.map((lesson) => {
						const done = completedLessonIds.has(lesson.id);
						return (
							<li key={lesson.id}>
								<button
									type="button"
									onClick={() => onOpen(lesson.id)}
									className="flex items-center gap-3 w-full px-4 py-3 hover:bg-accent border-t text-left"
								>
									{done ? (
										<CheckCircle2 className="h-4 w-4 text-green-500" />
									) : (
										<Circle className="h-4 w-4 text-muted-foreground" />
									)}
									<span className="flex-1 text-sm">{lesson.title}</span>
									<span className="text-xs text-muted-foreground">
										{lesson.estimated_minutes} min
									</span>
									{lesson.is_optional && (
										<Badge variant="outline" className="text-[10px]">
											optional
										</Badge>
									)}
									<ChevronRight className="h-4 w-4 text-muted-foreground" />
								</button>
							</li>
						);
					})}
				</ul>
			</CardContent>
		</Card>
	);
}
