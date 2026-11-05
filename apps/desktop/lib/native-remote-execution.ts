import type { IIntercomEvent } from "@flow-like/flow-like-ui/lib/schema/events/intercom-event";
import type { IProfile } from "@flow-like/flow-like-ui/lib/schema/profile/profile";
import type { AuthContextProps } from "react-oidc-context";
import { fetcher, streamFetcher } from "./api";
import { authForHub, hubAuthUser } from "./hub-auth";
import { followNativeExecution } from "./native-execution-realtime";

interface RemoteExecutionOptions {
	owner: object;
	profile: IProfile;
	auth: AuthContextProps;
	path: string;
	body: unknown;
	onEvent: (event: IIntercomEvent) => void;
	onRunId?: (runId: string) => void;
}

const remoteRuns = new WeakMap<object, Map<string, () => Promise<void>>>();

function hasControlCharacter(value: string) {
	return Array.from(value).some((character) => {
		const code = character.charCodeAt(0);
		return code < 32 || code === 127;
	});
}

export async function cancelNativeRemoteExecution(
	owner: object,
	runId: string,
): Promise<boolean> {
	const cancel = remoteRuns.get(owner)?.get(runId);
	if (!cancel) return false;
	await cancel();
	return true;
}

/** HTTP streams stay attached; accepted queued runs replay through the shared native owner. */
export async function runNativeRemoteExecution(
	options: RemoteExecutionOptions,
): Promise<void> {
	const subject = options.auth.user?.profile.sub;
	const issuer =
		options.auth.user?.profile.iss ?? options.auth.settings?.authority;
	const matchesAccount = () => {
		const current = hubAuthUser(options.auth);
		return (
			authForHub(options.profile, options.auth) &&
			current?.profile.sub === subject &&
			(current?.profile.iss ?? options.auth.settings?.authority) === issuer
		);
	};
	const controller = new AbortController();
	const runs =
		remoteRuns.get(options.owner) ?? new Map<string, () => Promise<void>>();
	remoteRuns.set(options.owner, runs);
	let runId: string | undefined;
	let closed = false;
	let completed = false;
	let cancel: (() => Promise<void>) | undefined;
	const initiated = (id: string) => {
		if (runId) return;
		runId = id;
		cancel = async () => {
			// Stop detaches this window whatever the API answers; a failed DELETE still rejects.
			closed = true;
			controller.abort();
			if (!matchesAccount())
				throw new Error("The Hub account changed before cancellation");
			await fetcher(
				options.profile,
				`execution/run/${encodeURIComponent(id)}`,
				{ method: "DELETE" },
				options.auth,
			);
		};
		runs.set(id, cancel);
		options.onRunId?.(id);
	};
	const receive = (event: IIntercomEvent) => {
		if (closed || controller.signal.aborted) return;
		if (
			event.event_type === "run_initiated" &&
			typeof event.payload?.run_id === "string"
		) {
			initiated(event.payload.run_id);
		}
		options.onEvent(event);
		completed ||= event.event_type === "completed";
	};
	try {
		await streamFetcher<IIntercomEvent>(
			options.profile,
			options.path,
			{
				method: "POST",
				body: JSON.stringify(options.body),
				signal: controller.signal,
			},
			options.auth,
			receive,
			async (value) => {
				if (closed || controller.signal.aborted) return;
				const receipt = value as {
					run_id?: unknown;
					status?: unknown;
					poll_token?: unknown;
				} | null;
				if (
					!receipt ||
					typeof receipt.run_id !== "string" ||
					!receipt.run_id ||
					receipt.run_id.length > 256 ||
					hasControlCharacter(receipt.run_id) ||
					typeof receipt.status !== "string" ||
					typeof receipt.poll_token !== "string" ||
					!receipt.poll_token ||
					new TextEncoder().encode(receipt.poll_token).byteLength > 8192 ||
					hasControlCharacter(receipt.poll_token)
				) {
					throw new Error("Invalid background execution receipt");
				}
				const profileId = options.profile.id;
				if (!profileId)
					throw new Error(
						"Select a profile to receive background execution updates",
					);
				if (!subject || !issuer)
					throw new Error("Sign in to receive background execution updates");
				if (!matchesAccount())
					throw new Error(
						"The Hub account changed before execution updates started",
					);
				receive({
					event_id: `${receipt.run_id}:receipt`,
					event_type: "run_initiated",
					payload: { run_id: receipt.run_id },
					timestamp: {
						secs_since_epoch: Math.floor(Date.now() / 1000),
						nanos_since_epoch: 0,
					},
				});
				await followNativeExecution({
					profileId,
					subject,
					issuer,
					run: {
						run_id: receipt.run_id,
						poll_token: receipt.poll_token,
						after_sequence: -1,
					},
					signal: controller.signal,
					onEvent: receive,
				});
			},
		);
	} catch (error) {
		// A queued run rejects with its outcome after `completed`; like an SSE run, its events carry it.
		if (!controller.signal.aborted && !completed) throw error;
	} finally {
		closed = true;
		if (runId && runs.get(runId) === cancel) runs.delete(runId);
	}
}
