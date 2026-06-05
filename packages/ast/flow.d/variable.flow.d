// Variable — FlowScript node declarations (generated, do not edit).
// One declare-function per catalog node. Names are camelCase node types.

// === Variable ===

/**
 * Get Variable Value
 * @param varRef — The reference to the variable
 * @returns valueRef — The value of the variable
 */
declare function variableGet({ varRef: string }): any;

/**
 * Set Variable Value
 * @param varRef — The reference to the variable
 * @param valueIn — The value of the variable
 * @returns valueRef — The newly set value
 * @impure has side effects / drives control flow
 */
declare function variableSet({ varRef: string, valueIn: any }): any;

