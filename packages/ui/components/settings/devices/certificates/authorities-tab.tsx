"use client";

import { useTranslation } from "@flow-like/locales";
import { Plus, RefreshCw, ShieldCheck, Trash2, Upload } from "lucide-react";
import { type MutableRefObject, useMemo, useState } from "react";
import { useAreaTime } from "../primitives/area-context";
import { Block } from "../primitives/block";
import { DvButton } from "../primitives/dv-button";
import { FreshnessStamp } from "../primitives/freshness-stamp";
import { InlineResult } from "../primitives/inline-result";
import { StateView } from "../primitives/state-view";
import { useDeviceWorkspace } from "../workspace/device-workspace-provider";
import { useInlineResults } from "../workspace/use-activity";
import { useDeviceAction } from "../workspace/use-device-action";
import { useHubSupport } from "../workspace/use-hub";
import { useLocalSummary } from "../workspace/use-keys";
import {
	type AuthorityAction,
	AuthorityCard,
	type AuthorityPlace,
	type AuthorityStorage,
	RemovedAuthorityCard,
} from "./authority-card";
import {
	type AuthorityNote,
	ChangePasswordSheet,
	RenewKeySheet,
	RestoreSheet,
	TestPasswordSheet,
} from "./authority-sheets";
import {
	type AuthorityView,
	type CertificateRow,
	signedBy,
} from "./certificates-model";
import {
	type CreateAuthorityResult,
	CreateAuthorityWizard,
} from "./create-authority-wizard";
import { PROSE, shortId } from "./parts";
import {
	type AuthorityStore,
	authorityResultKey,
	useSignedInName,
	useStillHere,
} from "./use-certificates";

export interface AuthoritiesTabProps {
	store: AuthorityStore;
	authorities: readonly AuthorityView[];
	/** Reported certificates, to show what each authority signed (from live reads). */
	rows: readonly CertificateRow[];
	creating: boolean;
	onCreating(open: boolean): void;
	restoring: boolean;
	onRestoring(open: boolean): void;
	/** Lets the screen ask before leaving a new authority whose backup isn't saved. */
	leaveGuard: MutableRefObject<(() => boolean) | null>;
}

const TOP = "top";

interface OpenSheet {
	kind: "renew" | "password" | "test";
	authorityId: string;
}

interface Removed {
	id: string;
	label: string;
}

function storageOf(
	platform: "desktop" | "web",
	persistence: string,
): AuthorityStorage {
	if (platform === "desktop") return "desktop";
	return persistence === "persisted" ? "web" : "web_unprotected";
}

/** What an authority's storage and its backup are bound to (account, hub, app profile) and what kind of storage holds it. */
function useAuthorityPlace(): AuthorityPlace {
	const { deps } = useDeviceWorkspace();
	const { host } = useHubSupport();
	const name = useSignedInName();
	const { persistence } = useLocalSummary();
	const { platform } = deps;
	const profile = deps.scope.profileId;
	return useMemo(
		() => ({
			home: { ...(name ? { name } : {}), host, profile },
			storage: storageOf(platform, persistence),
		}),
		[name, host, profile, platform, persistence],
	);
}

/** What the action layer filed for this authority that isn't simply "done": a write in progress or one that failed. */
function ActionResults({ authorityId }: Readonly<{ authorityId: string }>) {
	const results = useInlineResults(authorityResultKey(authorityId)).filter(
		(result) => result.state !== "done",
	);
	return (
		<>
			{results.map((result) => (
				<InlineResult
					key={result.id}
					tone={result.tone}
					onDismiss={result.dismiss}
				>
					{result.text}
				</InlineResult>
			))}
		</>
	);
}

