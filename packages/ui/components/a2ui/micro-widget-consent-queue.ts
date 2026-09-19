"use client";

import { useCallback, useEffect, useRef, useSyncExternalStore } from "react";
import { microWidgetConsentKey } from "./micro-widget-capability-consent";
import { normalizeWidgetPolicy } from "./micro-widget-policy";
import type { MicroWidgetConsentPrompt } from "./use-micro-widget-grant";

/**
 * One consent dialog per document (§14.5.7). Entries are keyed by consent key
 * plus policy digest; instances asking for the same thing share an entry and
 * the first of them renders the dialog. Order is join order, except that
 * Review, the runtime banner and the blocked card jump to the front.
 */

export interface MicroWidgetConsentQueueEntry {
	readonly key: string;
	/** Instances waiting on this entry; the first one renders the dialog. */
	readonly members: readonly string[];
}

export type MicroWidgetConsentQueueSnapshot =
	readonly MicroWidgetConsentQueueEntry[];

export class MicroWidgetConsentQueue {
	private entries: MicroWidgetConsentQueueSnapshot = [];
	private readonly frontRequests = new Set<string>();
	private readonly listeners = new Set<() => void>();

	subscribe = (listener: () => void): (() => void) => {
		this.listeners.add(listener);
		return () => {
			this.listeners.delete(listener);
		};
	};

	getSnapshot = (): MicroWidgetConsentQueueSnapshot => this.entries;

	/**
	 * Puts `member` on the entry for `key`. A member that moves to another key
	 * (a newer frozen prompt) keeps its place in line.
	 */
	join(key: string, member: string): void {
		const front = this.frontRequests.delete(member);
		const next = [...this.entries];
		const current = next.findIndex((entry) => entry.members.includes(member));
		if (current >= 0 && next[current].key === key) {
			if (front) this.set(moveToFront(next, current));
			return;
		}
		let insertAt = next.length;
		if (current >= 0) {
			const previous = next[current];
			const rest = previous.members.filter((id) => id !== member);
			if (rest.length === 0) next.splice(current, 1);
			else next[current] = { key: previous.key, members: rest };
			insertAt = current;
		}
		const existing = next.findIndex((entry) => entry.key === key);
		if (existing >= 0) {
			next[existing] = {
				key,
				members: [...next[existing].members, member],
			};
			this.set(front ? moveToFront(next, existing) : next);
			return;
		}
		next.splice(front ? 0 : insertAt, 0, { key, members: [member] });
		this.set(next);
	}

	leave(member: string): void {
		this.frontRequests.delete(member);
		const current = this.entries.findIndex((entry) =>
			entry.members.includes(member),
		);
		if (current < 0) return;
		const next = [...this.entries];
		const rest = next[current].members.filter((id) => id !== member);
		if (rest.length === 0) next.splice(current, 1);
		else next[current] = { key: next[current].key, members: rest };
		this.set(next);
	}

	/** The next entry `member` joins, or the one it is on, goes to the front. */
	requestFront(member: string): void {
		const current = this.entries.findIndex((entry) =>
			entry.members.includes(member),
		);
		if (current >= 0) {
			this.set(moveToFront([...this.entries], current));
			return;
		}
		this.frontRequests.add(member);
	}

	private set(entries: MicroWidgetConsentQueueEntry[]): void {
		this.entries = entries;
		for (const listener of this.listeners) listener();
	}
}

function moveToFront(
	entries: MicroWidgetConsentQueueEntry[],
	index: number,
): MicroWidgetConsentQueueEntry[] {
	if (index <= 0) return entries;
	const [entry] = entries.splice(index, 1);
	return [entry, ...entries];
}

export const microWidgetConsentQueue = new MicroWidgetConsentQueue();

/** Same consent target and the same frozen descriptor ⇒ one shared entry. */
export function microWidgetConsentQueueKey(
	prompt: MicroWidgetConsentPrompt,
	appId: string | null | undefined,
): string {
	const { subject } = prompt;
	return JSON.stringify([
		microWidgetConsentKey({
			source: subject.source,
			appId: appId ?? null,
			packageId: subject.packageId,
			widgetId: subject.widgetId,
		}),
		prompt.descriptor?.policyDigest ??
			JSON.stringify(normalizeWidgetPolicy(subject.policy)),
	]);
}

export interface MicroWidgetConsentQueueSlot {
	/** 1-based place of this instance's entry; 0 while it has none. */
	position: number;
	total: number;
	/** This instance renders the document's dialog. */
	active: boolean;
	/** On the queue but another dialog, or another instance with the same entry, is shown. */
	waiting: boolean;
	jumpToFront: () => void;
}

let nextMember = 0;

export function useMicroWidgetConsentQueue(
	key: string | null,
	queue: MicroWidgetConsentQueue = microWidgetConsentQueue,
): MicroWidgetConsentQueueSlot {
	const memberRef = useRef<string | null>(null);
	memberRef.current ??= `widget-consent-${++nextMember}`;
	const member = memberRef.current;
	const entries = useSyncExternalStore(
		queue.subscribe,
		queue.getSnapshot,
		queue.getSnapshot,
	);

	useEffect(() => {
		if (key === null) queue.leave(member);
		else queue.join(key, member);
	}, [queue, key, member]);

	useEffect(() => () => queue.leave(member), [queue, member]);

	const jumpToFront = useCallback(
		() => queue.requestFront(member),
		[queue, member],
	);

	const index = entries.findIndex(
		(entry) => entry.key === key && entry.members.includes(member),
	);
	const active = index === 0 && entries[0].members[0] === member;
	return {
		position: index + 1,
		total: entries.length,
		active,
		waiting: index >= 0 && !active,
		jumpToFront,
	};
}
