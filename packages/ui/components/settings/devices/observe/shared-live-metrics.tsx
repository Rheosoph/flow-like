"use client";

import { useTranslation } from "@flow-like/locales";
import {
	Check,
	CircleCheck,
	Copy,
	Download,
	RadioTower,
	RefreshCw,
	UserPlus,
} from "lucide-react";
import { type ReactNode, useEffect, useId, useRef, useState } from "react";
import { classifyDeviceError } from "../../../../lib/device-management/workspace/errors";
import type { GroupMetricsSample } from "../../../../lib/device-management/workspace/streams";
import { enumLabel } from "../copy/enum-labels";
import { errorCopy } from "../copy/error-copy";
import { type DevicesT, useAreaTime } from "../primitives/area-context";
import {
	ConsequencePreview,
	type ConsequenceRows,
} from "../primitives/consequence-preview";
import { DvButton } from "../primitives/dv-button";
import { DvSheet } from "../primitives/dv-sheet";
import {
	DropZone,
	DvTextarea,
	Field,
	SecretInput,
	SwitchField,
} from "../primitives/form-fields";
import { FreshnessStamp } from "../primitives/freshness-stamp";
import { GatedAction } from "../primitives/gate-notice";
import { InlineResult } from "../primitives/inline-result";
import { KeyValueList, KvRow } from "../primitives/key-value-list";
import { PersonChip } from "../primitives/person-chip";
import { StateView } from "../primitives/state-view";
import { useCopy } from "../primitives/use-copy";
import { stampOf } from "../shell/attention-popover";
import {
	inlineResultsOf,
	useDeviceWorkspace,
	useGate,
	useInlineResults,
	useKeySession,
	useLiveStream,
} from "../workspace";
import { LiveDataState, livePhase } from "./live-state";
import { PLAIN_PARAGRAPHS, downloadText } from "./observe-data";
import { DeviceResourceCells } from "./resource-cells";
import { DEVICE_SCOPE, type SaveFailure, policyAppliedOf } from "./use-history";
import {
	type ObserveTarget,
	type PersonNames,
	gateLine,
	usePeople,
} from "./use-observe-target";
import {
	type MetricsReaderRequest,
	type MetricsRequestProblem,
	type SharedReader,
	type SharedRoster,
	createMetricsRequest,
	parseMetricsRequest,
	sharedResultKey,
	useSaveMetricReaders,
	useSharedRoster,
} from "./use-shared-metrics";

const MLS_STATES = [
	"key_package",
	"joined",
	"application",
	"epoch_changed",
	"removed",
	"duplicate",
] as const;
type MlsState = (typeof MLS_STATES)[number];

const shortId = (id: string) => id.slice(0, 8);

function saveFailure(
	t: DevicesT,
	reason: SaveFailure,
	device: string,
	detail?: string,
): string {
	if (reason === "rules_not_applied")
		return t(
			"devices:observe.shared.failRulesNotApplied",
			"{{device}} hasn't applied the newest access rules yet. Try again once it has.",
			{ device },
		);
	if (reason === "wrong_password")
		return t(
			"devices:observe.shared.failPassword",
			"The device password isn't correct.",
		);
	if (reason === "rejected" && detail)
		return t(
			"devices:observe.shared.failRejected",
			"{{device}} refused the readers list: “{{reason}}”",
			{ device, reason: detail },
		);
	return t(
		"devices:observe.shared.failOther",
		"The readers list wasn't saved. Nothing changed on {{device}}.",
		{ device },
	);
}