/** SPEC §5.8 Organisation authorities: the certificate authorities whose signing keys are on this computer. */
export function AuthoritiesTab({
	store,
	authorities,
	rows,
	creating,
	onCreating,
	restoring,
	onRestoring,
	leaveGuard,
}: Readonly<AuthoritiesTabProps>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const actions = useDeviceAction();
	const stillHere = useStillHere();
	const place = useAuthorityPlace();
	const { home } = place;
	const [sheet, setSheet] = useState<OpenSheet | null>(null);
	const [notes, setNotes] = useState<Record<string, AuthorityNote | null>>({});
	const [restored, setRestored] = useState<ReadonlySet<string>>(new Set());
	const [removed, setRemoved] = useState<readonly Removed[]>([]);
	const say = (key: string, note: AuthorityNote | null) =>
		setNotes((current) => ({ ...current, [key]: note }));
	const current = authorities.find((view) => view.id === sheet?.authorityId);
	const gone = removed.filter(
		(entry) => !authorities.some((view) => view.id === entry.id),
	);

	const remove = async (view: AuthorityView) => {
		const here = stillHere();
		const usedBy = signedBy(rows, view.label);
		const outcome = await actions.run({
			action: "org_ca_manage",
			label: t(
				"certificates.remove.label",
				"Remove the signing key of {{label}}",
				{ label: view.label },
			),
			consequence: {
				what: t(
					"certificates.remove.what",
					"The encrypted signing key of {{label}} is deleted from this computer. You can't sign with it here until you restore it.",
					{ label: view.label },
				),
				who: usedBy.length
					? t(
							"certificates.remove.whoRenewals",
							"Nobody else: other computers and profiles keep theirs. Certificates that renew through it can't get a new renewal authority from here meanwhile.",
						)
					: t(
							"certificates.remove.who",
							"Nobody else: other computers and profiles keep theirs.",
						),
				stays: t(
					"certificates.remove.stays",
					"Certificates and renewal authorities it signed keep working. Your backup file still holds the root and the signing key.",
				),
				when: t("certificates.remove.when", "Immediately."),
				undo: {
					reversible: true,
					text: t(
						"certificates.remove.undo",
						"With your backup file and the authority password (Restore from backup…). Without the backup file you can't sign with it again.",
					),
				},
			},
			strength: "typed",
			confirm: {
				icon: Trash2,
				title: t(
					"certificates.remove.title",
					"Remove the signing key of {{label}} from this computer?",
					{ label: view.label },
				),
				sub: t(
					"certificates.remove.subtitle",
					"Authority {{id}} · stored for {{home}}",
					{
						id: shortId(view.id),
						home: [home.name, home.host].filter(Boolean).join(" · "),
					},
				),
				typed: view.label,
				tone: "danger",
			},
			resultKey: authorityResultKey(view.id),
			call: () => store.remove(view.id),
		});
		if (!here() || outcome.status !== "done") return;
		setRemoved((list) => [
			...list.filter((entry) => entry.id !== view.id),
			{ id: view.id, label: view.label },
		]);
		say(view.id, {
			tone: "good",
			text: t(
				"certificates.remove.done",
				"Signing key removed from this computer at {{time}}. Certificates it signed keep working.",
				{ time: time.clock(time.nowS) },
			),
		});
	};

	const onAction = (view: AuthorityView, action: AuthorityAction) => {
		if (action === "remove") void remove(view);
		else if (action === "restore") onRestoring(true);
		else setSheet({ kind: action, authorityId: view.id });
	};

	const closeWizard = (result?: CreateAuthorityResult) => {
		onCreating(false);
		if (result?.note) say(result.authorityId ?? TOP, result.note);
	};

	const restoreSheet = (
		<RestoreSheet
			key={restoring ? "open" : "closed"}
			store={store}
			home={home}
			open={restoring}
			onClose={() => onRestoring(false)}
			onDone={(result) => {
				if (result.restored)
					setRestored((ids) => new Set(ids).add(result.authorityId));
				say(result.authorityId, result.note);
			}}
		/>
	);

	if (creating)
		return (
			<CreateAuthorityWizard
				store={store}
				home={home}
				onClose={closeWizard}
				leaveGuard={leaveGuard}
			/>
		);

	const intro = (
		<p className={PROSE}>
			{t(
				"certificates.authorities.intro",
				"An organisation authority signs certificates for your own internal names, like mqtt.lab.internal, so devices can renew them without a public service. Its signing key stays on this computer, encrypted with the authority password. Its root key exists only in a backup file you download; Flow-Like never stores or uploads it.",
			)}
		</p>
	);
	const topNote = notes[TOP];
	const top = topNote ? (
		<InlineResult tone={topNote.tone} onDismiss={() => say(TOP, null)}>
			{topNote.text}
		</InlineResult>
	) : null;
	const title = t("certificates.authorities.title", "Organisation authorities");

	if (store.state !== "ready")
		return (
			<Block
				icon={ShieldCheck}
				title={title}
				stamp={
					<FreshnessStamp
						source="local"
						age={store.state === "failed" ? "error" : "notloaded"}
						text={
							store.state === "failed"
								? undefined
								: t("certificates.authorities.reading", "reading…")
						}
					/>
				}
			>
				{intro}
				{store.state === "failed" ? (
					<StateView
						kind="error"
						title={t(
							"certificates.authorities.failed",
							"Couldn't read the authorities saved on this computer",
						)}
						text={t(
							"certificates.authorities.failedText",
							"This app's storage didn't answer. Your backup files are not affected.",
						)}
						actions={
							<DvButton
								size="sm"
								icon={RefreshCw}
								onClick={() => void store.reload()}
							>
								{t("certificates.action.retry", "Try again")}
							</DvButton>
						}
					/>
				) : (
					<StateView kind="loading" rows={2} />
				)}
			</Block>
		);

	if (!authorities.length && !gone.length)
		return (
			<>
				{top}
				<Block
					id="certificates-authorities"
					icon={ShieldCheck}
					title={title}
					count={0}
					stamp={
						<FreshnessStamp
							source="local"
							age="current"
							text={t("certificates.authorities.none", "none on this computer")}
						/>
					}
					foot={
						<span>
							{t(
								"certificates.authorities.noneFoot",
								"Without an authority, certificates for internal names can't get a renewal authority from this computer. Certificates for public names can renew with Let's Encrypt instead.",
							)}
						</span>
					}
				>
					{intro}
					<StateView
						kind="empty"
						icon={ShieldCheck}
						title={t(
							"certificates.authorities.emptyTitle",
							"No organisation authority on this computer",
						)}
						text={t(
							"certificates.authorities.emptyText",
							"Create one for your internal names, or restore one you made on another computer or profile from its backup file. Authorities are kept per account, hub and app profile.",
						)}
						actions={
							<>
								<DvButton
									variant="primary"
									icon={Plus}
									onClick={() => onCreating(true)}
								>
									{t("certificates.authorities.create", "Create authority…")}
								</DvButton>
								<DvButton icon={Upload} onClick={() => onRestoring(true)}>
									{t("certificates.authority.restore", "Restore from backup…")}
								</DvButton>
							</>
						}
					/>
				</Block>
				{restoreSheet}
			</>
		);

	return (
		<>
			{top}
			<Block
				id="certificates-authorities"
				icon={ShieldCheck}
				title={title}
				count={authorities.length}
				stamp={
					<FreshnessStamp
						source="local"
						age="current"
						text={t(
							"certificates.authorities.stored",
							"stored on this computer",
						)}
					/>
				}
				tools={
					<>
						<DvButton size="sm" icon={Plus} onClick={() => onCreating(true)}>
							{t("certificates.authorities.create", "Create authority…")}
						</DvButton>
						<DvButton
							size="sm"
							variant="ghost"
							icon={Upload}
							onClick={() => onRestoring(true)}
						>
							{t("certificates.authority.restore", "Restore from backup…")}
						</DvButton>
					</>
				}
			>
				{intro}
			</Block>
			{authorities.map((view) => (
				<AuthorityCard
					key={view.id}
					view={view}
					restored={restored.has(view.id)}
					signed={signedBy(rows, view.label)}
					place={place}
					note={notes[view.id]}
					onNote={(note) => say(view.id, note)}
					onAction={(action) => onAction(view, action)}
					removing={actions.pending(authorityResultKey(view.id))}
					results={<ActionResults authorityId={view.id} />}
				/>
			))}
			{gone.map((entry) => (
				<RemovedAuthorityCard
					key={entry.id}
					label={entry.label}
					note={notes[entry.id]}
					onDismissNote={() => say(entry.id, null)}
					onRestore={() => onRestoring(true)}
				/>
			))}
			{current && sheet?.kind === "renew" ? (
				<RenewKeySheet
					view={current}
					store={store}
					open
					onClose={() => setSheet(null)}
					onDone={(note) => say(current.id, note)}
				/>
			) : null}
			{current && sheet?.kind === "password" ? (
				<ChangePasswordSheet
					view={current}
					store={store}
					open
					onClose={() => setSheet(null)}
					onDone={(note) => say(current.id, note)}
				/>
			) : null}
			{current && sheet?.kind === "test" ? (
				<TestPasswordSheet
					view={current}
					store={store}
					open
					onClose={() => setSheet(null)}
				/>
			) : null}
			{restoreSheet}
		</>
	);
}
