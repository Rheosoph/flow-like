"use client";

import {
	Alert,
	AlertDescription,
	AppCard,
	Avatar,
	AvatarFallback,
	AvatarImage,
	Badge,
	Button,
	Card,
	CardContent,
	type IApp,
	type IMetadata,
	Skeleton,
	useBackend,
	useInfiniteInvoke,
	useInvoke,
} from "@flow-like/flow-like-ui";
import { IAppSearchSort } from "@flow-like/flow-like-ui/lib/schema/app/app-search-query";
import type { IUserLookup } from "@flow-like/flow-like-ui/state/backend-state/types";
import { motion } from "framer-motion";
import {
	AlertCircle,
	Calendar,
	Loader2,
	Mail,
	Package,
	Sparkles,
	User,
} from "lucide-react";
import { useRouter, useSearchParams } from "next/navigation";
import { useMemo } from "react";

const ProfileSkeleton = () => (
	<div className="bg-gradient-to-br from-background via-background/50 to-primary/5 flex-1 min-h-0 overflow-auto">
		<div className="container mx-auto px-4 sm:px-6 lg:px-8 py-8 sm:py-12">
			<motion.div
				initial={{ opacity: 0, y: 20 }}
				animate={{ opacity: 1, y: 0 }}
				className="max-w-4xl mx-auto"
			>
				<div className="relative">
					<div className="absolute inset-0 bg-gradient-to-r from-primary/20 via-tertiary/20 to-tertiary/20 rounded-3xl blur-3xl opacity-30 animate-pulse" />
					<Card className="relative backdrop-blur-xl bg-background/80 border-0 shadow-2xl rounded-3xl overflow-hidden">
						<CardContent className="p-6 sm:p-8 lg:p-12">
							<div className="flex flex-col md:flex-row items-center gap-4 sm:gap-6 lg:gap-8">
								<Skeleton className="w-24 h-24 sm:w-28 sm:h-28 md:w-32 md:h-32 rounded-full" />
								<div className="flex-1 space-y-4 text-center md:text-left">
									<Skeleton className="h-8 w-64 mx-auto md:mx-0" />
									<Skeleton className="h-6 w-48 mx-auto md:mx-0" />
									<div className="flex flex-wrap gap-2 justify-center md:justify-start">
										<Skeleton className="h-6 w-20" />
										<Skeleton className="h-6 w-24" />
									</div>
								</div>
							</div>
						</CardContent>
					</Card>
				</div>
			</motion.div>
		</div>
	</div>
);

const ProfileError = ({ error }: { error: string }) => (
	<div className="bg-gradient-to-br from-background via-background/50 to-destructive/5 flex items-center justify-center flex-1 min-h-0 overflow-auto">
		<motion.div
			initial={{ opacity: 0, scale: 0.95 }}
			animate={{ opacity: 1, scale: 1 }}
			className="max-w-md mx-auto px-4"
		>
			<Alert className="border-destructive/20 bg-destructive/5">
				<AlertCircle className="h-4 w-4" />
				<AlertDescription className="text-center">
					{error || "Failed to load user profile"}
				</AlertDescription>
			</Alert>
		</motion.div>
	</div>
);