function ReadersValue({
	shared,
	people,
}: Readonly<{ shared: SharedRoster; people: PersonNames }>) {
	const { t } = useTranslation("devices");
	const { readers } = shared;
	if (!readers.length)
		return (
			<span className="text-muted-foreground">
				{t("devices:observe.shared.noReaders", "Nobody yet")}
			</span>
		);
	const tick = (reader: SharedReader) =>
		reader.confirmed ? (
			<CircleCheck
				aria-label={t(
					"devices:observe.shared.confirmedOne",
					"confirmed the latest sample",
				)}
				className="size-3.5 text-good"
			/>
		) : null;
	if (shared.named)
		return (
			<span className="inline-flex flex-wrap items-center gap-x-3 gap-y-1">
				{readers.map((reader) => (
					<span
						key={reader.endpointId}
						className="inline-flex items-center gap-1"
					>
						<PersonChip
							you={reader.you}
							name={
								(reader.userId ? people(reader.userId) : undefined) ??
								(reader.you
									? t("devices:observe.shared.you", "You")
									: t(
											"devices:observe.shared.someone",
											"Someone with shared access",
										))
							}
						/>
						{tick(reader)}
					</span>
				))}
			</span>
		);
	return (
		<span className="inline-flex flex-wrap items-center gap-x-2 gap-y-1">
			<span>
				{t("devices:observe.shared.endpoints", {
					count: readers.length,
					defaultValue_one: "{{count, number}} reader",
					defaultValue_other: "{{count, number}} readers",
				})}
			</span>
			{readers.map((reader) => (
				<span
					key={reader.endpointId}
					className="inline-flex items-center gap-1"
				>
					<span aria-hidden>·</span>
					<span className="font-mono">{shortId(reader.endpointId)}</span>
					{reader.you ? (
						<span className="text-muted-foreground">
							{t("devices:observe.shared.youMark", "(you)")}
						</span>
					) : null}
					{tick(reader)}
				</span>
			))}
		</span>
	);
}

function deliveryText(t: DevicesT, readers: readonly SharedReader[]): string {
	if (!readers.length)
		return t("devices:observe.shared.deliveryNone", "No reader to deliver to");
	if (readers.some((reader) => reader.confirmed === undefined))
		return t(
			"devices:observe.shared.deliveryUnknown",
			"Not reported by this version of the device agent",
		);
	const confirmed = readers.filter((reader) => reader.confirmed).length;
	return confirmed === readers.length
		? t("devices:observe.shared.deliveryAll", {
				count: readers.length,
				defaultValue_one: "The reader confirmed the latest sample",
				defaultValue_other:
					"All {{count, number}} readers confirmed the latest sample",
			})
		: t(
				"devices:observe.shared.deliverySome",
				"{{confirmed, number}} of {{count, number}} readers confirmed the latest sample",
				{ confirmed, count: readers.length },
			);
}

function problemText(t: DevicesT, problem: MetricsRequestProblem): string {
	const texts: Record<MetricsRequestProblem, string> = {
		invalid: t(
			"devices:observe.shared.requestInvalid",
			"This isn't a reader request for shared metrics. Ask the person to create it again under Metrics › Shared live metrics.",
		),
		other_device: t(
			"devices:observe.shared.requestOtherDevice",
			"This reader request was made for another device.",
		),
		too_many: t(
			"devices:observe.shared.requestTooMany",
			"Add at most 30 readers at once.",
		),
	};
	return texts[problem];
}

