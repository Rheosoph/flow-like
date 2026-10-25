import fixture from "../../apps/desktop/lib/doc-screenshot/fixtures/onboarding.tauri.json";
import type { IBit } from "../../packages/ui/lib/schema/bit/bit";
import type { IHub } from "../../packages/ui/lib/schema/hub/hub";
import type { ISettingsProfile } from "../../packages/ui/types";

const params = new URLSearchParams(location.search);

export const qa = {
	commands: [] as { command: string; args?: unknown }[],
	signIns: 0,
	profiles: structuredClone(
		fixture.responses.get_default_profiles,
	) as unknown as [[ISettingsProfile, IBit[]][], IHub],
	failProfiles: params.has("error"),
	failSignIn: false,
	serverProfiles: [] as unknown[],
	releaseProfiles: undefined as undefined | (() => void),
	failRestore: params.has("restore-error"),
	holdRestore: params.has("hold-restore"),
	releaseRestore: undefined as undefined | (() => void),
	holdUpsert: false,
	releaseUpsert: undefined as undefined | (() => void),
	holdSizes: false,
	sizeRequests: [] as { bitId: string; resolve: (size: number) => void }[],
	unmount: undefined as undefined | (() => void),
	setSignedIn: undefined as undefined | ((signedIn: boolean) => void),
};
declare global {
	interface Window {
		onboardingQa: typeof qa;
	}
}
window.onboardingQa = qa;
if (params.has("encoded"))
	qa.profiles[0][0][0].hub_profile.id = "starter + chat&one";

export async function invoke(command: string, args?: unknown) {
	qa.commands.push({ command, args });
	if (command === "get_default_profiles") {
		if (params.has("loading"))
			await new Promise<void>((resolve) => {
				qa.releaseProfiles = resolve;
			});
		if (qa.failProfiles) throw new Error("Fixture profiles unavailable");
		return params.has("empty") ? [[], {}] : qa.profiles;
	}
	if (command === "get_profiles") return [];
	if (command === "upsert_profile" && qa.holdUpsert)
		await new Promise<void>((resolve) => {
			qa.releaseUpsert = resolve;
		});
	return null;
}

export async function fetch() {
	if (qa.holdRestore)
		await new Promise<void>((resolve) => {
			qa.releaseRestore = resolve;
		});
	return new Response(JSON.stringify(qa.serverProfiles), {
		status: qa.failRestore ? 503 : 200,
		headers: { "Content-Type": "application/json" },
	});
}

export function convertFileSrc(path: string) {
	return path;
}
