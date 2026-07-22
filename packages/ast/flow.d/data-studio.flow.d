// Data Studio — FlowScript node declarations (generated, do not edit).
// One declare-function per catalog node. Names are camelCase node types.

// === Data Studio/Actions ===

/**
 * Reads the typed objects and parameters the ontology action was invoked with
 * @param ontologyId — Saved ontology identifier (types the outputs from the action contract)
 * @param actionId — Saved ontology action identifier (types the outputs from the action contract)
 * @returns object — The first (or only) object the action was invoked with
 * @returns objects — Every object the action was invoked with
 * @returns parameters — Typed parameters the action was invoked with
 * @returns objectType — Object type the action targets
 * @returns objectIds — Identifiers of the targeted objects
 * @returns idempotencyKey — Client-supplied retry key, if any
 * @impure has side effects / drives control flow
 */
declare function ontologyActionInput({ ontologyId: string, actionId: string }): { object: Struct, objects: Struct[], parameters: Struct, objectType: string, objectIds: string[], idempotencyKey: string };

/**
 * Builds a validated, typed action request from a Data Studio action binding
 * @param ontologyId — Saved ontology identifier
 * @param actionId — Saved ontology action identifier
 * @param objects — Objects selected for the action
 * @param parameters (optional) — Typed parameters supplied to the action
 * @returns errorMessage — Details for a failed action request
 * @returns actionRequest — Validated action binding, objects, and parameters
 * @impure has side effects / drives control flow
 */
declare function ontologyActionRequest({ ontologyId: string, actionId: string, objects: Struct[], parameters?: Struct }): { errorMessage: string, actionRequest: Struct };


// === Data Studio/Objects ===

/**
 * Reads a bounded object preview through a saved Data Studio ontology
 * @param ontologyId — Saved ontology identifier
 * @param objectType — Stable object type label resolved by the ontology
 * @param limit (optional) — Maximum number of objects to return (capped at 500)
 * @returns errorMessage — Details for a failed object read
 * @returns objects — Typed objects from the selected ontology object type
 * @impure has side effects / drives control flow
 */
declare function ontologyQueryObjects({ ontologyId: string, objectType: string, limit?: int }): { errorMessage: string, objects: Struct[] };


// === Data Studio/Remote Actions ===

/**
 * Runs a governed ontology action in a connected project through an installed contract; the producer validates and executes it authoritatively
 * @param bindingId — Local identifier of the installed remote ontology contract
 * @param actionId — Action identifier resolved through the installed contract
 * @param objects — Objects selected for the action
 * @param parameters (optional) — Typed parameters supplied to the action
 * @param timeout (optional) — Maximum seconds to wait for the remote action to finish (capped at 1800)
 * @returns errorMessage — Details for a failed remote action
 * @returns result — Result payload emitted by the producer's action run
 * @returns runId — Identifier of the producer-side action run
 * @impure has side effects / drives control flow
 */
declare function ontologyActionRequestRemote({ bindingId: string, actionId: string, objects: Struct[], parameters?: Struct, timeout?: int }): { errorMessage: string, result: Struct, runId: string };


// === Data Studio/Remote Objects ===

/**
 * Expands a parent object's containment children through an installed ontology contract from a connected project
 * @param bindingId — Local identifier of the installed remote ontology contract
 * @param objectType — Stable object type identifier of the parent, resolved through the installed contract
 * @param nodeId — Identifier of the parent object whose children should be loaded
 * @param limit (optional) — Maximum number of child objects to return (capped at 500)
 * @returns errorMessage — Details for a failed remote children read
 * @returns objects — Typed child objects reached through containment edges of the installed contract
 * @impure has side effects / drives control flow
 */
declare function ontologyQueryRemoteChildren({ bindingId: string, objectType: string, nodeId: any, limit?: int }): { errorMessage: string, objects: Struct[] };

/**
 * Reads a bounded object preview through an installed ontology contract from a connected project
 * @param bindingId — Local identifier of the installed remote ontology contract
 * @param objectType — Stable object type identifier resolved through the installed contract
 * @param limit (optional) — Maximum number of remote objects to return (capped at 500)
 * @returns errorMessage — Details for a failed remote object read
 * @returns objects — Typed objects from the installed remote ontology object type
 * @impure has side effects / drives control flow
 */
declare function ontologyQueryRemoteObjects({ bindingId: string, objectType: string, limit?: int }): { errorMessage: string, objects: Struct[] };

