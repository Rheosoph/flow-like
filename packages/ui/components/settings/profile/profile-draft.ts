import { apiErrorMessage } from "../../../lib/api-error";
import type { ISettingsProfile } from "../../../types";

export type ProfileSaveStatus = "saved" | "pending" | "saving" | "error";
export type ProfileDraftSnapshot = {
	profile: ISettingsProfile | null;
	status: ProfileSaveStatus;
	error: string | null;
};

type Entry = {
	profile: ISettingsProfile;
	revision: number;
	savedRevision: number;
	saving: boolean;
	error: string | null;
	timer?: ReturnType<typeof setTimeout>;
};

export function validateProfile(profile: ISettingsProfile): string | null {
	if (!profile.hub_profile.name.trim()) return "Enter a profile name.";
	if (profile.hub_profile.name.length > 100)
		return "Use 100 characters or fewer for the profile name.";
	const context = profile.execution_settings.max_context_size;
	if (!Number.isInteger(context) || context < 0 || context > 4294967295) {
		return "Enter a whole number between 0 and 4,294,967,295 for the context size.";
	}
	return null;
}

/** Keeps edits separate from query refreshes and serializes writes across profiles. */
export class ProfileDraftController {
	private entries = new Map<string, Entry>();
	private listeners = new Set<() => void>();
	private ready = new Set<string>();
	private activeId: string | null = null;
	private running: Promise<void> | null = null;
	private snapshot: ProfileDraftSnapshot = {
		profile: null,
		status: "saved",
		error: null,
	};

	constructor(
		private save: (profile: ISettingsProfile) => Promise<void>,
		private delay = 500,
	) {}

	setSaveHandler(save: (profile: ISettingsProfile) => Promise<void>) {
		this.save = save;
	}

	getSnapshot = () => this.snapshot;
	subscribe = (listener: () => void) => {
		this.listeners.add(listener);
		return () => {
			this.listeners.delete(listener);
		};
	};

	private publish() {
		const entry = this.activeId ? this.entries.get(this.activeId) : undefined;
		this.snapshot = {
			profile: entry?.profile ?? null,
			status: entry?.error
				? "error"
				: entry?.saving
					? "saving"
					: entry && entry.revision !== entry.savedRevision
						? "pending"
						: "saved",
			error: entry?.error ?? null,
		};
		for (const listener of this.listeners) listener();
	}

	setSource(profile: ISettingsProfile) {
		const id = profile.hub_profile.id;
		if (!id) return;
		this.activeId = id;
		const existing = this.entries.get(id);
		if (
			!existing ||
			(!existing.saving && existing.revision === existing.savedRevision)
		) {
			this.entries.set(id, {
				profile,
				revision: 0,
				savedRevision: 0,
				saving: false,
				error: null,
			});
		}
		this.publish();
	}

	update = (updates: Partial<ISettingsProfile>) => {
		const entry = this.activeId ? this.entries.get(this.activeId) : undefined;
		if (!entry || !this.activeId) return;
		const now = new Date().toISOString();
		entry.profile = {
			...entry.profile,
			...updates,
			hub_profile: {
				...entry.profile.hub_profile,
				...updates.hub_profile,
				updated: now,
			},
			updated: now,
		};
		entry.revision += 1;
		entry.error = validateProfile(entry.profile);
		clearTimeout(entry.timer);
		this.ready.delete(this.activeId);
		if (!entry.error) {
			const id = this.activeId;
			entry.timer = setTimeout(() => {
				this.ready.add(id);
				void this.drain();
			}, this.delay);
		}
		this.publish();
	};

	private drain(): Promise<void> {
		if (this.running) return this.running;
		this.running = (async () => {
			while (this.ready.size) {
				const id = this.ready.values().next().value as string;
				this.ready.delete(id);
				const entry = this.entries.get(id);
				if (
					!entry ||
					entry.revision === entry.savedRevision ||
					validateProfile(entry.profile)
				)
					continue;
				const revision = entry.revision;
				const profile = entry.profile;
				entry.saving = true;
				entry.error = null;
				this.publish();
				try {
					await this.save(profile);
					entry.savedRevision = revision;
				} catch (error) {
					entry.error = apiErrorMessage(
						error,
						error instanceof Error ? error.message : String(error),
					);
					if (entry.revision === revision) this.ready.delete(id);
				} finally {
					entry.saving = false;
					this.publish();
				}
			}
		})().finally(() => {
			this.running = null;
		});
		return this.running;
	}

	retry = () => {
		void this.flush().catch(() => {});
	};

	async flush(id = this.activeId): Promise<void> {
		if (!id) return;
		const entry = this.entries.get(id);
		if (!entry || entry.revision === entry.savedRevision) return;
		clearTimeout(entry.timer);
		const validation = validateProfile(entry.profile);
		if (validation) {
			entry.error = validation;
			this.publish();
			throw new Error(validation);
		}
		entry.error = null;
		this.ready.add(id);
		await this.drain();
		if (entry.error) throw new Error(entry.error);
	}

	flushAll = async () => {
		await Promise.all(
			[...this.entries.keys()].map((id) => this.flush(id).catch(() => {})),
		);
	};

	hasUnsaved = () =>
		[...this.entries.values()].some(
			(entry) => entry.revision !== entry.savedRevision,
		);

	forget(id: string) {
		clearTimeout(this.entries.get(id)?.timer);
		this.ready.delete(id);
		this.entries.delete(id);
		if (this.activeId === id) this.activeId = null;
		this.publish();
	}
}

// Failed drafts survive a route change within this tab, separated by account and hub.
const sessions = new Map<string, ProfileDraftController>();

export function profileDraftSession(
	scope: string,
	save: (profile: ISettingsProfile) => Promise<void>,
) {
	let controller = sessions.get(scope);
	if (!controller) {
		controller = new ProfileDraftController(save);
		sessions.set(scope, controller);
	}
	controller.setSaveHandler(save);
	return controller;
}

export function releaseProfileDraftSession(
	scope: string,
	controller: ProfileDraftController,
) {
	if (!controller.hasUnsaved() && sessions.get(scope) === controller)
		sessions.delete(scope);
}

export function workspaceProfileDraftScope(
	platform: "desktop" | "web",
	accountId?: string,
	hub?: string,
) {
	return JSON.stringify([platform, hub ?? "default-hub", accountId ?? "local"]);
}

export async function flushCachedProfileDraft(
	scope: string,
	profileId: string,
) {
	await sessions.get(scope)?.flush(profileId);
}

export function forgetCachedProfileDraft(scope: string, profileId: string) {
	sessions.get(scope)?.forget(profileId);
}
