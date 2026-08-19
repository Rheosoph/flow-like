"use client";
import { useTranslation } from "@flow-like/locales";
import {
	Badge,
	Button,
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
	GlobalPermission,
	Input,
	LearningPathsAdmin,
	Skeleton,
	useBackend,
	useInvoke,
} from "@flow-like/flow-like-ui";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
	ArrowLeft,
	BookOpenCheck,
	Eye,
	GraduationCap,
	LockKeyhole,
	Pencil,
	Plus,
	ShieldCheck,
	Sparkles,
} from "lucide-react";
import Link from "next/link";
import { useRouter } from "next/navigation";
import { useMemo, useState } from "react";
import { useAuth } from "react-oidc-context";
import { toast } from "sonner";
import { learnApi } from "../../../lib/learn-api";

export default function AdminCoursesPage() {
	const { t } = useTranslation("common");
	const auth = useAuth();
	const router = useRouter();
	const queryClient = useQueryClient();
	const backend = useBackend();
	const profileQuery = useInvoke(
		backend.userState.getSettingsProfile,
		backend.userState,
		[],
	);
	const infoQuery = useInvoke(
		backend.userState.getInfo,
		backend.userState,
		[],
		Boolean(auth?.isAuthenticated),
		[auth?.user?.profile?.sub, auth?.isAuthenticated],
	);
	const profile = profileQuery.data?.hub_profile ?? null;
	const profileId = profile?.id ?? "no-profile";
	const getProfile = () => {
		if (!profile) {
			throw new Error("Profile is required for course authoring.");
		}
		return profile;
	};
	const userPermissions = new GlobalPermission(infoQuery.data?.permission ?? 0);
	const isAdmin = userPermissions.contains(GlobalPermission.Admin);
	const canRead = userPermissions.contains(GlobalPermission.ReadCourses);
	const canWrite = userPermissions.contains(GlobalPermission.WriteCourses);
	const canViewAdmin = canRead || canWrite || isAdmin;
	const canManageCourses = canWrite || isAdmin;
	const checkingAccess = auth.isLoading || infoQuery.isLoading;

	const coursesQuery = useQuery({
		queryKey: [
			"learn",
			"admin",
			"courses",
			profileId,
			{ includeDrafts: canViewAdmin },
		],
		enabled: Boolean(profile && canViewAdmin),
		queryFn: () =>
			learnApi.listCourses(getProfile(), auth, {
				includeUnpublished: canViewAdmin,
				limit: 200,
			}),
	});

	const [newId, setNewId] = useState("");
	const [newTitle, setNewTitle] = useState("");

	const createMutation = useMutation({
		mutationFn: () => {
			const id =
				newId.trim() ||
				`course-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 6)}`;
			return learnApi.upsertCourse(getProfile(), auth, id, {
				language: "en",
				name: newTitle.trim() || "Untitled course",
				is_published: false,
			});
		},
		onSuccess: (created) => {
			setNewId("");
			setNewTitle("");
			queryClient.invalidateQueries({
				queryKey: ["learn", "admin", "courses"],
			});
			queryClient.invalidateQueries({ queryKey: ["learn", "courses"] });
			router.push(
				`/learn/admin/course?courseId=${encodeURIComponent(created.id)}`,
			);
		},
		onError: (err) => {
			console.error(err);
			toast.error("Could not create course");
		},
	});

	const filteredCourses = useMemo(
		() => coursesQuery.data ?? [],
		[coursesQuery.data],
	);

	if (checkingAccess) {
		return (
			<div className="flex-1 overflow-auto">
				<div className="mx-auto max-w-5xl space-y-6 p-6 md:p-10">
					<section className="relative overflow-hidden rounded-2xl border border-border/60 bg-linear-to-br from-sky-500/10 via-card/90 to-emerald-500/10 p-6 shadow-sm">
						<div className="absolute inset-x-0 top-0 h-1 bg-linear-to-r from-sky-300 via-emerald-300 to-amber-300" />
						<div className="flex items-center gap-3">
							<Skeleton className="h-12 w-12 rounded-2xl" />
							<div className="space-y-2">
								<Skeleton className="h-6 w-44" />
								<Skeleton className="h-4 w-72 max-w-full" />
							</div>
						</div>
					</section>
				</div>
			</div>
		);
	}

	if (!canViewAdmin) {
		const requiredPermissions = [
			{
				label: t('readcourses', 'ReadCourses'),
				description: t('reviewPublicCoursesAndDrafts', 'Review public courses and drafts.'),
				icon: Eye,
			},
			{
				label: t('writecourses', 'WriteCourses'),
				description: t('createAndMaintainCourseContent', 'Create and maintain course content.'),
				icon: Pencil,
			},
			{
				label: t('admin1', 'Admin (1)'),
				description: t('globalAdministratorAccess', 'Global administrator access.'),
				icon: ShieldCheck,
			},
		];

		return (
			<div className="flex-1 overflow-auto">
				<div className="mx-auto max-w-5xl space-y-6 p-6 md:p-10">
					<section className="relative overflow-hidden rounded-2xl border border-border/60 bg-linear-to-br from-sky-500/10 via-card/90 to-rose-500/10 p-6 shadow-sm md:p-8">
						<div className="absolute inset-x-0 top-0 h-1 bg-linear-to-r from-sky-300 via-emerald-300 to-rose-300" />
						<div className="flex flex-col gap-6 md:flex-row md:items-end md:justify-between">
							<div className="max-w-2xl space-y-4">
								<div className="inline-flex items-center gap-2 rounded-full border border-border/60 bg-background/50 px-3 py-1 text-xs font-medium text-muted-foreground">
									<LockKeyhole className="h-3.5 w-3.5" />
									{t('courseAdmin', 'Course admin')}
								</div>
								<div className="space-y-2">
									<h1 className="text-3xl font-semibold tracking-tight">
										{t('courseAccessNeeded', 'Course access needed')}
									</h1>
									<p className="text-sm leading-6 text-muted-foreground">
										{t('universityAdminOpensForUsersWithReadcoursesWritecoursesOrTheAdminPermissionAskAnAdminToGrantOneOfTheseGlobalPermissions', "University admin opens for users with ReadCourses, WriteCourses, or the Admin permission. Ask an admin to grant one of these global permissions.")}
									</p>
								</div>
							</div>
							<div className="flex flex-wrap gap-2">
								<Button asChild variant="outline">
									<Link href="/learn">
										<ArrowLeft className="mr-2 h-4 w-4" />
										{t('university', 'University')}
									</Link>
								</Button>
								<Button asChild variant="secondary">
									<Link href="/admin">
										<ShieldCheck className="mr-2 h-4 w-4" />
										{t('adminDashboard', 'Admin Dashboard')}
									</Link>
								</Button>
							</div>
						</div>
					</section>

					<div className="grid gap-3 md:grid-cols-3">
						{requiredPermissions.map((item) => {
							const Icon = item.icon;
							return (
								<Card key={item.label} className="border-border/60 bg-card/70">
									<CardHeader className="space-y-3">
										<div className="flex h-10 w-10 items-center justify-center rounded-xl border border-border/60 bg-background/60">
											<Icon className="h-4 w-4 text-muted-foreground" />
										</div>
										<div>
											<CardTitle className="text-base">{item.label}</CardTitle>
											<CardDescription>{item.description}</CardDescription>
										</div>
									</CardHeader>
								</Card>
							);
						})}
					</div>
				</div>
			</div>
		);
	}

	return (
		<div className="flex-1 overflow-auto">
			<div className="mx-auto max-w-6xl space-y-6 p-6 md:p-8 lg:p-10">
				<section className="relative overflow-hidden rounded-2xl border border-border/60 bg-linear-to-br from-sky-500/15 via-card/90 to-emerald-500/10 p-6 shadow-sm md:p-8">
					<div className="absolute inset-x-0 top-0 h-1 bg-linear-to-r from-sky-300 via-emerald-300 to-amber-300" />
					<div className="flex flex-col gap-6 lg:flex-row lg:items-end lg:justify-between">
						<div className="max-w-2xl space-y-4">
							<div className="inline-flex items-center gap-2 rounded-full border border-border/60 bg-background/50 px-3 py-1 text-xs font-medium text-muted-foreground">
								<GraduationCap className="h-3.5 w-3.5" />
								{t('flowlikeUniversity', 'FlowLike University')}
							</div>
							<div className="space-y-2">
								<h1 className="text-3xl font-semibold tracking-tight">
									{t('courseAdmin', 'Course admin')}
								</h1>
								<p className="text-sm leading-6 text-muted-foreground">
									{t('shapeTheCourseLibraryReviewDraftsAndKeepPublishedLearningPathsCalmAndUseful', "Shape the course library, review drafts, and keep published learning paths calm and useful.")}
								</p>
							</div>
							<div className="flex flex-wrap gap-2">
								<Badge variant={canRead || isAdmin ? "secondary" : "outline"}>
									<Eye className="mr-1.5 h-3 w-3" />
									{t('readcourses', 'ReadCourses')}
								</Badge>
								<Badge
									variant={canManageCourses ? "secondary" : "outline"}
									className={canManageCourses ? "bg-emerald-500/15" : undefined}
								>
									<Pencil className="mr-1.5 h-3 w-3" />
									{t('writecourses', 'WriteCourses')}
								</Badge>
								{isAdmin && (
									<Badge className="bg-amber-500/15 text-amber-700 hover:bg-amber-500/20 dark:text-amber-300">
										<ShieldCheck className="mr-1.5 h-3 w-3" />
										{t('admin', 'Admin')}
									</Badge>
								)}
							</div>
						</div>
						<div className="flex flex-wrap gap-2">
							<Button asChild variant="outline">
								<Link href="/learn">
									<ArrowLeft className="mr-2 h-4 w-4" />
									{t('university', 'University')}
								</Link>
							</Button>
							<Button asChild variant="secondary">
								<Link href="/admin">
									<ShieldCheck className="mr-2 h-4 w-4" />
									{t('admin', 'Admin')}
								</Link>
							</Button>
						</div>
					</div>
				</section>

				{canManageCourses ? (
					<Card className="border-border/60 bg-card/80">
						<CardHeader>
							<CardTitle className="flex items-center gap-2 text-base">
								<Sparkles className="h-4 w-4 text-sky-400" />
								{`Create a new course`}
							</CardTitle>
							<CardDescription>
								{t('newCoursesStartAsDraftsAndAppearToCourseAdminsBeforeTheyArePublished', "New courses start as drafts and appear to course admins before they are published.")}
							</CardDescription>
						</CardHeader>
						<CardContent>
							<form
								className="grid grid-cols-1 items-end gap-3 md:grid-cols-[1fr_1fr_auto]"
								onSubmit={(e) => {
									e.preventDefault();
									createMutation.mutate();
								}}
							>
								<div className="space-y-1.5">
									<label
										htmlFor="new-course-id"
										className="text-xs font-medium text-muted-foreground"
									>
										{t('idOptional', 'ID (optional)')}
									</label>
									<Input
										id="new-course-id"
										value={newId}
										onChange={(e) => setNewId(e.target.value)}
										placeholder="auto-generated"
									/>
								</div>
								<div className="space-y-1.5">
									<label
										htmlFor="new-course-title"
										className="text-xs font-medium text-muted-foreground"
									>
										{t('title', 'Title')}
									</label>
									<Input
										id="new-course-title"
										value={newTitle}
										onChange={(e) => setNewTitle(e.target.value)}
										placeholder={t('buildYourFirstChatAgent', 'Build your first chat agent')}
									/>
								</div>
								<Button type="submit" disabled={createMutation.isPending}>
									<Plus className="mr-2 h-4 w-4" />
									{t('create', 'Create')}
								</Button>
							</form>
						</CardContent>
					</Card>
				) : (
					<Card className="border-sky-500/30 bg-sky-500/5">
						<CardHeader>
							<CardTitle className="flex items-center gap-2 text-base">
								<Eye className="h-4 w-4 text-sky-400" />
								{t('readonlyCourseAccess', 'Read-only course access')}
							</CardTitle>
							<CardDescription>
								{t('youCanReviewPublicAndDraftCoursesCreatingOrEditingCoursesRequiresWritecoursesOrAdmin', "You can review public and draft courses. Creating or editing courses requires WriteCourses or Admin.")}
							</CardDescription>
						</CardHeader>
					</Card>
				)}

				<LearningPathsAdmin
					api={learnApi}
					profile={profile}
					auth={auth}
					courses={filteredCourses}
					canManage={canManageCourses}
				/>

				<div className="space-y-3">
					<div className="flex flex-col gap-2 sm:flex-row sm:items-end sm:justify-between">
						<div>
							<h2 className="text-lg font-semibold">{t('courseLibrary', 'Course library')}</h2>
							<p className="text-sm text-muted-foreground">{t('countCourseSAvailable', { defaultValue_one: "{{count}} course available", defaultValue_other: "{{count}} courses available", count: filteredCourses.length })}</p>
						</div>
						<Badge variant="outline" className="w-fit">
							<BookOpenCheck className="mr-1.5 h-3 w-3" />
							{t('draftsIncluded', 'Drafts included')}
						</Badge>
					</div>

					{filteredCourses.length === 0 ? (
						<Card className="border-dashed bg-card/60">
							<CardHeader>
								<CardTitle className="text-base">{t('noCoursesYet', 'No courses yet')}</CardTitle>
								<CardDescription>
									{canManageCourses
										? t('createTheFirstDraftAbove', 'Create the first draft above.')
										: t('courseDraftsAndPublishedCoursesWillAppearHere', 'Course drafts and published courses will appear here.')}
								</CardDescription>
							</CardHeader>
						</Card>
					) : (
						filteredCourses.map((c) => (
							<Card
								key={c.id}
								className="border-border/60 bg-card/75 transition-colors hover:border-sky-400/40"
							>
								<CardHeader className="flex flex-row items-center gap-3 space-y-0">
									<div className="grid h-11 w-11 shrink-0 place-items-center overflow-hidden rounded-xl border border-border/60 bg-background/60">
										{c.icon_url ? (
											<img
												src={c.icon_url}
												alt=""
												className="h-full w-full object-cover"
											/>
										) : (
											<GraduationCap className="h-4 w-4 text-muted-foreground" />
										)}
									</div>
									<div className="min-w-0 flex-1">
										<CardTitle className="truncate text-base">
											{c.name ?? c.id}
										</CardTitle>
										<CardDescription className="line-clamp-1">
											{c.description ?? c.id}
										</CardDescription>
									</div>
									<Badge variant={c.is_published ? "default" : "outline"}>
										{c.is_published ? "Public" : "Draft"}
									</Badge>
									<Badge variant="outline" className="hidden sm:inline-flex">
										{c.difficulty}
									</Badge>
									<Badge variant="outline" className="hidden md:inline-flex">
										{c.category}
									</Badge>
									{canManageCourses && (
										<Button
											variant="outline"
											size="sm"
											onClick={() =>
												router.push(
													`/learn/admin/course?courseId=${encodeURIComponent(c.id)}`,
												)
											}
										>
											<Pencil className="mr-1.5 h-3.5 w-3.5" />
											{t('edit', 'Edit')}
										</Button>
									)}
								</CardHeader>
							</Card>
						))
					)}
				</div>
			</div>
		</div>
	);
}