function ReadersSheet({
	target,
	scope,
	renew,
	onClose,
}: Readonly<{
	target: ObserveTarget;
	scope: string;
	/** Renew the list as it is (no new readers): shown when the signature needs the password. */
	renew: boolean;
	onClose(): void;
}>) {
	const { t } = useTranslation("devices");
	const id = useId();
	const workspace = useDeviceWorkspace();
	const session = useKeySession(target.deviceId);
	const save = useSaveMetricReaders(target, scope);
	const [requests, setRequests] = useState<MetricsReaderRequest[]>([]);
	const [pasted, setPasted] = useState("");
	const [problem, setProblem] = useState<MetricsRequestProblem | null>(null);
	const [password, setPassword] = useState("");
	const [askPassword, setAskPassword] = useState(false);
	const [busy, setBusy] = useState(false);
	const [failure, setFailure] = useState<string | null>(null);
	const needsPassword = askPassword || !session.canSign;

	const importText = (text: string) => {
		const parsed = parseMetricsRequest(text, target.deviceId, scope);
		if (!parsed.ok) {
			setProblem(parsed.problem);
			return false;
		}
		setProblem(null);
		setRequests((known) => {
			const next = [...known];
			for (const row of parsed.requests)
				if (
					!next.some(
						(other) => other.member.endpoint_id === row.member.endpoint_id,
					)
				)
					next.push(row);
			return next;
		});
		return true;
	};

	const submit = async () => {
		setBusy(true);
		setFailure(null);
		try {
			const outcome = await save({
				add: requests,
				label: renew
					? t(
							"devices:observe.shared.renewLabel",
							"Renew shared metric readers",
						)
					: t("devices:observe.shared.addLabel", "Add shared metric readers"),
				...(needsPassword ? { password } : {}),
			});
			if (outcome.status === "done") {
				setPassword("");
				onClose();
			} else if (outcome.status === "password_required") {
				inlineResultsOf(workspace).clear(
					sharedResultKey(target.deviceId, scope),
				);
				setAskPassword(true);
			} else if (outcome.status === "failed")
				setFailure(saveFailure(t, outcome.reason, target.name, outcome.detail));
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
			icon={RadioTower}
			title={
				renew
					? t("devices:observe.shared.sheetRenew", "Renew the readers list")
					: t("devices:observe.shared.sheetAdd", "Add readers")
			}
			sub={t(
				"devices:observe.shared.sheetSub",
				"Shared live metrics · whole device · {{device}}",
				{ device: target.name },
			)}
			foot={
				<>
					<DvButton onClick={onClose} disabled={busy}>
						{t("devices:observe.shared.cancel", "Cancel")}
					</DvButton>
					<DvButton
						variant="primary"
						busy={busy}
						disabled={
							(!renew && !requests.length) || (needsPassword && !password)
						}
						onClick={() => void submit()}
					>
						{renew
							? t("devices:observe.shared.renewAction", "Renew list")
							: t("devices:observe.shared.addAction", "Add readers")}
					</DvButton>
				</>
			}
		>
			{renew ? null : (
				<>
					<p className="text-ui text-muted-foreground">
						{t(
							"devices:observe.shared.sheetIntro",
							"Each reader creates a request on their own computer under Metrics › Shared live metrics and sends it to you. It holds a public key, no secret.",
						)}
					</p>
					<DropZone
						id={`${id}-files`}
						accept="application/json,.json"
						multiple
						title={t("devices:observe.shared.drop", "Add reader requests")}
						hint={t(
							"devices:observe.shared.dropHint",
							"Drop the .json files people sent you, or paste one below.",
						)}
						files={
							requests.length ? (
								<span className="inline-flex flex-wrap gap-1.5">
									{requests.map((row) => (
										<span
											key={row.member.endpoint_id}
											className="rounded-sm border border-hairline bg-card px-1.5 font-mono text-xs"
										>
											{shortId(row.member.endpoint_id)}
										</span>
									))}
								</span>
							) : undefined
						}
						onFiles={(files) => {
							void Promise.all(files.map((file) => file.text())).then(
								(texts) => {
									for (const text of texts) if (!importText(text)) break;
								},
							);
						}}
					/>
					<Field
						id={`${id}-paste`}
						label={t("devices:observe.shared.paste", "Paste a reader request")}
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
							{t("devices:observe.shared.addPasted", "Add reader")}
						</DvButton>
					</div>
				</>
			)}
			{needsPassword ? (
				<Field
					id={`${id}-password`}
					label={t("devices:observe.shared.password", "Device password")}
					hint={t(
						"devices:observe.shared.passwordHint",
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
			<ConsequencePreview rows={consequence(t, renew)} />
			{failure ? (
				<InlineResult tone="critical" onDismiss={() => setFailure(null)}>
					{failure}
				</InlineResult>
			) : null}
		</DvSheet>
	);
}

function consequence(t: DevicesT, renew: boolean): ConsequenceRows {
	return renew
		? {
				what: t(
					"devices:observe.shared.renewWhat",
					"Signs the same readers list again, valid for one more day.",
				),
				who: t(
					"devices:observe.shared.renewWho",
					"Nobody loses access. Readers keep receiving samples.",
				),
				when: t("devices:observe.shared.renewWhen", "Immediately."),
				undo: {
					reversible: null,
					text: t(
						"devices:observe.shared.renewUndo",
						"Not needed: it changes nothing but the date.",
					),
				},
			}
		: {
				what: t(
					"devices:observe.shared.addWhat",
					"The new readers join the encrypted group and receive every sample from now on.",
				),
				who: t(
					"devices:observe.shared.addWho",
					"Current readers keep reading. The list is signed for one more day.",
				),
				when: t(
					"devices:observe.shared.addWhen",
					"From the next sample, a few seconds after they join.",
				),
				undo: {
					reversible: true,
					text: t(
						"devices:observe.shared.addUndo",
						"A reader leaves when the list is renewed without them, or when it expires.",
					),
				},
			};
}

function ReaderRequest({
	target,
	scope,
}: Readonly<{ target: ObserveTarget; scope: string }>) {
	const { t } = useTranslation("devices");
	const workspace = useDeviceWorkspace();
	const { copied, copy } = useCopy();
	const [busy, setBusy] = useState(false);
	const [request, setRequest] = useState<string | null>(null);
	const [note, setNote] = useState<{
		tone: "good" | "critical";
		text: string;
	} | null>(null);
	const alive = useRef(true);
	useEffect(() => {
		alive.current = true;
		return () => {
			alive.current = false;
		};
	}, []);
	const file = `metrics-reader-request-${target.deviceId.slice(0, 8)}.json`;

	const create = async () => {
		if (request) return request;
		setBusy(true);
		try {
			const text = JSON.stringify(
				await createMetricsRequest(workspace, target.deviceId, scope),
				null,
				2,
			);
			if (alive.current) setRequest(text);
			return text;
		} catch (error) {
			if (alive.current)
				setNote({
					tone: "critical",
					text: errorCopy(t, classifyDeviceError(error).code),
				});
			return null;
		} finally {
			if (alive.current) setBusy(false);
		}
	};

	return (
		<div className="flex flex-col gap-2">
			<p className="max-w-[72ch] text-ui text-muted-foreground">
				{t(
					"devices:observe.shared.requestIntro",
					"To read shared metrics on this computer, send the owner a reader request. They add it under Shared live metrics on their computer.",
				)}
			</p>
			<div className="flex flex-wrap gap-2">
				<DvButton
					size="sm"
					icon={copied ? Check : Copy}
					busy={busy}
					onClick={() =>
						void create().then((text) => {
							if (text) void copy(text);
						})
					}
				>
					{copied
						? t("devices:observe.shared.copied", "Copied")
						: t("devices:observe.shared.copyRequest", "Copy reader request")}
				</DvButton>
				<DvButton
					size="sm"
					icon={Download}
					disabled={busy}
					onClick={() =>
						void create().then((text) => {
							if (text && downloadText(file, text, "application/json"))
								setNote({
									tone: "good",
									text: t(
										"devices:observe.shared.requestSaved",
										"Saved as {{file}}. It holds no secret.",
										{ file },
									),
								});
						})
					}
				>
					{t(
						"devices:observe.shared.downloadRequest",
						"Download reader request",
					)}
				</DvButton>
			</div>
			{note ? (
				<InlineResult tone={note.tone} onDismiss={() => setNote(null)}>
					{note.text}
				</InlineResult>
			) : null}
		</div>
	);
}

function LiveReading({
	target,
	scope,
}: Readonly<{ target: ObserveTarget; scope: string }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const id = useId();
	const [reading, setReading] = useState(false);
	const gate = useGate("group_metrics_read", target.deviceId);
	const stream = useLiveStream<GroupMetricsSample>(
		target.deviceId,
		reading && gate.ok ? { kind: "group_metrics", scope } : null,
	);
	const sample = stream.data;
	const state = MLS_STATES.includes(sample?.state as MlsState)
		? enumLabel(t, "mlsResult", sample?.state as MlsState)
		: undefined;
	const gated = gateLine(t, gate, time);
	return (
		<div className="flex flex-col gap-2">
			<SwitchField
				id={`${id}-reading`}
				checked={reading && gate.ok}
				disabled={!gate.ok}
				onCheckedChange={setReading}
			>
				{t(
					"devices:observe.shared.readHere",
					"Read the shared samples on this computer",
				)}
				<span className="block text-xs text-muted-foreground">
					{gated?.reason ??
						t(
							"devices:observe.shared.readHereHint",
							"The device learns that this computer received each sample.",
						)}
				</span>
			</SwitchField>
			{reading && gate.ok ? (
				sample?.sample ? (
					<div className="overflow-hidden rounded-lg border border-hairline">
						<p className="flex flex-wrap items-center gap-x-3 gap-y-1 border-b border-hairline bg-surface-sunken px-4 py-1.5 text-xs text-muted-foreground">
							<FreshnessStamp {...stampOf(stream.freshness)} />
							{state ? <span>{state}</span> : null}
							<span>
								{sample.receiptPending
									? t(
											"devices:observe.shared.receiptPending",
											"Delivery not confirmed yet; it is retried on the next read.",
										)
									: t(
											"devices:observe.shared.receiptDone",
											"Delivery confirmed.",
										)}
							</span>
						</p>
						<DeviceResourceCells
							samples={[
								{ at: stream.freshness.at ?? time.nowS, data: sample.sample },
							]}
							lastKnown={false}
							services={target.services}
						/>
					</div>
				) : stream.freshness.age === "error" ? (
					<InlineResult tone="warning">
						{t(
							"devices:observe.shared.readFailed",
							"No shared sample could be read. This computer may not be a reader yet, or its reader was removed.",
						)}
					</InlineResult>
				) : (
					<StateView
						kind="loading"
						title={t(
							"devices:observe.shared.readingSamples",
							"Waiting for a shared sample…",
						)}
					/>
				)
			) : null}
		</div>
	);
}

/** SPEC §5.2 "Shared live metrics": advanced, collapsed by default; nothing is read until it is opened. */
export function SharedLiveMetrics({
	target,
}: Readonly<{ target: ObserveTarget }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const scope = DEVICE_SCOPE;
	const [open, setOpen] = useState(false);
	const [sheet, setSheet] = useState<"add" | "renew" | null>(null);
	const session = useKeySession(target.deviceId);
	const shared = useSharedRoster(target, scope, open);
	const gate = useGate("approve_metric_readers", target.deviceId, {
		extra: { policyApplied: policyAppliedOf(target) },
	});
	const save = useSaveMetricReaders(target, scope);
	const results = useInlineResults(sharedResultKey(target.deviceId, scope));
	const people = usePeople(
		(shared.data?.readers ?? []).flatMap((reader) =>
			reader.userId ? [reader.userId] : [],
		),
	);
	const gated = gateLine(t, gate, time);
	const roster = shared.data?.roster;
	const expired = roster ? roster.expires_at <= time.nowS : false;

	const renew = async () => {
		if (!session.canSign) {
			setSheet("renew");
			return;
		}
		const outcome = await save({
			add: [],
			label: t(
				"devices:observe.shared.renewLabel",
				"Renew shared metric readers",
			),
			consequence: consequence(t, true),
		});
		if (outcome.status === "password_required") setSheet("renew");
	};

	let body: ReactNode;
	if (livePhase(target) !== "open")
		body = <LiveDataState target={target} what="metrics" />;
	else if (shared.failed)
		body = (
			<StateView
				kind="error"
				title={t(
					"devices:observe.shared.failed",
					"The shared readers of {{device}} couldn't be read",
					{ device: target.name },
				)}
				actions={
					<DvButton
						size="sm"
						icon={RefreshCw}
						busy={shared.reading}
						onClick={() => void shared.refresh()}
					>
						{t("devices:observe.shared.retry", "Try again")}
					</DvButton>
				}
			/>
		);
	else if (!shared.data)
		body = (
			<StateView
				kind="loading"
				title={t(
					"devices:observe.shared.reading",
					"Reading the shared readers…",
				)}
			/>
		);
	else
		body = (
			<>
				{roster ? (
					<KeyValueList>
						<KvRow label={t("devices:observe.shared.readers", "Readers")}>
							<ReadersValue shared={shared.data} people={people} />
						</KvRow>
						<KvRow
							label={
								expired
									? t(
											"devices:observe.shared.expiredLabel",
											"Readers list expired",
										)
									: t("devices:observe.shared.expires", "Readers list expires")
							}
							className={expired ? "text-warning" : undefined}
						>
							{time.at(roster.expires_at)}
						</KvRow>
						<KvRow label={t("devices:observe.shared.delivery", "Delivery")}>
							{deliveryText(t, shared.data.readers)}
						</KvRow>
					</KeyValueList>
				) : (
					<StateView
						kind="empty"
						title={t("devices:observe.shared.notSetUp", "Not set up")}
						text={
							target.owner
								? t(
										"devices:observe.shared.notSetUpOwner",
										"Nobody receives shared samples from {{device}} yet. Add readers to start.",
										{ device: target.name },
									)
								: t(
										"devices:observe.shared.notSetUpShared",
										"The owner hasn't set up shared live metrics for {{device}}.",
										{ device: target.name },
									)
						}
					/>
				)}
				{target.owner ? (
					<div className="flex flex-wrap items-start gap-2">
						<GatedAction gate={gated}>
							<DvButton
								size="sm"
								icon={UserPlus}
								onClick={() => setSheet("add")}
							>
								{t("devices:observe.shared.add", "Add readers…")}
							</DvButton>
						</GatedAction>
						{roster && !gated ? (
							<DvButton size="sm" onClick={() => void renew()}>
								{t("devices:observe.shared.renew", "Renew")}
							</DvButton>
						) : null}
					</div>
				) : (
					<ReaderRequest target={target} scope={scope} />
				)}
				{roster ? <LiveReading target={target} scope={scope} /> : null}
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

	return (
		<details
			id="observe-shared-metrics"
			data-block=""
			open={open}
			onToggle={(event) => setOpen(event.currentTarget.open)}
			className="min-w-0 overflow-clip rounded-lg border border-border bg-card"
		>
			<summary className="flex min-h-12 cursor-pointer list-none flex-wrap items-center gap-x-3 gap-y-2 px-4 py-3 [&::-webkit-details-marker]:hidden">
				<h2 className="inline-flex min-w-0 items-center gap-2 text-[15px]/5 font-semibold tracking-[-0.005em] text-foreground">
					<RadioTower aria-hidden className="size-4 shrink-0 text-ink-2" />
					{t("devices:observe.shared.title", "Shared live metrics")}
				</h2>
				<span className="flex-1" />
				{open && shared.data ? (
					<FreshnessStamp
						source="live"
						age="live"
						observedAt={shared.data.readAt}
						cadenceSec={60}
					/>
				) : null}
				<span className="text-xs text-muted-foreground">
					{t("devices:observe.shared.advanced", "Advanced")}
				</span>
			</summary>
			{open ? (
				<div className="flex min-w-0 flex-col gap-3 border-t border-hairline px-4 py-3">
					<p className="max-w-[72ch] text-xs text-muted-foreground">
						{t(
							"devices:observe.shared.intro",
							"An encrypted feed of the device's resource samples that several readers receive at the same time. The live values above don't need it.",
						)}
					</p>
					{body}
				</div>
			) : null}
			{sheet ? (
				<ReadersSheet
					target={target}
					scope={scope}
					renew={sheet === "renew"}
					onClose={() => setSheet(null)}
				/>
			) : null}
		</details>
	);
}
