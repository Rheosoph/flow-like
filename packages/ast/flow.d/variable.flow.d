// Variable — FlowScript node declarations (generated, do not edit).
// One `function` per catalog node, grouped by FlowScript namespace. Call a node as
// `ns::alias({ pin: value })`, or write `use ns::*` once at the top of a .flow file and
// call `alias({ pin: value })`. A `this: T` parameter marks the receiver pin: such a node
// is also a method on that value (`x.alias(...)`, remaining inputs positional or named).
// JSDoc tags carry the node type (`@node`), the receiver pin (`@receiver`) and the legacy
// camelCase spelling (`@alias`), which is still accepted.

declare namespace variable {
    // === Variable ===

    /**
     * Get Variable Value
     * @node variable_get @alias variableGet
     * @param varRef — The reference to the variable
     * @returns valueRef — The value of the variable
     */
    function get({ varRef: string }): any;

    /**
     * Set Variable Value
     * @node variable_set @alias variableSet
     * @param varRef — The reference to the variable
     * @param valueIn — The value of the variable
     * @returns valueRef — The newly set value
     * @impure has side effects / drives control flow
     */
    function set({ varRef: string, valueIn: any }): any;
}
