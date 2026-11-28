// Bots — FlowScript node declarations (generated, do not edit).
// One `function` per catalog node, grouped by FlowScript namespace. Call a node as
// `ns::alias({ pin: value })`, or write `use ns::*` once at the top of a .flow file and
// call `alias({ pin: value })`. Leading inputs may also be passed positionally, in
// declaration order, before the named object: `ns::alias(a, b, { pin: value })`. A
// `this: T` parameter marks the receiver pin: such a node is also a method on that value
// (`x.alias(...)`, remaining inputs positional or named).
// JSDoc tags carry the node type (`@node`), the receiver pin (`@receiver`) and the legacy
// camelCase spelling (`@alias`), which is still accepted.

declare namespace bots {
    // === Bots ===

    /**
     * Sends a message to one of your bots, as if a flow event had woken it, and optionally waits for its answer. The exchange lands in the bot's chat with you. Works for bots on a server or on a desktop that is online; a bot that only lives in a local store can't be reached.
     * @node bots_ask_bot @alias botsAskBot
     * @param botId (optional) — The id of one of your bots (the person this run acts as)
     * @param message (optional) — What the bot is asked, at most 8,000 characters
     * @param waitSeconds (optional) — How long to wait for the bot's answer, at most 100. 0 continues as soon as the turn is queued.
     * @returns turnId — The turn the message queued
     * @returns status — queued, running, waiting (for your approval), done, failed or cancelled
     * @returns reply — The bot's answer; empty when you didn't wait or it hasn't answered yet
     * @impure has side effects / drives control flow
     */
    function askBot({ botId?: string, message?: string, waitSeconds?: int }): { turnId: string, status: string, reply: string };
}
