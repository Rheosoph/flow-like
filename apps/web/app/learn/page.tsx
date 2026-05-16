"use client";
import { useQuery } from "@tanstack/react-query";
import {
	CourseCatalog,
	GlobalPermission,
	useBackend,
	useInvoke,
} from "@tm9657/flow-like-ui";
import { useRouter } from "next/navigation";
import { useMemo } from "react";
import { useAuth } from "react-oidc-context";
import { learnApi } from "../../lib/learn-api";

export default function LearnPage() {
	const router = useRouter();
	const auth = useAuth();
	const backend = useBackend();
	const profileQuery = useInvoke(
		backend.userState.getSettingsProfile,
		backend.userState,
		[],
	);
	const profile = profileQuery.data?.hub_profile ?? null;
	const profileId = profile?.id ?? "no-profile";
	const infoQuery = useInvoke(
		backend.userState.getInfo,
		backend.userState,
		[],
		Boolean(auth?.isAuthenticated),
		[auth?.user?.profile?.sub, auth?.isAuthenticated],
	);
	const getProfile = () => {
		if (!profile) {
			throw new Error("Profile is required for learning API calls.");
		}
		return profile;
	};
	const userPermissions = new GlobalPermission(infoQuery.data?.permission ?? 0);
	const isAdmin = userPermissions.contains(GlobalPermission.Admin);
	const canReadCourses = userPermissions.contains(GlobalPermission.ReadCourses);
	const canWriteCourses = userPermissions.contains(
		GlobalPermission.WriteCourses,
	);
	const canViewDrafts = canReadCourses || canWriteCourses || isAdmin;

	const coursesQuery = useQuery({
		queryKey: ["learn", "courses", profileId, { includeDrafts: canViewDrafts }],
		enabled: Boolean(profile && !auth.isLoading && !infoQuery.isLoading),
		queryFn: () =>
			learnApi.listCourses(getProfile(), auth, {
				includeUnpublished: canViewDrafts,
			}),
	});

	const enrollmentsQuery = useQuery({
		queryKey: ["learn", "enrollments", "me", profileId, auth.user?.profile.sub],
		enabled: Boolean(profile && auth.user),
		queryFn: () => learnApi.myEnrollments(getProfile(), auth),
	});

	const certificatesQuery = useQuery({
		queryKey: [
			"learn",
			"certificates",
			"me",
			profileId,
			auth.user?.profile.sub,
		],
		enabled: Boolean(profile && auth.user),
		queryFn: () => learnApi.myCertificates(getProfile(), auth),
	});

	const optInQuery = useQuery({
		queryKey: ["learn", "leaderboard", "me", profileId, auth.user?.profile.sub],
		enabled: Boolean(profile && auth.user),
		queryFn: () => learnApi.getMyOptIn(getProfile(), auth),
	});

	const pathsQuery = useQuery({
		queryKey: [
			"learn",
			"paths",
			profileId,
			{ includeUnpublished: canViewDrafts },
		],
		enabled: Boolean(profile && !auth.isLoading && !infoQuery.isLoading),
		queryFn: () =>
			learnApi.listLearningPaths(getProfile(), auth, {
				includeUnpublished: canViewDrafts,
			}),
	});

	const progressByCourseId = useMemo<Record<string, number>>(() => {
		const map: Record<string, number> = {};
		for (const e of enrollmentsQuery.data ?? []) {
			map[e.course_id] = e.completed_at ? 1 : 0.05;
		}
		return map;
	}, [enrollmentsQuery.data]);

	const stats = useMemo(() => {
		const enrollments = enrollmentsQuery.data ?? [];
		const completed = enrollments.filter((e) => e.completed_at !== null).length;
		return {
			enrolled: enrollments.length,
			completed,
			points: optInQuery.data?.total_points ?? 0,
			certificates: certificatesQuery.data?.length ?? 0,
		};
	}, [enrollmentsQuery.data, certificatesQuery.data, optInQuery.data]);

	const displayName =
		optInQuery.data?.display_name ||
		(typeof auth.user?.profile?.preferred_username === "string"
			? auth.user.profile.preferred_username
			: typeof auth.user?.profile?.name === "string"
				? auth.user.profile.name
				: null);

	return (
		<div className="flex-1 overflow-auto">
			<div className="mx-auto max-w-7xl p-6 md:p-10">
				<CourseCatalog
					courses={coursesQuery.data ?? []}
					paths={pathsQuery.data ?? []}
					progressByCourseId={progressByCourseId}
					onSelect={(c) =>
						router.push(`/learn/course?courseId=${encodeURIComponent(c.id)}`)
					}
					stats={stats}
					displayName={displayName}
					onOpenLeaderboard={() => router.push("/learn/leaderboard")}
					onOpenCertificates={() => router.push("/learn/certificates")}
					onOpenAuthoring={
						canViewDrafts ? () => router.push("/learn/admin") : undefined
					}
				/>
			</div>
		</div>
	);
}
