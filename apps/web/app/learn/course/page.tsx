"use client";
import {
	Button,
	CourseDetailView,
	useBackend,
	useInvoke,
} from "@flow-like/flow-like-ui";
import { useTranslation } from "@flow-like/locales";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { ArrowLeft } from "lucide-react";
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
	const { t } = useTranslation("common");
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

	const enrollment = useMemo(
		() =>
			(enrollmentsQuery.data ?? []).find((e) => e.course_id === courseId) ??
			null,
		[enrollmentsQuery.data, courseId],
	);
	const isEnrolled = Boolean(courseId) && enrollment !== null;

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

	const certificateMutation = useMutation({
		mutationFn: () => learnApi.issueCertificate(getProfile(), auth, courseId),
		onSuccess: () => {
			queryClient.invalidateQueries({
				queryKey: ["learn", "certificates", "me"],
			});
		},
	});

	const workspaceAppId = useMemo(() => {
		const ids = Object.values(enrollment?.linked_app_ids ?? {});
		return ids[0] ?? null;
	}, [enrollment]);

	if (!courseId) {
		return (
			<div className="flex-1 overflow-auto">
				<div className="mx-auto max-w-3xl p-6 md:p-10">
					<div className="rounded-xl border border-border/70 p-6">
						<h1 className="text-lg font-semibold">
							{t("courseMissing", "Course missing")}
						</h1>
						<p className="mt-1 text-sm text-muted-foreground">
							{t(
								"openACourseFromTheUniversityCatalog",
								"Open a course from the university catalog.",
							)}
						</p>
						<Button asChild variant="outline" className="mt-4">
							<Link href="/learn">
								<ArrowLeft className="mr-2 h-4 w-4" />
								{t("allCourses", "All courses")}
							</Link>
						</Button>
					</div>
				</div>
			</div>
		);
	}

	return (
		<CourseDetailView
			courseId={courseId}
			course={structureQuery.data?.course}
			modules={structureQuery.data?.modules ?? []}
			completedLessonIds={completedLessonIds}
			isEnrolled={isEnrolled}
			workspaceAppId={workspaceAppId}
			enrollPending={enrollMutation.isPending || !profile}
			certificatePending={certificateMutation.isPending}
			onBack={() => router.push("/learn")}
			onEnroll={() => enrollMutation.mutate()}
			onClaimCertificate={() => certificateMutation.mutate()}
			onOpenWorkspace={(appId) =>
				router.push(`/library/config?id=${encodeURIComponent(appId)}`)
			}
			onOpenLesson={(moduleId, lessonId) =>
				router.push(
					`/learn/lesson?learnCourseId=${encodeURIComponent(courseId)}&learnModuleId=${encodeURIComponent(moduleId)}&learnLessonId=${encodeURIComponent(lessonId)}`,
				)
			}
		/>
	);
}
