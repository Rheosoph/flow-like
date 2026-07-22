import {
	detectFlowScriptCandidateRegression,
	extractFlowScriptWorkspaceCandidates,
	isFlowScriptWorkspaceApplicable,
	isPartialFlowScriptWorkspace,
	parseFlowScriptWorkspaceCandidate,
	profileFlowScriptCandidate,
	rememberFlowScriptWorkspaceCandidate,
	resolveFinalFlowScriptWorkspaceCandidate,
	resolveFlowScriptWorkspaceCandidate,
	selectBestRecoverableFlowScriptCandidate,
	shouldPromoteFlowScriptWorkspaceEvents,
} from "@flow-like/flow-like-ui/components/flowpilot/flowscript-workspace-candidates";
import { describe, expect, test } from "vitest";

const richDraft = `function buildSupportReply() {
  logInfo({ message: "draft" })
}

eventsSimple() {
  buildSupportReply()
}`;
const tinyDraft = `eventsSimple() {
  logInfo({ message: "ok" })
}`;

describe("FlowScript workspace candidate selection", () => {
	test("parses raw documents and status envelopes", () => {
		expect(parseFlowScriptWorkspaceCandidate(tinyDraft)).toEqual({
			source: tinyDraft,
		});
		expect(
			parseFlowScriptWorkspaceCandidate(
				JSON.stringify({ source: richDraft, status: "validation_errors" }),
			),
		).toEqual({ source: richDraft, status: "validation_errors" });
	});

	test("treats live drafting source as preview-only until it is queued", () => {
		expect(
			isFlowScriptWorkspaceApplicable({
				source: richDraft,
				status: "drafting",
			}),
		).toBe(false);
		expect(
			isFlowScriptWorkspaceApplicable({
				source: richDraft,
				status: "submitted",
			}),
		).toBe(false);
		expect(
			isFlowScriptWorkspaceApplicable({ source: richDraft, status: "queued" }),
		).toBe(true);
	});

	test("profiles legacy and canonical Event headers identically", () => {
		const withHeader = (header: string) => `function buildPayload() {
  domainLookup()
}

${header} {
  buildPayload()
}`;
		const legacy = profileFlowScriptCandidate(
			withHeader("eventsGeneric(payload: Struct)"),
		);
		const canonical = profileFlowScriptCandidate(
			withHeader("eventsGeneric wikiExplorerLoad(payload: Struct)"),
		);

		expect(canonical).toEqual(legacy);
		expect(canonical).toMatchObject({
			eventEntries: 1,
			callSites: 2,
			callNames: ["buildpayload", "domainlookup"],
			eventsCallingHelpers: 1,
			helperDomainCallSites: 1,
		});
		expect(canonical.callNames).not.toContain("wikiexplorerload");
	});

	test("retains every candidate when transport batches several frames", () => {
		const chunk = [
			"before",
			`<flowscript_workspace>${JSON.stringify({ source: richDraft, status: "submitted" })}</flowscript_workspace>`,
			`<flowscript_workspace>${JSON.stringify({ source: richDraft, status: "validation_errors" })}</flowscript_workspace>`,
			`<flowscript_workspace>${JSON.stringify({ source: tinyDraft, status: "queued" })}</flowscript_workspace>`,
			"after",
		].join("");

		const extracted = extractFlowScriptWorkspaceCandidates(chunk);
		expect(extracted.candidates).toEqual([
			{ source: richDraft, status: "submitted" },
			{ source: richDraft, status: "validation_errors" },
			{ source: tinyDraft, status: "queued" },
		]);
		expect(extracted.remainder).toBe("beforeafter");
	});

	test("never borrows queued status from a different candidate", () => {
		let history = rememberFlowScriptWorkspaceCandidate([], {
			source: richDraft,
			status: "validation_errors",
		});
		history = rememberFlowScriptWorkspaceCandidate(history, {
			source: tinyDraft,
			status: "queued",
		});

		const selected = resolveFlowScriptWorkspaceCandidate(history, {
			source: richDraft,
		});
		expect(selected).toEqual({
			source: richDraft,
			status: "validation_errors",
		});
		expect(isFlowScriptWorkspaceApplicable(selected)).toBe(false);
	});

	test("uses the status belonging to the authoritative final source", () => {
		const history = [
			{ source: richDraft, status: "validation_errors" },
			{ source: tinyDraft, status: "queued" },
		];

		const selected = resolveFinalFlowScriptWorkspaceCandidate(
			history,
			tinyDraft,
			false,
		);
		expect(selected).toEqual({ source: tinyDraft, status: "queued" });
		expect(isFlowScriptWorkspaceApplicable(selected)).toBe(true);
	});

	test("accepts an unstreamed raw final source only with validated commands", () => {
		const withoutCommands = resolveFinalFlowScriptWorkspaceCandidate(
			[],
			richDraft,
			false,
		);
		const withCommands = resolveFinalFlowScriptWorkspaceCandidate(
			[],
			richDraft,
			true,
		);

		expect(withoutCommands).toEqual({ source: richDraft });
		expect(isFlowScriptWorkspaceApplicable(withoutCommands)).toBe(false);
		expect(withCommands).toEqual({ source: richDraft, status: "queued" });
		expect(isFlowScriptWorkspaceApplicable(withCommands)).toBe(true);
	});

	test("preserves partial-slice metadata without promoting production Events", () => {
		const retainedFullSource = `${richDraft}\n// unresolved approval branch`;
		const candidate = parseFlowScriptWorkspaceCandidate(
			JSON.stringify({
				source: tinyDraft,
				status: "queued",
				completion: "partial_working_slice",
				retained_full_source: retainedFullSource,
				regression: {
					previous_call_sites: 8,
					candidate_call_sites: 2,
				},
			}),
		);

		expect(candidate).toEqual({
			source: tinyDraft,
			status: "queued",
			completion: "partial_working_slice",
			retained_full_source: retainedFullSource,
			regression: {
				previous_call_sites: 8,
				candidate_call_sites: 2,
			},
		});
		expect(isPartialFlowScriptWorkspace(candidate)).toBe(true);
		expect(isFlowScriptWorkspaceApplicable(candidate)).toBe(true);
		expect(shouldPromoteFlowScriptWorkspaceEvents(candidate, false, true)).toBe(
			false,
		);
	});

	test("merges final partial metadata with the queued status for the same source", () => {
		const selected = resolveFinalFlowScriptWorkspaceCandidate(
			[{ source: tinyDraft, status: "queued" }],
			JSON.stringify({
				source: tinyDraft,
				completion: "partial_working_slice",
				retained_full_source: richDraft,
			}),
			false,
		);

		expect(selected).toEqual({
			source: tinyDraft,
			status: "queued",
			completion: "partial_working_slice",
			retained_full_source: richDraft,
		});
		expect(isFlowScriptWorkspaceApplicable(selected)).toBe(true);
		expect(shouldPromoteFlowScriptWorkspaceEvents(selected, false, true)).toBe(
			false,
		);
	});

	test("allows Event promotion for a complete queued workspace", () => {
		const candidate = { source: richDraft, status: "queued" };
		expect(shouldPromoteFlowScriptWorkspaceEvents(candidate, false, true)).toBe(
			true,
		);
	});

	test("keeps failed candidates in bounded turn history", () => {
		let history = rememberFlowScriptWorkspaceCandidate([], {
			source: richDraft,
			status: "validation_errors",
		});
		history = rememberFlowScriptWorkspaceCandidate(history, {
			source: tinyDraft,
			status: "queued",
		});

		expect(history).toEqual([
			{ source: richDraft, status: "validation_errors" },
			{ source: tinyDraft, status: "queued" },
		]);
	});

	test("detects a same-objective collapse to an Event plus log stub", () => {
		const fullDraft = `@secret
const IMAP_PASSWORD: string = ""

function pollInbox() {
  const connection = emailImapConnect({ username: "support", password: IMAP_PASSWORD })
  const inbox = mailImapInbox({ connection: connection })
  const unread = mailImapList({ inbox: inbox, filter: "UNSEEN" })
  for (const item of controlForEach({ array: unread })) {
    emailImapInboxFetchMail({ emailRef: item.value })
    logInfo({ message: "received" })
  }
}

function requestApproval() {
  emailSmtpSend({ to: "reviewer@example.com", subject: "Review" })
}

eventsSimple() {
  pollInbox()
  requestApproval()
}`;

		const regression = detectFlowScriptCandidateRegression(
			profileFlowScriptCandidate(fullDraft),
			profileFlowScriptCandidate(tinyDraft),
		);
		expect(regression).toMatchObject({
			candidate_call_sites: 1,
		});
		expect(regression?.previous_call_sites).toBeGreaterThanOrEqual(8);

		const protectedCandidate = resolveFinalFlowScriptWorkspaceCandidate(
			[{ source: fullDraft, status: "validation_errors" }],
			JSON.stringify({ source: tinyDraft, status: "queued" }),
			true,
		);
		expect(protectedCandidate).toMatchObject({
			source: fullDraft,
			status: "validation_errors",
			completion: "regression_blocked",
		});
		expect(isFlowScriptWorkspaceApplicable(protectedCandidate)).toBe(false);

		const modularSlice = `function fetchUnread() {
  emailImapConnect({ username: "support", password: "" })
}

eventsSimple() {
  fetchUnread()
}`;
		const partial = resolveFinalFlowScriptWorkspaceCandidate(
			[{ source: fullDraft, status: "validation_errors" }],
			JSON.stringify({ source: modularSlice, status: "queued" }),
			true,
		);
		expect(partial).toMatchObject({
			source: modularSlice,
			status: "queued",
			completion: "partial_working_slice",
			retained_full_source: fullDraft,
		});
		expect(shouldPromoteFlowScriptWorkspaceEvents(partial, false, true)).toBe(
			false,
		);
	});

	test("does not flag ordinary repair edits and retains the richer failed draft", () => {
		const repaired = richDraft.replace('"draft"', '"repaired"');
		expect(
			detectFlowScriptCandidateRegression(
				profileFlowScriptCandidate(richDraft),
				profileFlowScriptCandidate(repaired),
			),
		).toBeUndefined();

		expect(
			selectBestRecoverableFlowScriptCandidate([
				{ source: tinyDraft, status: "queued" },
				{ source: richDraft, status: "validation_errors" },
			]),
		).toEqual({ source: richDraft, status: "validation_errors" });
	});
});
