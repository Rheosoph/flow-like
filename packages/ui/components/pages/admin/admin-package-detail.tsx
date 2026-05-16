"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useCallback } from "react";
import { useInvoke } from "../../../hooks/use-invoke";
import type {
	AdminPackageDetailResponse,
	PackageDetails,
	PackageReview,
	PackageReviewer,
	ReviewRequest,
} from "../../../lib/schema/wasm";
import { useBackend } from "../../../state/backend-state";
import { AdminPackageDetailView } from "../../store/admin-package-detail-view";

interface RawAuthorInfo {
	user_id?: string;
	username?: string | null;
	name?: string | null;
}

interface RawPackageDetails {
	id: string;
	name: string;
	description: string;
	version: string;
	authors?: RawAuthorInfo[] | string[];
	license?: string;
	homepage?: string;
	repository?: string;
	keywords?: string[];
	status: string;
	visibility: string;
	verified: boolean;
	downloadCount?: number;
	download_count?: number;
	wasmSize?: number;
	wasm_size?: number;
	nodes: unknown;
	permissions: unknown;
	price?: number;
	primaryCategory?: string;
	primary_category?: string;
	secondaryCategory?: string;
	secondary_category?: string;
	createdAt?: string;
	created_at?: string;
	updatedAt?: string;
	updated_at?: string;
	publishedAt?: string;
	published_at?: string | null;
	readme?: string;
	submitterId?: string;
	submitter_id?: string | null;
}

interface RawPackageReview {
	id: string;
	packageId?: string;
	package_id?: string;
	reviewerId?: string;
	reviewer_id?: string;
	action: string;
	comment?: string;
	securityScore?: number;
	security_score?: number;
	codeQualityScore?: number;
	code_quality_score?: number;
	documentationScore?: number;
	documentation_score?: number;
	createdAt?: string;
	created_at?: string;
	reviewer?: {
		userId?: string;
		user_id?: string;
		username?: string | null;
		name?: string | null;
		avatar?: string | null;
		role?: string | null;
	};
}

interface RawAdminPackageDetailResponse {
	package: RawPackageDetails;
	reviews: RawPackageReview[];
}

function normalizeAuthors(authors: RawPackageDetails["authors"]): string[] {
	if (!authors) {
		return [];
	}

	return authors
		.map((author) => {
			if (typeof author === "string") {
				return author;
			}

			return author.name ?? author.username ?? author.user_id ?? "";
		})
		.filter((author): author is string => Boolean(author));
}

function normalizePackage(pkg: RawPackageDetails): PackageDetails {
	return {
		id: pkg.id,
		name: pkg.name,
		description: pkg.description,
		version: pkg.version,
		authors: normalizeAuthors(pkg.authors),
		license: pkg.license,
		homepage: pkg.homepage,
		repository: pkg.repository,
		keywords: pkg.keywords ?? [],
		status: pkg.status.toLowerCase() as PackageDetails["status"],
		visibility: pkg.visibility.toLowerCase() as PackageDetails["visibility"],
		verified: pkg.verified,
		downloadCount: pkg.downloadCount ?? pkg.download_count ?? 0,
		wasmSize: pkg.wasmSize ?? pkg.wasm_size ?? 0,
		nodes: Array.isArray(pkg.nodes) ? pkg.nodes : [],
		permissions: (pkg.permissions ?? {}) as PackageDetails["permissions"],
		price: pkg.price ?? 0,
		primaryCategory: (pkg.primaryCategory ??
			pkg.primary_category) as PackageDetails["primaryCategory"],
		secondaryCategory: (pkg.secondaryCategory ??
			pkg.secondary_category) as PackageDetails["secondaryCategory"],
		createdAt: pkg.createdAt ?? pkg.created_at ?? "",
		updatedAt: pkg.updatedAt ?? pkg.updated_at ?? "",
		publishedAt: pkg.publishedAt ?? pkg.published_at ?? undefined,
		readme: pkg.readme,
		submitterId: pkg.submitterId ?? pkg.submitter_id ?? undefined,
	};
}

function normalizeReview(review: RawPackageReview): PackageReview {
	const reviewer = review.reviewer
		? ({
				userId: review.reviewer.userId ?? review.reviewer.user_id ?? "",
				username: review.reviewer.username ?? undefined,
				name: review.reviewer.name ?? undefined,
				avatar: review.reviewer.avatar ?? undefined,
				role: review.reviewer.role ?? undefined,
			} satisfies PackageReviewer)
		: undefined;

	return {
		id: review.id,
		packageId: review.packageId ?? review.package_id ?? "",
		reviewerId: review.reviewerId ?? review.reviewer_id ?? "",
		reviewer,
		action: review.action as PackageReview["action"],
		comment: review.comment,
		securityScore: review.securityScore ?? review.security_score,
		codeQualityScore: review.codeQualityScore ?? review.code_quality_score,
		documentationScore: review.documentationScore ?? review.documentation_score,
		createdAt: review.createdAt ?? review.created_at ?? "",
	};
}

function normalizePackageDetail(
	response: RawAdminPackageDetailResponse,
): AdminPackageDetailResponse {
	return {
		package: normalizePackage(response.package),
		reviews: response.reviews.map(normalizeReview),
	};
}

export interface AdminPackageDetailProps {
	packageId: string;
	onBack: () => void;
	onSuccess?: () => void;
}

export function AdminPackageDetail({
	packageId,
	onBack,
	onSuccess,
}: AdminPackageDetailProps) {
	const backend = useBackend();
	const queryClient = useQueryClient();

	const profile = useInvoke(
		backend.userState.getProfile,
		backend.userState,
		[],
	);

	const packageDetail = useQuery<
		RawAdminPackageDetailResponse,
		Error,
		AdminPackageDetailResponse
	>({
		queryKey: ["admin", "packages", packageId],
		queryFn: async () => {
			if (!profile.data) throw new Error("Profile not loaded");
			return backend.apiState.get<RawAdminPackageDetailResponse>(
				profile.data,
				`admin/packages/${packageId}`,
			);
		},
		enabled: !!profile.data && !!packageId,
		select: normalizePackageDetail,
	});

	const submitReview = useMutation({
		mutationFn: async (review: ReviewRequest) => {
			if (!profile.data) throw new Error("Profile not loaded");
			return backend.apiState.post(
				profile.data,
				`admin/packages/${packageId}/review`,
				review,
			);
		},
		onSuccess: () => {
			onSuccess?.();
			queryClient.invalidateQueries({
				queryKey: ["admin", "packages", packageId],
			});
		},
	});

	const updatePackage = useMutation({
		mutationFn: async (data: { status?: string; verified?: boolean }) => {
			if (!profile.data) throw new Error("Profile not loaded");
			return backend.apiState.patch(
				profile.data,
				`admin/packages/${packageId}`,
				data,
			);
		},
		onSuccess: () => {
			onSuccess?.();
			queryClient.invalidateQueries({ queryKey: ["admin", "packages"] });
		},
	});

	const handleSubmitReview = useCallback(
		(review: ReviewRequest) => submitReview.mutate(review),
		[submitReview],
	);

	const handleUpdatePackage = useCallback(
		(data: { status?: string; verified?: boolean }) =>
			updatePackage.mutate(data),
		[updatePackage],
	);

	return (
		<AdminPackageDetailView
			packageDetail={packageDetail.data}
			isLoading={packageDetail.isLoading}
			onBack={onBack}
			onSubmitReview={handleSubmitReview}
			onUpdatePackage={handleUpdatePackage}
			isSubmittingReview={submitReview.isPending}
			isUpdatingPackage={updatePackage.isPending}
		/>
	);
}