const ProfileContent = ({
	user,
	apps,
	hasNextPage,
	fetchNextPage,
	isFetchingNextPage,
	isAppsLoading,
	appsError,
}: {
	user: IUserLookup;
	apps: [IApp, IMetadata | undefined][];
	hasNextPage?: boolean;
	fetchNextPage: () => void;
	isFetchingNextPage: boolean;
	isAppsLoading: boolean;
	appsError: Error | null;
}) => {
	const router = useRouter();
	const displayName =
		user.name || user.preferred_username || user.username || "Unknown User";
	const username = user.preferred_username || user.username;
	const initials = displayName
		.split(" ")
		.map((n) => n[0])
		.join("")
		.toUpperCase()
		.slice(0, 2);
	const joinDate = new Date(user.created_at).toLocaleDateString("en-US", {
		year: "numeric",
		month: "long",
		day: "numeric",
	});

	const containerVariants = {
		hidden: { opacity: 0 },
		visible: {
			opacity: 1,
			transition: {
				staggerChildren: 0.1,
				delayChildren: 0.2,
			},
		},
	};

	const itemVariants = {
		hidden: { opacity: 0, y: 20 },
		visible: { opacity: 1, y: 0 },
	};

	return (
		<div className="relative overflow-auto flex-1 min-h-0">
			<div className="container mx-auto px-4 sm:px-6 lg:px-8 py-8 sm:py-12 relative z-10">
				<motion.div
					variants={containerVariants}
					initial="hidden"
					animate="visible"
					className="max-w-4xl mx-auto"
				>
					{/* Main Profile Card */}
					<motion.div variants={itemVariants} className="relative mb-8">
						<Card className="relative backdrop-blur-xl bg-background/80 border-0 shadow-2xl rounded-3xl overflow-hidden">
							<CardContent className="p-6 sm:p-8 lg:p-12">
								<div className="flex flex-col lg:flex-row items-center gap-4 sm:gap-6 lg:gap-8">
									<motion.div
										whileHover={{ scale: 1.05 }}
										transition={{ type: "spring", stiffness: 300 }}
										className="relative"
									>
										<div className="absolute inset-0 bg-gradient-to-r from-primary to-tertiary rounded-full blur-lg opacity-50 animate-pulse" />
										<Avatar className="w-24 h-24 sm:w-28 sm:h-28 md:w-32 md:h-32 relative z-10 border-2 md:border-4 border-background shadow-xl">
											<AvatarImage src={user.avatar_url} alt={displayName} />
											<AvatarFallback className="text-2xl font-bold bg-gradient-to-br from-primary to-tertiary text-white">
												{initials}
											</AvatarFallback>
										</Avatar>
										<motion.div
											animate={{ rotate: 360 }}
											transition={{
												duration: 20,
												repeat: Number.POSITIVE_INFINITY,
												ease: "linear",
											}}
											className="absolute -inset-2 border-2 border-dashed border-primary/30 rounded-full hidden sm:block"
										/>
									</motion.div>

									<div className="flex-1 text-center lg:text-left space-y-4">
										<motion.div variants={itemVariants} className="space-y-2">
											<h1 className="text-2xl sm:text-3xl md:text-4xl lg:text-5xl font-bold bg-gradient-to-r from-primary via-tertiary to-tertiary bg-clip-text text-transparent leading-tight break-words">
												{displayName}
											</h1>
											{username && (
												<p className="text-base sm:text-lg md:text-xl text-muted-foreground break-words">
													@{username}
												</p>
											)}
										</motion.div>

										<motion.div
											variants={itemVariants}
											className="flex flex-wrap gap-3 justify-center lg:justify-start"
										>
											{user.email && (
												<Badge
													variant="secondary"
													className="px-4 py-2 bg-primary/10 hover:bg-primary/20 transition-colors"
												>
													<Mail className="w-4 h-4 mr-2" />
													Contact Available
												</Badge>
											)}
											<Badge
												variant="outline"
												className="px-4 py-2 border-primary/30 hover:bg-primary/5 transition-colors"
											>
												<Calendar className="w-4 h-4 mr-2" />
												Joined {joinDate}
											</Badge>
											<Badge className="px-4 py-2 bg-gradient-to-r from-primary to-tertiary hover:opacity-90 transition-opacity">
												<Sparkles className="w-4 h-4 mr-2" />
												Flow-Like User
											</Badge>
										</motion.div>
									</div>
								</div>
							</CardContent>
						</Card>
					</motion.div>

					{/* Description Section */}
					{user.description && (
						<motion.div variants={itemVariants} className="relative mb-8">
							<div className="absolute inset-0 bg-gradient-to-r from-blue-500/10 via-tertiary/10 to-tertiary/10 rounded-2xl blur-2xl opacity-50" />
							<Card className="relative backdrop-blur-xl bg-background/60 border-0 shadow-xl rounded-2xl">
								<CardContent className="p-4 sm:p-6 lg:p-8">
									<h2 className="text-xl sm:text-2xl font-semibold mb-4 flex items-center gap-2">
										<User className="w-6 h-6 text-primary" />
										About
									</h2>
									<p className="text-base sm:text-lg leading-relaxed text-muted-foreground">
										{user.description}
									</p>
								</CardContent>
							</Card>
						</motion.div>
					)}

					{/* Additional Info Section */}
					{user.additional_info && (
						<motion.div variants={itemVariants} className="relative mb-8">
							<div className="absolute inset-0 bg-gradient-to-r from-green-500/10 via-tertiary/10 to-tertiary/10 rounded-2xl blur-2xl opacity-50" />
							<Card className="relative backdrop-blur-xl bg-background/60 border-0 shadow-xl rounded-2xl">
								<CardContent className="p-4 sm:p-6 lg:p-8">
									<h2 className="text-xl sm:text-2xl font-semibold mb-4 flex items-center gap-2">
										<Sparkles className="w-6 h-6 text-primary" />
										Additional Information
									</h2>
									<p className="text-base sm:text-lg leading-relaxed text-muted-foreground">
										{user.additional_info}
									</p>
								</CardContent>
							</Card>
						</motion.div>
					)}

					{/* Apps Section */}
					<motion.div variants={itemVariants} className="relative">
						<div className="absolute inset-0 bg-gradient-to-r from-purple-500/10 via-tertiary/10 to-tertiary/10 rounded-2xl blur-2xl opacity-50" />
						<Card className="relative backdrop-blur-xl bg-background/60 border-0 shadow-xl rounded-2xl">
							<CardContent className="p-4 sm:p-6 lg:p-8">
								<h2 className="text-xl sm:text-2xl font-semibold mb-6 flex items-center gap-2">
									<Package className="w-6 h-6 text-primary" />
									Published Apps
									{apps.length > 0 && (
										<Badge variant="secondary" className="ml-2">
											{apps.length}
										</Badge>
									)}
								</h2>

								{appsError && (
									<Alert className="mb-6 border-destructive/20 bg-destructive/5">
										<AlertCircle className="h-4 w-4" />
										<AlertDescription>
											Failed to load apps: {appsError.message}
										</AlertDescription>
									</Alert>
								)}

								{isAppsLoading ? (
									<div className="grid gap-6 md:grid-cols-2 lg:grid-cols-3">
										{[...Array(6)].map((_, i) => (
											<div key={i} className="space-y-3">
												<Skeleton className="h-40 sm:h-48 w-full rounded-lg" />
												<Skeleton className="h-4 w-3/4" />
												<Skeleton className="h-4 w-1/2" />
											</div>
										))}
									</div>
								) : apps.length === 0 ? (
									<div className="text-center py-12">
										<Package className="w-16 h-16 mx-auto text-muted-foreground/50 mb-4" />
										<p className="text-lg text-muted-foreground">
											{displayName} hasn&apos;t published any apps yet.
										</p>
									</div>
								) : (
									<>
										<div className="grid gap-6 md:grid-cols-2 lg:grid-cols-3">
											{apps.map(([app, metadata]) => (
												<AppCard
													key={app.id}
													app={app}
													variant="extended"
													metadata={metadata}
													className="w-full"
													onClick={() => router.push(`/store?id=${app.id}`)}
													href={`/store?id=${app.id}`}
												/>
											))}
										</div>

										{hasNextPage && (
											<div className="flex justify-center mt-8">
												<Button
													onClick={fetchNextPage}
													disabled={isFetchingNextPage}
													variant="outline"
													size="lg"
													className="px-8"
												>
													{isFetchingNextPage ? (
														<>
															<Loader2 className="w-4 h-4 mr-2 animate-spin" />
															Loading more...
														</>
													) : (
														"Load More Apps"
													)}
												</Button>
											</div>
										)}
									</>
								)}
							</CardContent>
						</Card>
					</motion.div>
				</motion.div>
			</div>
		</div>
	);
};

export default function ProfilePage() {
	const params = useSearchParams();
	const sub = params.get("sub") || "";
	const backend = useBackend();
	const user = useInvoke(backend.userState.lookupUser, backend.userState, [
		sub,
	]);
	const {
		data: apps,
		hasNextPage,
		fetchNextPage,
		isFetchingNextPage,
		isLoading: isAppsLoading,
		error: appsError,
	} = useInfiniteInvoke(backend.appState.searchApps, backend.appState, [
		undefined,
		undefined,
		undefined,
		undefined,
		sub,
		IAppSearchSort.BestRated,
		undefined,
	]);

	const combinedApps = useMemo(() => {
		if (!apps) return [];
		return apps.pages.flat();
	}, [apps]);

	if (user.isFetching) {
		return <ProfileSkeleton />;
	}

	if (user.error) {
		return <ProfileError error={user.error.message} />;
	}

	if (!user.data) {
		return <ProfileError error="User not found" />;
	}

	return (
		<ProfileContent
			user={user.data}
			apps={combinedApps}
			hasNextPage={hasNextPage}
			fetchNextPage={fetchNextPage}
			isFetchingNextPage={isFetchingNextPage}
			isAppsLoading={isAppsLoading}
			appsError={appsError}
		/>
	);
}
