"use client";

import { useTranslation } from "@flow-like/locales";
import { History, Users } from "lucide-react";
import { useId, useMemo, useState } from "react";
import type { ArchiveRecipient } from "../../../../lib/device-management/types";
import {
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "../../../ui/select";
import { enumLabel } from "../copy/enum-labels";
import { type DevicesT, useAreaTime } from "../primitives/area-context";
import { ConsequencePreview } from "../primitives/consequence-preview";
import { DvButton } from "../primitives/dv-button";
import { DvSheet } from "../primitives/dv-sheet";
import {
	CheckField,
	DropZone,
	DvTextarea,
	Field,
	SecretInput,
} from "../primitives/form-fields";
import { InlineResult } from "../primitives/inline-result";
import { PersonChip } from "../primitives/person-chip";
import {
	inlineResultsOf,
	useDeviceWorkspace,
	useKeySession,
} from "../workspace";
import { PLAIN_PARAGRAPHS, dayLabel } from "./observe-data";
import {
	type ReaderRequestProblem,
	parseReaderRequest,
} from "./reader-request";
import {
	DEVICE_SCOPE,
	type HistoryKind,
	type HistoryStream,
	MAX_READERS,
	MAX_RECORDING_S,
	type SaveFailure,
	type SaveReadersOutcome,
	historyResultKey,
	readerGrantExpiry,
	useSaveReaders,
} from "./use-history";
import { type ObserveTarget, usePeople } from "./use-observe-target";

export type ReadersMode = "setup" | "change" | "resume";

/** Which readers lists the sheet replaces: one scope, one kind or both. */
export interface ReadersEdit {
	scope: string;
	kinds: readonly HistoryKind[];
	projectId: string | null;
	mode: ReadersMode;
	/** The lists as last read, for the readers they name. */
	streams: readonly HistoryStream[];
}

const DAY_S = 86_400;
const DURATIONS = [1, 7, 31] as const;

interface Candidate {
	recipient: ArchiveRecipient;
	/** Until when this person may read every chosen kind; undefined = no permission. */
	until: number | undefined;
	added: boolean;
}

function kindsLabel(t: DevicesT, kinds: readonly HistoryKind[]): string {
	return kinds.length > 1
		? t("devices:observe.history.bothKinds", "Logs and metrics")
		: enumLabel(t, "archiveKind", kinds[0] ?? "logs");
}

function problemText(t: DevicesT, problem: ReaderRequestProblem): string {
	const texts: Record<ReaderRequestProblem, string> = {
		invalid: t(
			"devices:observe.history.sheet.requestInvalid",
			"This isn't a reader request. Ask the person to copy it again from Activity & logs › History access.",
		),
		other_device: t(
			"devices:observe.history.sheet.requestOtherDevice",
			"This reader request was made for another device.",
		),
		too_many: t(
			"devices:observe.history.sheet.requestTooMany",
			"A readers list holds at most 32 readers.",
		),
	};
	return texts[problem];
}

function failureText(
	t: DevicesT,
	reason: SaveFailure,
	device: string,
	detail: string | undefined,
): string {
	const texts: Record<SaveFailure, string> = {
		rules_not_applied: t(
			"devices:observe.history.sheet.failRulesNotApplied",
			"{{device}} hasn't applied the newest access rules yet. Try again once it has, usually within 5 minutes while it's online.",
			{ device },
		),
		rules_unverified: t(
			"devices:observe.history.sheet.failRulesUnverified",
			"The access rules of {{device}} couldn't be verified, so nothing was signed.",
			{ device },
		),
		reader_without_access: t(
			"devices:observe.history.sheet.failReaderWithoutAccess",
			"A chosen reader has no permission to read this any more. Remove them or change their access first.",
		),
		too_many_readers: t(
			"devices:observe.history.sheet.failTooMany",
			"This list is too long for one message to the device. Choose fewer readers.",
		),
		wrong_password: t(
			"devices:observe.history.sheet.failPassword",
			"The device password isn't correct.",
		),
		rejected: detail
			? t(
					"devices:observe.history.sheet.failRejectedReason",
					"{{device}} refused the list: “{{reason}}”",
					{ device, reason: detail },
				)
			: t(
					"devices:observe.history.sheet.failRejected",
					"{{device}} refused the list.",
					{
						device,
					},
				),
		unconfirmed: t(
			"devices:observe.history.sheet.failUnconfirmed",
			"{{device}} didn't confirm the new readers list, so it may or may not be in place. Close this and check the row once it has been read again.",
			{ device },
		),
		other: t(
			"devices:observe.history.sheet.failOther",
			"The readers list wasn't saved. Nothing changed on {{device}}.",
			{ device },
		),
	};
	return texts[reason];
}

/** A run stopped before anything was sent (the action can't run right now) still gets an answer, never silence. */
function unsavedText(
	t: DevicesT,
	outcome: Exclude<
		SaveReadersOutcome,
		{ status: "done" | "password_required" }
	>,
	device: string,
): string {
	if (outcome.status === "failed")
		return failureText(t, outcome.reason, device, outcome.detail);
	return failureText(t, "other", device, undefined);
}

export function HistoryReadersSheet({
	target,
	edit,
	onClose,
}: Readonly<{
	target: ObserveTarget;
	edit: ReadersEdit;
	onClose(): void;
}>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const id = useId();
	const workspace = useDeviceWorkspace();
	const session = useKeySession(target.deviceId);
	const save = useSaveReaders(target);
	const { scope, kinds, projectId, mode } = edit;
	const now = Math.floor(time.nowS);

	const current = useMemo(() => {
		const byId = new Map<string, ArchiveRecipient>();
		for (const stream of edit.streams)
			for (const row of stream.roster?.recipients ?? [])
				if (row.user_id !== target.me) byId.set(row.recipient_id, row);
		return [...byId.values()];
	}, [edit.streams, target.me]);
	const [added, setAdded] = useState<ArchiveRecipient[]>([]);
	const [dropped, setDropped] = useState<ReadonlySet<string>>(new Set());
	const [pasted, setPasted] = useState("");
	const [problem, setProblem] = useState<ReaderRequestProblem | null>(null);
	const [days, setDays] = useState<string>(String(DURATIONS.at(-1)));
	const [password, setPassword] = useState("");
	const [askPassword, setAskPassword] = useState(false);
	const [busy, setBusy] = useState(false);
	const [failure, setFailure] = useState<string | null>(null);

	const untilOf = (userId: string) => {
		const expiries = kinds.map((kind) =>
			readerGrantExpiry(target.policy, userId, { scope, kind, projectId }, now),
		);
		return expiries.some((value) => value === undefined)
			? undefined
			: Math.min(...(expiries as number[]));
	};
	const candidates: Candidate[] = [
		...current.map((recipient) => ({
			recipient,
			until: untilOf(recipient.user_id),
			added: false,
		})),
		...added
			.filter(
				(row) =>
					!current.some((known) => known.recipient_id === row.recipient_id),
			)
			.map((recipient) => ({
				recipient,
				until: untilOf(recipient.user_id),
				added: true,
			})),
	];
	const chosen = candidates.filter(
		(row) =>
			row.until !== undefined && !dropped.has(row.recipient.recipient_id),
	);
	const people = usePeople(candidates.map((row) => row.recipient.user_id));
	const nameOf = (userId: string) =>
		people(userId) ??
		t("devices:observe.history.someone", "Someone with shared access");

	const limit = Math.min(
		target.policy?.expires_at ?? now + MAX_RECORDING_S,
		...chosen.map((row) => row.until as number),
	);
	const options = DURATIONS.flatMap((count) => {
		const until = Math.min(now + count * DAY_S, limit);
		return until > now ? [{ count, until, capped: until === limit }] : [];
	}).filter(
		(option, index, all) =>
			all.findIndex((other) => other.until === option.until) === index,
	);
	const picked =
		options.find((option) => String(option.count) === days) ?? options.at(-1);
	const needsPassword = askPassword || !session.canSign;
	const kindText = kindsLabel(t, kinds).toLowerCase();
	const scopeText =
		scope === DEVICE_SCOPE
			? t("devices:observe.history.wholeDevice", "Whole device")
			: scope;

	const importText = (text: string) => {
		const parsed = parseReaderRequest(text, target.deviceId);
		if (!parsed.ok) {
			setProblem(parsed.problem);
			return false;
		}
		setProblem(null);
		setAdded((known) => {
			const next = [...known];
			for (const row of parsed.recipients)
				if (
					row.user_id !== target.me &&
					!next.some((other) => other.recipient_id === row.recipient_id)
				)
					next.push(row);
			return next.slice(0, MAX_READERS);
		});
		return true;
	};

	const titles: Record<ReadersMode, string> = {
		setup:
			scope === DEVICE_SCOPE
				? t("devices:observe.history.sheet.setupDevice", "Set up history")
				: t(
						"devices:observe.history.sheet.setupService",
						"History for {{service}}",
						{
							service: scope,
						},
					),
		change: t("devices:observe.history.sheet.change", "Change readers"),
		resume: t("devices:observe.history.sheet.resume", "Resume recording"),
	};
	const confirms: Record<ReadersMode, string> = {
		setup: t("devices:observe.history.sheet.start", "Start recording"),
		change: t(
			"devices:observe.history.sheet.replaceReaders",
			"Replace readers",
		),
		resume: t("devices:observe.history.sheet.resumeAction", "Resume recording"),
	};

	const submit = async () => {
		if (!picked || busy) return;
		// The typed password leaves the field with the click, whatever comes of the signature.
		const secret = needsPassword ? password : undefined;
		setPassword("");
		setBusy(true);
		setFailure(null);
		try {
			for (const kind of kinds) {
				const outcome = await save({
					stream: { scope, kind, projectId },
					others: chosen.map((row) => row.recipient),
					durationS: picked.until - now,
					label: t(
						"devices:observe.history.sheet.label",
						"{{action}}: {{kind}} of {{scope}}",
						{
							action: confirms[mode],
							kind: enumLabel(t, "archiveKind", kind).toLowerCase(),
							scope: scope === DEVICE_SCOPE ? target.name : scope,
						},
					),
					...(secret === undefined ? {} : { password: secret }),
				});
				if (outcome.status === "done") continue;
				if (outcome.status === "password_required") {
					inlineResultsOf(workspace).clear(historyResultKey(target.deviceId));
					setAskPassword(true);
					return;
				}
				setFailure(unsavedText(t, outcome, target.name));
				return;
			}
			onClose();
		} finally {
			setBusy(false);
		}
	};

	return (
		<DvSheet
			open
			bodyClassName={PLAIN_PARAGRAPHS}
			onOpenChange={(open) => {
				if (!open && !busy) onClose();
			}}
			icon={mode === "change" ? Users : History}
			title={titles[mode]}
			sub={t(
				"devices:observe.history.sheet.subtitle",
				"Retained {{kind}} · {{scope}} · {{device}}",
				{ kind: kindText, scope: scopeText, device: target.name },
			)}
			foot={
				<>
					<DvButton onClick={onClose} disabled={busy}>
						{t("devices:observe.history.sheet.cancel", "Cancel")}
					</DvButton>
					<DvButton
						variant="primary"
						busy={busy}
						disabled={!picked || (needsPassword && !password)}
						onClick={() => void submit()}
					>
						{confirms[mode]}
					</DvButton>
				</>
			}
		>
			<p className="text-ui text-muted-foreground">
				{t(
					"devices:observe.history.sheet.intro",
					"Readers can decrypt the retained records; nobody else can, the hub included. A reader needs permission to read {{kind}} here and sends you a reader request from their own computer.",
					{ kind: kindText },
				)}
			</p>
			<ul className="flex flex-col gap-2">
				<li>
					<CheckField
						id={`${id}-me`}
						checked
						disabled
						onCheckedChange={() => {}}
					>
						<span className="inline-flex flex-wrap items-center gap-1.5">
							<PersonChip name={t("devices:observe.history.you", "You")} you />
							<span className="text-muted-foreground">
								{t(
									"devices:observe.history.sheet.owner",
									"· owner, always a reader",
								)}
							</span>
						</span>
					</CheckField>
				</li>
				{candidates.map((row, index) => {
					const key = row.recipient.recipient_id;
					const allowed = row.until !== undefined;
					return (
						<li key={key}>
							<CheckField
								id={`${id}-reader-${index}`}
								checked={allowed && !dropped.has(key)}
								disabled={!allowed}
								onCheckedChange={(checked) =>
									setDropped((known) => {
										const next = new Set(known);
										if (checked) next.delete(key);
										else next.add(key);
										return next;
									})
								}
							>
								<span className="inline-flex flex-wrap items-center gap-1.5">
									<PersonChip name={nameOf(row.recipient.user_id)} />
									<span className="text-muted-foreground">
										{allowed
											? t(
													"devices:observe.history.sheet.readerUntil",
													"· may read {{kind}} here until {{date}}",
													{
														kind: kindText,
														date: dayLabel(time, row.until as number),
													},
												)
											: t(
													"devices:observe.history.sheet.readerNoAccess",
													"· has no permission to read {{kind}} here: can't be a reader",
													{ kind: kindText },
												)}
									</span>
								</span>
							</CheckField>
						</li>
					);
				})}
			</ul>
			<DropZone
				id={`${id}-files`}
				accept="application/json,.json"
				multiple
				title={t("devices:observe.history.sheet.drop", "Add reader requests")}
				hint={t(
					"devices:observe.history.sheet.dropHint",
					"Drop the .json files people sent you, or paste one below.",
				)}
				onFiles={(files) => {
					void Promise.all(files.map((file) => file.text())).then((texts) => {
						for (const text of texts) if (!importText(text)) break;
					});
				}}
			/>
			<Field
				id={`${id}-paste`}
				label={t(
					"devices:observe.history.sheet.paste",
					"Paste a reader request",
				)}
				error={problem ? problemText(t, problem) : undefined}
			>
				<DvTextarea
					rows={3}
					value={pasted}
					spellCheck={false}
					className="font-mono text-xs"
					onChange={(event) => setPasted(event.target.value)}
				/>
			</Field>
			<div>
				<DvButton
					size="sm"
					disabled={!pasted.trim()}
					onClick={() => {
						if (importText(pasted)) setPasted("");
					}}
				>
					{t("devices:observe.history.sheet.addPasted", "Add reader")}
				</DvButton>
			</div>
			{options.length ? (
				<Field
					id={`${id}-until`}
					label={t(
						"devices:observe.history.sheet.until",
						"Keep recording until",
					)}
					hint={t(
						"devices:observe.history.sheet.untilHint",
						"Recording can't outlast the device's access rules or a reader's access. Renew the list before it expires to keep recording.",
					)}
				>
					<Select value={String(picked?.count ?? "")} onValueChange={setDays}>
						<SelectTrigger className="h-8.5 rounded-lg border-border bg-card text-ui shadow-none">
							<SelectValue />
						</SelectTrigger>
						<SelectContent className="border-border-strong bg-popover shadow-none">
							{options.map((option) => (
								<SelectItem
									key={option.count}
									value={String(option.count)}
									className="focus:bg-row-hover focus:text-foreground"
								>
									{option.capped
										? t(
												"devices:observe.history.sheet.untilCapped",
												"{{date}} · the longest your access rules allow",
												{ date: time.at(option.until) },
											)
										: t("devices:observe.history.sheet.untilDays", {
												count: option.count,
												date: time.at(option.until),
												defaultValue_one: "{{date}} · {{count, number}} day",
												defaultValue_other: "{{date}} · {{count, number}} days",
											})}
								</SelectItem>
							))}
						</SelectContent>
					</Select>
				</Field>
			) : (
				<InlineResult tone="warning">
					{t(
						"devices:observe.history.sheet.rulesExpired",
						"The access rules of {{device}} have expired, so a readers list can't be signed. Renew them under Access first.",
						{ device: target.name },
					)}
				</InlineResult>
			)}
			{needsPassword ? (
				<Field
					id={`${id}-password`}
					label={t("devices:observe.history.sheet.password", "Device password")}
					hint={t(
						"devices:observe.history.sheet.passwordHint",
						"Signing a readers list is an owner action, so it needs the password of {{device}}.",
						{ device: target.name },
					)}
				>
					<SecretInput
						value={password}
						onValueChange={setPassword}
						autoComplete="current-password"
					/>
				</Field>
			) : null}
			<ConsequencePreview
				rows={
					mode === "setup"
						? {
								what: t(
									"devices:observe.history.sheet.setupWhat",
									"{{device}} seals these {{kind}} every 30 s, encrypted for the readers, and stores them on the hub.",
									{ device: target.name, kind: kindText },
								),
								who: t(
									"devices:observe.history.sheet.setupWho",
									"Readers can decrypt records from now on. Nobody else can, the hub included.",
								),
								when: t(
									"devices:observe.history.sheet.setupWhen",
									"From the next sealed chunk, within 30 s.",
								),
								undo: {
									reversible: true,
									text: t(
										"devices:observe.history.sheet.setupUndo",
										"Change the readers at any time; stored records stay until they expire.",
									),
								},
							}
						: {
								what: t(
									"devices:observe.history.sheet.changeWhat",
									"Replaces the readers list. Added people can read records from now on.",
								),
								who: t(
									"devices:observe.history.sheet.changeWho",
									"Removed people stop receiving new records; what they already read stays readable to them.",
								),
								when: t(
									"devices:observe.history.sheet.changeWhen",
									"Applies to future records only, from the next sealed chunk (every 30 s).",
								),
								undo: {
									reversible: true,
									text: t(
										"devices:observe.history.sheet.changeUndo",
										"Change the list again the same way.",
									),
								},
							}
				}
			/>
			{failure ? (
				<InlineResult tone="critical" onDismiss={() => setFailure(null)}>
					{failure}
				</InlineResult>
			) : null}
		</DvSheet>
	);
}
