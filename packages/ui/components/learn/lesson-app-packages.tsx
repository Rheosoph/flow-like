"use client";

import {
	type UseQueryResult,
	useQueries,
	useQueryClient,
} from "@tanstack/react-query";
import { useCallback } from "react";
import { isHubUnavailable } from "../../lib/api-error";
import type { Challenge, LessonAppRef } from "../../lib/learn/types";
import { asArray } from "../../lib/response-shape";
import type { IBlockedPackage } from "../../lib/schema/app/fork";
import {
	ForkBlockedPackages,
	usePendingCheckouts,
} from "../settings/forking/fork-blocked-packages";
import { invalidateAppPackageQueries } from "../store/app-packages/use-package-manifests";

/** How the learner's copy of a course app stands against its template's packages. */
export interface LessonAppCopy {
	readonly app_id: string;
	/** Template packages the copy still lacks, each with what stands in the way. */
	readonly blocked_packages?: readonly IBlockedPackage[];
	/** Packages the sync just added to the copy. */
	readonly added_packages?: readonly string[];
}

interface LessonAppPackagesProps {
	courseId: string;
	profileId: string;
	appRefs: readonly LessonAppRef[];
	challenges: readonly Challenge[];
	linkedAppIds: Readonly<Record<string, string>>;
	syncCopyPackages(alias: string): Promise<LessonAppCopy>;
	checkouts: ReturnType<typeof useLessonPackageCheckouts>;
}

const COPY_STALE_MS = 5 * 60_000;
const COPY_KEY = ["learn", "shared-app-packages"] as const;
const SYNC_RETRIES = 2;

/** The alias a board challenge works in, when it names one. */
function challengeAlias(challenge: Challenge): string | undefined {
	if (challenge.kind !== "BOARD_RIDDLE" && challenge.kind !== "EXECUTE_NODE")
		return undefined;
	const payload = challenge.payload as {
		appAlias?: unknown;
		app_alias?: unknown;
	};
	const alias = payload.appAlias ?? payload.app_alias;
	return typeof alias === "string" ? alias : undefined;
}

/** The aliases a lesson works in that the learner already has an app for. */
function linkedAliases(
	appRefs: readonly LessonAppRef[],
	challenges: readonly Challenge[],
	linkedAppIds: Readonly<Record<string, string>>,
): string[] {
	const named = [
		...appRefs.map((ref) => ref.app_alias),
		...challenges.map(challengeAlias),
	];
	const aliases = new Set<string>();
	for (const alias of named) {
		if (alias && linkedAppIds[alias]) aliases.add(alias);
	}
	return [...aliases];
}

function blockedAcrossCopies(
	copies: UseQueryResult<LessonAppCopy>[],
): IBlockedPackage[] {
	const byId = new Map<string, IBlockedPackage>();
	for (const copy of copies) {
		for (const pkg of asArray(copy.data?.blocked_packages))
			byId.set(pkg.package_id, pkg);
	}
	return [...byId.values()];
}

/**
 * Only a sync the hub never ruled on is repeated. A refusal stands: the alias
 * has no copy, or the hub predates this route.
 */
function retrySync(failures: number, error: unknown): boolean {
	return failures < SYNC_RETRIES && isHubUnavailable(error);
}

/**
 * Syncs the copies behind `aliases` and returns the packages they still lack.
 * `syncCopies` syncs them again, which a row asks for once it resolved.
 */
function useBlockedPackages(
	courseId: string,
	profileId: string,
	aliases: readonly string[],
	syncCopyPackages: LessonAppPackagesProps["syncCopyPackages"],
) {
	const queryClient = useQueryClient();
	const blocked = useQueries({
		queries: aliases.map((alias) => ({
			queryKey: [...COPY_KEY, courseId, alias, profileId],
			queryFn: async (): Promise<LessonAppCopy> => {
				const copy = await syncCopyPackages(alias);
				if (asArray(copy.added_packages).length > 0)
					invalidateAppPackageQueries(queryClient, copy.app_id);
				return copy;
			},
			staleTime: COPY_STALE_MS,
			// A row that resolves asks for the next sync itself.
			refetchOnWindowFocus: false,
			retry: retrySync,
			// An offer to buy must not outlive the session that computed it.
			meta: { persist: false },
		})),
		combine: blockedAcrossCopies,
	});
	const syncCopies = useCallback(
		() =>
			void queryClient.invalidateQueries({
				queryKey: [...COPY_KEY, courseId],
			}),
		[queryClient, courseId],
	);
	return { blocked, syncCopies };
}

/**
 * The payments the notice's rows are waiting for. The lesson body is rebuilt
 * while the next lesson loads and whenever the workspace layout changes, so
 * the lesson page, which outlives both, keeps them and hands them to
 * {@link LessonAppPackages}.
 */
export function useLessonPackageCheckouts() {
	return usePendingCheckouts();
}

/**
 * The packages a lesson's app templates use that the learner's copies lack,
 * each with the way to get it where there is one. A copy is made without the
 * packages its learner can't use; syncing it adds the ones they can use by
 * then, so a row that resolves syncs the copies again.
 *
 * Only aliases the learner already has an app for are synced: the copy is
 * made when they start the lesson, and the hub refuses an alias without one.
 */
export function LessonAppPackages(props: Readonly<LessonAppPackagesProps>) {
	const { blocked, syncCopies } = useBlockedPackages(
		props.courseId,
		props.profileId,
		linkedAliases(props.appRefs, props.challenges, props.linkedAppIds),
		props.syncCopyPackages,
	);

	if (blocked.length === 0) return null;
	return (
		<ForkBlockedPackages
			context="copy"
			packages={blocked}
			onAccessChanged={syncCopies}
			onRequestSent={syncCopies}
			onCheckoutPendingChange={props.checkouts.handleCheckoutPendingChange}
			pendingCheckouts={props.checkouts.pendingCheckouts}
		/>
	);
}
