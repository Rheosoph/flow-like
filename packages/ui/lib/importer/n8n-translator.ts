import type { IBoard, INode, IPin } from "../schema";
import { ICommentType, IPinType, IValueType, IVariableType } from "../schema";
import {
	type CatalogIndex,
	addCommentToBoard,
	addExecPins,
	addLayerToBoard,
	addNodeToBoard,
	addVariableToBoard,
	buildCatalogIndex,
	cloneNodeFromCatalog,
	computeLayerCoordinates,
	connectPins,
	createBridgePinsForBoard,
	createCompositionLayer,
	createEmptyBoard,
	createNode,
	createPin,
	createTodoLayer,
	createVariable,
	diagError,
	findPinByName,
	info,
	now,
	setPinDefault,
	warn,
} from "./board-builder";
import type {
	N8nNode,
	N8nWorkflow,
	NodeMappingType,
	TranslateN8nOptions,
	TranslationDiagnostic,
	TranslationResult,
} from "./types";
import {
	MAPPING_DEFS,
	N8N_MAPPING_OVERRIDES,
	resolveN8nMappingDefs,
} from "./mappings";
import type {
	FlowDirectDef,
	FlowLayerDef,
	N8nManualMappingOverrides,
	ParameterRule,
	ResolvedN8nMappingDef,
} from "./mappings/types";

interface NodeMapping {
	catalog: string;
	category: string;
	type: NodeMappingType;
	isEvent?: boolean;
	skipDefaultExecPins?: boolean;
	fallbackConfigure?: (
		node: INode,
		n8nNode: N8nNode,
		diag: TranslationDiagnostic[],
	) => void;
	/** Catalog-driven: sets defaults on catalog-cloned node (all pins already present) */
	configure?: (
		node: INode,
		n8nNode: N8nNode,
		diag: TranslationDiagnostic[],
		board: IBoard,
		ci: CatalogIndex,
	) => void;
	/** Legacy fallback: creates pins from scratch when catalog unavailable */
	setupPins?: (
		node: INode,
		n8nNode: N8nNode,
		diag: TranslationDiagnostic[],
		board: IBoard,
	) => void;
}

function composeCompanion(
	board: IBoard,
	ci: CatalogIndex,
	companionCatalog: string,
	mainNode: INode,
	companionOutputPinName: string,
	mainInputPinName: string,
	companionFriendlyName?: string,
	configure?: (companion: INode) => void,
): INode | undefined {
	const [x, y] = mainNode.coordinates ?? [0, 0];
	const companion = cloneNodeFromCatalog(ci, companionCatalog, {
		friendlyName: companionFriendlyName ?? companionCatalog,
		x: x - 300,
		y,
	});
	if (!companion) return undefined;
	if (configure) configure(companion);
	addNodeToBoard(board, companion);
	const outPin = findPinByName(companion, companionOutputPinName, IPinType.Output);
	const inPin = findPinByName(mainNode, mainInputPinName, IPinType.Input);
	if (outPin && inPin) connectPins(companion, outPin, mainNode, inPin);
	return companion;
}

function setupHttpFetchPins(
	node: INode,
	n8nNode: N8nNode,
	diag: TranslationDiagnostic[],
	board: IBoard,
): void {
	const requestPin = createPin({
		name: "request",
		friendlyName: "Request",
		description: "The HTTP request to perform",
		pinType: IPinType.Input,
		dataType: IVariableType.Struct,
	});
	node.pins[requestPin.id] = requestPin;

	const responsePin = createPin({
		name: "response",
		friendlyName: "Response",
		description: "The HTTP response",
		pinType: IPinType.Output,
		dataType: IVariableType.Struct,
	});
	node.pins[responsePin.id] = responsePin;

	const errorExec = createPin({
		name: "exec_error",
		friendlyName: "Error",
		description: "Execution if the request fails",
		pinType: IPinType.Output,
		dataType: IVariableType.Execution,
	});
	node.pins[errorExec.id] = errorExec;

	const params = n8nNode.parameters;
	const url = typeof params.url === "string" ? params.url : undefined;
	const method =
		typeof params.method === "string"
			? String(params.method).toUpperCase()
			: "GET";

	// Compose: create an http_make_request node and connect it
	const [nodeX, nodeY] = node.coordinates ?? [0, 0];
	const makeReq = createNode({
		name: "http_make_request",
		friendlyName: `${n8nNode.name} (Request)`,
		description: "Creates the HTTP request",
		category: "Web/API/Request",
		x: nodeX - 300,
		y: nodeY,
	});

	const methodPin = createPin({
		name: "method",
		friendlyName: "Method",
		description: "HTTP Method",
		pinType: IPinType.Input,
		dataType: IVariableType.String,
		defaultValue: method,
		options: {
			valid_values: ["GET", "POST", "PUT", "DELETE", "PATCH"],
		},
	});
	makeReq.pins[methodPin.id] = methodPin;

	const urlPin = createPin({
		name: "url",
		friendlyName: "URL",
		description: "The request URL",
		pinType: IPinType.Input,
		dataType: IVariableType.String,
		defaultValue: url ?? "",
	});
	makeReq.pins[urlPin.id] = urlPin;

	const makeReqOut = createPin({
		name: "request",
		friendlyName: "Request",
		description: "The HTTP request",
		pinType: IPinType.Output,
		dataType: IVariableType.Struct,
	});
	makeReq.pins[makeReqOut.id] = makeReqOut;

	addNodeToBoard(board, makeReq);
	connectPins(makeReq, makeReqOut, node, requestPin);

	// Downstream: http_response_to_json (parse JSON body)
	const toJson = createNode({
		name: "http_response_to_json",
		friendlyName: `${n8nNode.name} (To Struct)`,
		description: "Parses JSON response body into a struct",
		category: "Web/API/Response",
		x: nodeX + 300,
		y: nodeY,
	});

	const toJsonExecIn = createPin({
		name: "exec_in",
		friendlyName: "▶",
		description: "Execution input",
		pinType: IPinType.Input,
		dataType: IVariableType.Execution,
	});
	toJson.pins[toJsonExecIn.id] = toJsonExecIn;

	const toJsonExecOut = createPin({
		name: "exec_out",
		friendlyName: "▶",
		description: "Execution output",
		pinType: IPinType.Output,
		dataType: IVariableType.Execution,
	});
	toJson.pins[toJsonExecOut.id] = toJsonExecOut;

	const toJsonResponse = createPin({
		name: "response",
		friendlyName: "Response",
		description: "The HTTP response",
		pinType: IPinType.Input,
		dataType: IVariableType.Struct,
	});
	toJson.pins[toJsonResponse.id] = toJsonResponse;

	const toJsonStruct = createPin({
		name: "struct",
		friendlyName: "Struct",
		description: "Parsed JSON body",
		pinType: IPinType.Output,
		dataType: IVariableType.Struct,
	});
	toJson.pins[toJsonStruct.id] = toJsonStruct;

	addNodeToBoard(board, toJson);
	connectPins(node, responsePin, toJson, toJsonResponse);

	// Wire exec: find the exec_out on the http_fetch node
	const fetchExecOut = Object.values(node.pins).find(
		(p) => p.name === "exec_out" && p.pin_type === IPinType.Output,
	);
	if (fetchExecOut) connectPins(node, fetchExecOut, toJson, toJsonExecIn);

	// Downstream: struct_get (access fields with dot notation)
	const getField = createNode({
		name: "struct_get",
		friendlyName: `${n8nNode.name} (Get Field)`,
		description: "Access struct fields (supports user.name dot notation)",
		category: "Structs/Fields",
		x: nodeX + 600,
		y: nodeY,
	});

	const getFieldStruct = createPin({
		name: "struct",
		friendlyName: "Struct",
		description: "The struct to read from",
		pinType: IPinType.Input,
		dataType: IVariableType.Struct,
	});
	getField.pins[getFieldStruct.id] = getFieldStruct;

	const getFieldField = createPin({
		name: "field",
		friendlyName: "Field",
		description: "Field path (e.g. user.name, items[0].id)",
		pinType: IPinType.Input,
		dataType: IVariableType.String,
		defaultValue: "data",
	});
	getField.pins[getFieldField.id] = getFieldField;

	const getFieldValue = createPin({
		name: "value",
		friendlyName: "Value",
		description: "The extracted value",
		pinType: IPinType.Output,
		dataType: IVariableType.Generic,
	});
	getField.pins[getFieldValue.id] = getFieldValue;

	const getFieldFound = createPin({
		name: "found",
		friendlyName: "Found?",
		description: "Whether the field was found",
		pinType: IPinType.Output,
		dataType: IVariableType.Boolean,
	});
	getField.pins[getFieldFound.id] = getFieldFound;

	addNodeToBoard(board, getField);
	connectPins(toJson, toJsonStruct, getField, getFieldStruct);

	if (url) {
		node.comment = `HTTP ${method} ${url}`;
	}
}

function setupBranchPins(
	node: INode,
	_n8nNode: N8nNode,
	_diag: TranslationDiagnostic[],
): void {
	const conditionPin = createPin({
		name: "condition",
		friendlyName: "Condition",
		description: "The condition to evaluate",
		pinType: IPinType.Input,
		dataType: IVariableType.Boolean,
		defaultValue: true,
	});
	node.pins[conditionPin.id] = conditionPin;

	const truePin = createPin({
		name: "true",
		friendlyName: "True",
		description: "The flow to follow if the condition is true",
		pinType: IPinType.Output,
		dataType: IVariableType.Execution,
	});
	node.pins[truePin.id] = truePin;

	const falsePin = createPin({
		name: "false",
		friendlyName: "False",
		description: "The flow to follow if the condition is false",
		pinType: IPinType.Output,
		dataType: IVariableType.Execution,
	});
	node.pins[falsePin.id] = falsePin;
}

function setupForEachPins(
	node: INode,
	_n8nNode: N8nNode,
	_diag: TranslationDiagnostic[],
): void {
	const arrayPin = createPin({
		name: "array",
		friendlyName: "Array",
		description: "Array to loop over",
		pinType: IPinType.Input,
		dataType: IVariableType.Generic,
		valueType: IValueType.Array,
	});
	node.pins[arrayPin.id] = arrayPin;

	const execOutPin = createPin({
		name: "exec_out",
		friendlyName: "For Each Element",
		description: "Executes the current item",
		pinType: IPinType.Output,
		dataType: IVariableType.Execution,
	});
	node.pins[execOutPin.id] = execOutPin;

	const valuePin = createPin({
		name: "value",
		friendlyName: "Value",
		description: "The current item value",
		pinType: IPinType.Output,
		dataType: IVariableType.Generic,
	});
	node.pins[valuePin.id] = valuePin;

	const indexPin = createPin({
		name: "index",
		friendlyName: "Index",
		description: "Current array index",
		pinType: IPinType.Output,
		dataType: IVariableType.Integer,
	});
	node.pins[indexPin.id] = indexPin;

	const donePin = createPin({
		name: "done",
		friendlyName: "Done",
		description: "Executes once the array is fully iterated",
		pinType: IPinType.Output,
		dataType: IVariableType.Execution,
	});
	node.pins[donePin.id] = donePin;
}

function setupStructSetPins(
	node: INode,
	n8nNode: N8nNode,
	diag: TranslationDiagnostic[],
): void {
	const structInPin = createPin({
		name: "struct_in",
		friendlyName: "Struct",
		description: "Struct input",
		pinType: IPinType.Input,
		dataType: IVariableType.Struct,
	});
	node.pins[structInPin.id] = structInPin;

	const fieldPin = createPin({
		name: "field",
		friendlyName: "Field",
		description: "Field path to set",
		pinType: IPinType.Input,
		dataType: IVariableType.String,
	});
	node.pins[fieldPin.id] = fieldPin;

	const valuePin = createPin({
		name: "value",
		friendlyName: "Value",
		description: "Value to set",
		pinType: IPinType.Input,
		dataType: IVariableType.Generic,
	});
	node.pins[valuePin.id] = valuePin;

	const structOutPin = createPin({
		name: "struct_out",
		friendlyName: "Struct",
		description: "Struct output",
		pinType: IPinType.Output,
		dataType: IVariableType.Struct,
	});
	node.pins[structOutPin.id] = structOutPin;

	const params = n8nNode.parameters;
	const assignments = params.assignments as
		| { assignments?: Array<{ name: string; value: unknown; type: string }> }
		| undefined;
	if (assignments?.assignments && assignments.assignments.length > 0) {
		const fieldNames = assignments.assignments
			.map((a) => a.name)
			.join(", ");
		node.comment = `Sets fields: ${fieldNames}. Chain multiple struct_set nodes for each field.`;
	}
}

function setupEmailPins(
	node: INode,
	n8nNode: N8nNode,
	_diag: TranslationDiagnostic[],
	board: IBoard,
): void {
	const connectionPin = createPin({
		name: "connection",
		friendlyName: "Connection",
		description: "SMTP connection handle",
		pinType: IPinType.Input,
		dataType: IVariableType.Struct,
	});
	node.pins[connectionPin.id] = connectionPin;

	const fromPin = createPin({
		name: "from",
		friendlyName: "From",
		description: "From header (single address)",
		pinType: IPinType.Input,
		dataType: IVariableType.String,
	});
	node.pins[fromPin.id] = fromPin;

	const toPin = createPin({
		name: "to",
		friendlyName: "To",
		description: "Comma-separated list",
		pinType: IPinType.Input,
		dataType: IVariableType.String,
		defaultValue: n8nNode.parameters.sendTo ?? "",
	});
	node.pins[toPin.id] = toPin;

	const ccPin = createPin({
		name: "cc",
		friendlyName: "Cc",
		description: "Comma-separated list",
		pinType: IPinType.Input,
		dataType: IVariableType.String,
		defaultValue: "",
	});
	node.pins[ccPin.id] = ccPin;

	const bccPin = createPin({
		name: "bcc",
		friendlyName: "Bcc",
		description: "Comma-separated list",
		pinType: IPinType.Input,
		dataType: IVariableType.String,
		defaultValue: "",
	});
	node.pins[bccPin.id] = bccPin;

	const subjectPin = createPin({
		name: "subject",
		friendlyName: "Subject",
		description: "Subject line",
		pinType: IPinType.Input,
		dataType: IVariableType.String,
		defaultValue: n8nNode.parameters.subject ?? "(no subject)",
	});
	node.pins[subjectPin.id] = subjectPin;

	const bodyTextPin = createPin({
		name: "body_text",
		friendlyName: "Body (text)",
		description: "Plaintext body",
		pinType: IPinType.Input,
		dataType: IVariableType.String,
		defaultValue: n8nNode.parameters.message ?? "",
	});
	node.pins[bodyTextPin.id] = bodyTextPin;

	const bodyHtmlPin = createPin({
		name: "body_html",
		friendlyName: "Body (HTML)",
		description: "Optional HTML body",
		pinType: IPinType.Input,
		dataType: IVariableType.String,
		defaultValue: "",
	});
	node.pins[bodyHtmlPin.id] = bodyHtmlPin;

	const messageIdPin = createPin({
		name: "message_id",
		friendlyName: "Message-ID",
		description: "The generated Message-ID",
		pinType: IPinType.Output,
		dataType: IVariableType.String,
	});
	node.pins[messageIdPin.id] = messageIdPin;

	// Compose: create an SMTP connect node and wire it
	const [nodeX, nodeY] = node.coordinates ?? [0, 0];
	const smtpConnect = createNode({
		name: "email_smtp_connect",
		friendlyName: "SMTP Connect",
		description: "SMTP connection",
		category: "Mail/Smtp",
		x: nodeX - 300,
		y: nodeY,
	});

	const smtpExecIn = createPin({
		name: "exec_in",
		friendlyName: "▶",
		description: "Execution input",
		pinType: IPinType.Input,
		dataType: IVariableType.Execution,
	});
	smtpConnect.pins[smtpExecIn.id] = smtpExecIn;

	const hostPin = createPin({
		name: "host",
		friendlyName: "Host",
		description: "SMTP server host",
		pinType: IPinType.Input,
		dataType: IVariableType.String,
		defaultValue: "smtp.example.com",
	});
	smtpConnect.pins[hostPin.id] = hostPin;

	const portPin = createPin({
		name: "port",
		friendlyName: "Port",
		description: "SMTP server port",
		pinType: IPinType.Input,
		dataType: IVariableType.Integer,
		defaultValue: 587,
	});
	smtpConnect.pins[portPin.id] = portPin;

	const smtpExecOut = createPin({
		name: "exec_out",
		friendlyName: "▶",
		description: "Execution output",
		pinType: IPinType.Output,
		dataType: IVariableType.Execution,
	});
	smtpConnect.pins[smtpExecOut.id] = smtpExecOut;

	const connOut = createPin({
		name: "connection",
		friendlyName: "Connection",
		description: "SMTP connection handle",
		pinType: IPinType.Output,
		dataType: IVariableType.Struct,
	});
	smtpConnect.pins[connOut.id] = connOut;

	addNodeToBoard(board, smtpConnect);
	connectPins(smtpConnect, connOut, node, connectionPin);
}

function setupGoogleSheetsPins(
	node: INode,
	n8nNode: N8nNode,
	_diag: TranslationDiagnostic[],
	board: IBoard,
): void {
	const providerPin = createPin({
		name: "provider",
		friendlyName: "Provider",
		description: "Google Drive provider",
		pinType: IPinType.Input,
		dataType: IVariableType.Struct,
	});
	node.pins[providerPin.id] = providerPin;

	const sheetIdPin = createPin({
		name: "spreadsheet_id",
		friendlyName: "Spreadsheet ID",
		description: "ID of the spreadsheet",
		pinType: IPinType.Input,
		dataType: IVariableType.String,
		defaultValue: (n8nNode.parameters.documentId as Record<string, unknown>)?.value ?? "",
	});
	node.pins[sheetIdPin.id] = sheetIdPin;

	const rangePin = createPin({
		name: "range",
		friendlyName: "Range",
		description: "A1 notation range",
		pinType: IPinType.Input,
		dataType: IVariableType.String,
		defaultValue: n8nNode.parameters.range ?? "",
	});
	node.pins[rangePin.id] = rangePin;

	const valueRenderPin = createPin({
		name: "value_render",
		friendlyName: "Value Render",
		description: "How values should be rendered",
		pinType: IPinType.Input,
		dataType: IVariableType.String,
		defaultValue: "FORMATTED_VALUE",
		options: {
			valid_values: ["FORMATTED_VALUE", "UNFORMATTED_VALUE", "FORMULA"],
		},
	});
	node.pins[valueRenderPin.id] = valueRenderPin;

	const execOutPin = createPin({
		name: "exec_out",
		friendlyName: "Success",
		description: "Execution on success",
		pinType: IPinType.Output,
		dataType: IVariableType.Execution,
	});
	node.pins[execOutPin.id] = execOutPin;

	const errorExec = createPin({
		name: "error",
		friendlyName: "Error",
		description: "Execution on error",
		pinType: IPinType.Output,
		dataType: IVariableType.Execution,
	});
	node.pins[errorExec.id] = errorExec;

	const valuesPin = createPin({
		name: "values",
		friendlyName: "Values",
		description: "2D array of cell values",
		pinType: IPinType.Output,
		dataType: IVariableType.Generic,
		valueType: IValueType.Array,
	});
	node.pins[valuesPin.id] = valuesPin;

	const rowCountPin = createPin({
		name: "row_count",
		friendlyName: "Row Count",
		description: "Number of rows returned",
		pinType: IPinType.Output,
		dataType: IVariableType.Integer,
	});
	node.pins[rowCountPin.id] = rowCountPin;

	const errorMsg = createPin({
		name: "error_message",
		friendlyName: "Error Message",
		description: "Error message",
		pinType: IPinType.Output,
		dataType: IVariableType.String,
	});
	node.pins[errorMsg.id] = errorMsg;

	// Compose: create a Google provider node and connect it
	const [nodeX, nodeY] = node.coordinates ?? [0, 0];
	const providerNode = createNode({
		name: "data_google_provider",
		friendlyName: "Google",
		description: "Authenticate with Google",
		category: "Data/Google",
		x: nodeX - 300,
		y: nodeY,
	});

	const providerOut = createPin({
		name: "provider",
		friendlyName: "Provider",
		description: "Google provider with authentication",
		pinType: IPinType.Output,
		dataType: IVariableType.Struct,
	});
	providerNode.pins[providerOut.id] = providerOut;

	addNodeToBoard(board, providerNode);
	connectPins(providerNode, providerOut, node, providerPin);
}

function setupCodePins(
	node: INode,
	n8nNode: N8nNode,
	diag: TranslationDiagnostic[],
): void {
	const codePin = createPin({
		name: "code",
		friendlyName: "Code",
		description: "Python code to execute",
		pinType: IPinType.Input,
		dataType: IVariableType.String,
		defaultValue: n8nNode.parameters.jsCode ?? "",
	});
	node.pins[codePin.id] = codePin;

	const inputsPin = createPin({
		name: "inputs",
		friendlyName: "Inputs",
		description: "Arbitrary JSON/Struct data exposed as the `inputs` dict inside Python",
		pinType: IPinType.Input,
		dataType: IVariableType.Struct,
	});
	node.pins[inputsPin.id] = inputsPin;

	const packagesPin = createPin({
		name: "packages",
		friendlyName: "Packages",
		description: "micropip package names to install before execution",
		pinType: IPinType.Input,
		dataType: IVariableType.String,
		valueType: IValueType.Array,
		defaultValue: [],
	});
	node.pins[packagesPin.id] = packagesPin;

	const timeoutPin = createPin({
		name: "timeout_secs",
		friendlyName: "Timeout (s)",
		description: "Hard execution time limit in seconds",
		pinType: IPinType.Input,
		dataType: IVariableType.Float,
		defaultValue: 30.0,
	});
	node.pins[timeoutPin.id] = timeoutPin;

	const resultPin = createPin({
		name: "result",
		friendlyName: "Result",
		description: "Contents of the Python `outputs` dict after execution",
		pinType: IPinType.Output,
		dataType: IVariableType.Struct,
	});
	node.pins[resultPin.id] = resultPin;

	const stdoutPin = createPin({
		name: "stdout",
		friendlyName: "Stdout",
		description: "Captured standard output from the Python code",
		pinType: IPinType.Output,
		dataType: IVariableType.String,
	});
	node.pins[stdoutPin.id] = stdoutPin;

	const errorExec = createPin({
		name: "exec_error",
		friendlyName: "Error",
		description: "Activated when execution raises an unhandled exception or times out",
		pinType: IPinType.Output,
		dataType: IVariableType.Execution,
	});
	node.pins[errorExec.id] = errorExec;

	const errorMsgPin = createPin({
		name: "error_msg",
		friendlyName: "Error Message",
		description: "Full traceback / error message when execution fails",
		pinType: IPinType.Output,
		dataType: IVariableType.String,
	});
	node.pins[errorMsgPin.id] = errorMsgPin;

	diag.push({
		level: "warn",
		nodeId: n8nNode.id,
		nodeName: n8nNode.name,
		message:
			"n8n Code node contains JavaScript; manual conversion to Python is required.",
	});
}

function setupEventPins(
	node: INode,
	_n8nNode: N8nNode,
	_diag: TranslationDiagnostic[],
): void {
	const execOut = createPin({
		name: "exec_out",
		friendlyName: "▶",
		description: "Starting an event",
		pinType: IPinType.Output,
		dataType: IVariableType.Execution,
	});
	node.pins[execOut.id] = execOut;
}

function setupGenericEventPins(
	node: INode,
	_n8nNode: N8nNode,
	_diag: TranslationDiagnostic[],
): void {
	const execOut = createPin({
		name: "exec_out",
		friendlyName: "▶",
		description: "Starting an event",
		pinType: IPinType.Output,
		dataType: IVariableType.Execution,
	});
	node.pins[execOut.id] = execOut;

	const payloadPin = createPin({
		name: "payload",
		friendlyName: "Payload",
		description: "The payload of the event",
		pinType: IPinType.Output,
		dataType: IVariableType.Struct,
	});
	node.pins[payloadPin.id] = payloadPin;
}

function setupTelegramPins(
	node: INode,
	n8nNode: N8nNode,
	_diag: TranslationDiagnostic[],
	board: IBoard,
): void {
	const sessionPin = createPin({
		name: "session",
		friendlyName: "Session",
		description: "Telegram session",
		pinType: IPinType.Input,
		dataType: IVariableType.Struct,
	});
	node.pins[sessionPin.id] = sessionPin;

	const messagePin = createPin({
		name: "message",
		friendlyName: "Message",
		description: "Message text",
		pinType: IPinType.Input,
		dataType: IVariableType.String,
		defaultValue: n8nNode.parameters.text ?? "",
	});
	node.pins[messagePin.id] = messagePin;

	const replyToPin = createPin({
		name: "reply_to",
		friendlyName: "Reply To",
		description: "Message ID to reply to",
		pinType: IPinType.Input,
		dataType: IVariableType.String,
		defaultValue: "",
	});
	node.pins[replyToPin.id] = replyToPin;

	const disableNotifPin = createPin({
		name: "disable_notification",
		friendlyName: "Silent",
		description: "Send silently without notification",
		pinType: IPinType.Input,
		dataType: IVariableType.Boolean,
		defaultValue: false,
	});
	node.pins[disableNotifPin.id] = disableNotifPin;

	const sentMsgPin = createPin({
		name: "sent_message",
		friendlyName: "Sent Message",
		description: "The sent message object",
		pinType: IPinType.Output,
		dataType: IVariableType.Struct,
	});
	node.pins[sentMsgPin.id] = sentMsgPin;

	// Compose: create a telegram_to_session node and wire it
	const [nodeX, nodeY] = node.coordinates ?? [0, 0];
	const toSession = createNode({
		name: "telegram_to_session",
		friendlyName: "Telegram Session",
		description: "Creates a Telegram session from credentials",
		category: "Web/Telegram",
		x: nodeX - 300,
		y: nodeY,
	});

	const localSessionIn = createPin({
		name: "local_session",
		friendlyName: "Local Session",
		description: "Bot token or credential struct",
		pinType: IPinType.Input,
		dataType: IVariableType.Struct,
	});
	toSession.pins[localSessionIn.id] = localSessionIn;

	const sessionOut = createPin({
		name: "session",
		friendlyName: "Session",
		description: "Active Telegram session",
		pinType: IPinType.Output,
		dataType: IVariableType.Struct,
	});
	toSession.pins[sessionOut.id] = sessionOut;

	addNodeToBoard(board, toSession);
	connectPins(toSession, sessionOut, node, sessionPin);
}

function setupDiscordPins(
	node: INode,
	n8nNode: N8nNode,
	_diag: TranslationDiagnostic[],
	board: IBoard,
): void {
	const sessionPin = createPin({
		name: "session",
		friendlyName: "Session",
		description: "Discord session",
		pinType: IPinType.Input,
		dataType: IVariableType.Struct,
	});
	node.pins[sessionPin.id] = sessionPin;

	const contentPin = createPin({
		name: "content",
		friendlyName: "Content",
		description: "Message content",
		pinType: IPinType.Input,
		dataType: IVariableType.String,
		defaultValue: n8nNode.parameters.content ?? "",
	});
	node.pins[contentPin.id] = contentPin;

	const channelIdPin = createPin({
		name: "channel_id",
		friendlyName: "Channel ID",
		description: "Target channel id",
		pinType: IPinType.Input,
		dataType: IVariableType.String,
		defaultValue: n8nNode.parameters.channelId ?? "",
	});
	node.pins[channelIdPin.id] = channelIdPin;

	const replyToPin = createPin({
		name: "reply_to",
		friendlyName: "Reply To",
		description: "Message ID to reply to",
		pinType: IPinType.Input,
		dataType: IVariableType.String,
		defaultValue: "",
	});
	node.pins[replyToPin.id] = replyToPin;

	const messageIdOut = createPin({
		name: "message_id",
		friendlyName: "Message ID",
		description: "ID of the sent message",
		pinType: IPinType.Output,
		dataType: IVariableType.String,
	});
	node.pins[messageIdOut.id] = messageIdOut;

	// Compose: create a discord_to_session node and wire it
	const [nodeX, nodeY] = node.coordinates ?? [0, 0];
	const toSession = createNode({
		name: "discord_to_session",
		friendlyName: "Discord Session",
		description: "Creates a Discord session from credentials",
		category: "Web/Discord",
		x: nodeX - 300,
		y: nodeY,
	});

	const localSessionIn = createPin({
		name: "local_session",
		friendlyName: "Local Session",
		description: "Bot token or credential struct",
		pinType: IPinType.Input,
		dataType: IVariableType.Struct,
	});
	toSession.pins[localSessionIn.id] = localSessionIn;

	const sessionOut = createPin({
		name: "session",
		friendlyName: "Session",
		description: "Active Discord session",
		pinType: IPinType.Output,
		dataType: IVariableType.Struct,
	});
	toSession.pins[sessionOut.id] = sessionOut;

	addNodeToBoard(board, toSession);
	connectPins(toSession, sessionOut, node, sessionPin);
}

const N8N_AGENT_TYPES = new Set([
	"@n8n/n8n-nodes-langchain.agent",
	"@n8n/n8n-nodes-langchain.chainLlm",
	"@n8n/n8n-nodes-langchain.chainSummarization",
	"@n8n/n8n-nodes-langchain.chainRetrievalQa",
]);

function setupModelBuilderPins(
	node: INode,
	n8nNode: N8nNode,
	_diag: TranslationDiagnostic[],
): void {
	const modelOut = createPin({
		name: "model",
		friendlyName: "Model",
		description: "The built LLM model",
		pinType: IPinType.Output,
		dataType: IVariableType.Struct,
	});
	node.pins[modelOut.id] = modelOut;

	const modelName = n8nNode.parameters?.modelName ?? n8nNode.parameters?.model;
	if (modelName) {
		const modelIdPin = createPin({
			name: "model_id",
			friendlyName: "Model ID",
			pinType: IPinType.Input,
			dataType: IVariableType.String,
			defaultValue: String(modelName),
		});
		node.pins[modelIdPin.id] = modelIdPin;
	}
}

// ── Mapping data helpers ────────────────────────────────────

function getNestedValue(obj: unknown, path: string): unknown {
	const parts = path.split(".");
	let current: unknown = obj;
	for (const part of parts) {
		if (current == null || typeof current !== "object") return undefined;
		current = (current as Record<string, unknown>)[part];
	}
	return current;
}

function extractParams(
	n8nNode: N8nNode,
	paramDefs: Record<string, string | ParameterRule>,
): Record<string, unknown> {
	const result: Record<string, unknown> = {};
	for (const [key, rule] of Object.entries(paramDefs)) {
		const r: ParameterRule =
			typeof rule === "string" ? { path: rule } : rule;
		let value = getNestedValue(n8nNode, r.path);
		if (
			(value === undefined || value === null || value === "") &&
			r.fallback
		) {
			value = getNestedValue(n8nNode, r.fallback);
		}
		if (value === undefined || value === null) value = r.default;
		if (r.transform === "uppercase" && typeof value === "string")
			value = value.toUpperCase();
		if (r.transform === "lowercase" && typeof value === "string")
			value = value.toLowerCase();
		if (r.transform === "number" && value !== undefined)
			value = Number(value);
		result[key] = value;
	}
	return result;
}

function resolveDefault(
	value: unknown,
	params: Record<string, unknown>,
): unknown {
	if (typeof value === "string" && value.startsWith("$")) {
		return params[value.slice(1)];
	}
	return value;
}

function emitMappingWarnings(
	diag: TranslationDiagnostic[],
	n8nNode: N8nNode,
	warnings?: string[],
): void {
	if (!warnings) return;
	for (const w of warnings) {
		diag.push({
			level: "warn",
			nodeId: n8nNode.id,
			nodeName: n8nNode.name,
			message: w,
		});
	}
}

function buildConfigure(
	n8nDef: { parameters?: Record<string, string | ParameterRule>; warnings?: string[] },
	flowDef: FlowDirectDef | FlowLayerDef,
): NodeMapping["configure"] {
	if (flowDef.mode === "direct") {
		return (node, n8nNode, diag) => {
			const params = extractParams(n8nNode, n8nDef.parameters ?? {});
			if (flowDef.defaults) {
				for (const [pin, value] of Object.entries(flowDef.defaults)) {
					const resolved = resolveDefault(value, params);
					if (resolved !== undefined) setPinDefault(node, pin, resolved);
				}
			}
			emitMappingWarnings(diag, n8nNode, n8nDef.warnings);
		};
	}

	const layerDef = flowDef;
	return (node, n8nNode, diag, board, ci) => {
		const params = extractParams(n8nNode, n8nDef.parameters ?? {});
		const nodeMap = new Map<string, INode>();

		for (const nodeDef of layerDef.nodes) {
			if (nodeDef.primary) {
				nodeMap.set(nodeDef.id, node);
				continue;
			}
			const [x, y] = node.coordinates ?? [0, 0];
			const cloned = cloneNodeFromCatalog(ci, nodeDef.catalog, {
				friendlyName: nodeDef.nameSuffix
					? `${n8nNode.name} ${nodeDef.nameSuffix}`
					: nodeDef.catalog,
				x: x + (nodeDef.offset?.[0] ?? 0),
				y: y + (nodeDef.offset?.[1] ?? 0),
			});
			if (cloned) {
				addNodeToBoard(board, cloned);
				nodeMap.set(nodeDef.id, cloned);
			}
		}

		for (const [from, to] of layerDef.connections ?? []) {
			const [fromId, fromPin] = from.split(":");
			const [toId, toPin] = to.split(":");
			const fromNode = nodeMap.get(fromId);
			const toNode = nodeMap.get(toId);
			if (fromNode && toNode) {
				const outPin = findPinByName(fromNode, fromPin, IPinType.Output);
				const inPin = findPinByName(toNode, toPin, IPinType.Input);
				if (outPin && inPin) connectPins(fromNode, outPin, toNode, inPin);
			}
		}

		if (layerDef.defaults) {
			for (const [key, value] of Object.entries(layerDef.defaults)) {
				const [nodeId, pin] = key.split(":");
				const targetNode = nodeMap.get(nodeId);
				if (targetNode && pin) {
					const resolved = resolveDefault(value, params);
					if (resolved !== undefined)
						setPinDefault(targetNode, pin, resolved);
				}
			}
		}

		emitMappingWarnings(diag, n8nNode, n8nDef.warnings);
	};
}

function buildFallbackConfigure(
	n8nDef: { parameters?: Record<string, string | ParameterRule> },
	flowDef: FlowDirectDef | FlowLayerDef,
): NodeMapping["fallbackConfigure"] {
	if (flowDef.mode !== "direct") return undefined;

	return (node, n8nNode) => {
		const params = extractParams(n8nNode, n8nDef.parameters ?? {});
		if (!flowDef.defaults) return;

		for (const [pin, value] of Object.entries(flowDef.defaults)) {
			const resolved = resolveDefault(value, params);
			if (resolved !== undefined) {
				setPinDefault(node, pin, resolved);
			}
		}
	};
}

// ── Build NODE_REGISTRY from mapping definitions ────────────

const MAPPING_CATEGORIES: Record<string, string> = {
	manual_trigger: "Events",
	schedule_trigger: "Events",
	webhook: "Events",
	chat_trigger: "Events",
	if: "Control",
	switch: "Control",
	split_in_batches: "Control",
	wait: "Control",
	no_op: "Control",
	merge: "Control",
	http_request: "Web/Api",
	respond_to_webhook: "Logging",
	set: "Structs/Fields",
	code: "Code",
	gmail: "Mail/Smtp",
	google_sheets: "Data/Google",
	telegram: "Web/Telegram",
	discord: "Web/Discord",
};

const BUILTIN_LEGACY_SETUP: Record<string, NodeMapping["setupPins"]> = {
	"n8n-nodes-base.manualTrigger": setupEventPins,
	"n8n-nodes-base.scheduleTrigger": (node, n8nNode, diag) => {
		setupEventPins(node, n8nNode, diag);
		emitMappingWarnings(diag, n8nNode, [
			"Schedule trigger mapped to simple event. Flow-Like does not have a built-in cron trigger; use an external scheduler to invoke this flow.",
		]);
	},
	"n8n-nodes-base.webhook": (node, n8nNode, diag) => {
		setupGenericEventPins(node, n8nNode, diag);
		emitMappingWarnings(diag, n8nNode, [
			"Webhook trigger mapped to generic event. Use the event's payload pin for incoming request data.",
		]);
	},
	"@n8n/n8n-nodes-langchain.chatTrigger": setupEventPins,
	"n8n-nodes-base.if": (node, n8nNode, diag) => {
		const execIn = createPin({
			name: "exec_in",
			friendlyName: "▶",
			description: "Execution input",
			pinType: IPinType.Input,
			dataType: IVariableType.Execution,
		});
		node.pins[execIn.id] = execIn;
		setupBranchPins(node, n8nNode, diag);
	},
	"n8n-nodes-base.switch": (node, n8nNode, diag) => {
		const execIn = createPin({
			name: "exec_in",
			friendlyName: "▶",
			description: "Execution input",
			pinType: IPinType.Input,
			dataType: IVariableType.Execution,
		});
		node.pins[execIn.id] = execIn;
		setupBranchPins(node, n8nNode, diag);
		emitMappingWarnings(diag, n8nNode, [
			"Switch mapped to branch (boolean). For multi-way routing, chain multiple branch nodes.",
		]);
	},
	"n8n-nodes-base.splitInBatches": (node, n8nNode, diag) => {
		const execIn = createPin({
			name: "exec_in",
			friendlyName: "▶",
			description: "Execution input",
			pinType: IPinType.Input,
			dataType: IVariableType.Execution,
		});
		node.pins[execIn.id] = execIn;
		setupForEachPins(node, n8nNode, diag);
	},
	"n8n-nodes-base.wait": (node, n8nNode) => {
		const waitMs =
			typeof n8nNode.parameters.amount === "number"
				? n8nNode.parameters.amount * 1000
				: 1000;
		const timePin = createPin({
			name: "time",
			friendlyName: "Time (ms)",
			description: "Delay duration in milliseconds",
			pinType: IPinType.Input,
			dataType: IVariableType.Float,
			defaultValue: waitMs,
		});
		node.pins[timePin.id] = timePin;
	},
	"n8n-nodes-base.httpRequest": (node, n8nNode, diag, board) => {
		const execIn = createPin({
			name: "exec_in",
			friendlyName: "▶",
			description: "Execution input",
			pinType: IPinType.Input,
			dataType: IVariableType.Execution,
		});
		node.pins[execIn.id] = execIn;
		const execSuccess = createPin({
			name: "exec_success",
			friendlyName: "Success",
			description: "Execution if the request succeeds",
			pinType: IPinType.Output,
			dataType: IVariableType.Execution,
		});
		node.pins[execSuccess.id] = execSuccess;
		setupHttpFetchPins(node, n8nNode, diag, board);
	},
	"n8n-nodes-base.respondToWebhook": (node, n8nNode, diag) => {
		const msgPin = createPin({
			name: "message",
			friendlyName: "Message",
			description: "Webhook response placeholder",
			pinType: IPinType.Input,
			dataType: IVariableType.Generic,
			defaultValue: "Webhook response (placeholder)",
		});
		node.pins[msgPin.id] = msgPin;
		const toastPin = createPin({
			name: "toast",
			friendlyName: "Toast",
			description: "Show in-app toast notification",
			pinType: IPinType.Input,
			dataType: IVariableType.Boolean,
			defaultValue: false,
		});
		node.pins[toastPin.id] = toastPin;
		emitMappingWarnings(diag, n8nNode, [
			"Respond to Webhook has no direct equivalent. Mapped to log_info as placeholder.",
		]);
	},
	"n8n-nodes-base.set": setupStructSetPins,
	"n8n-nodes-base.code": setupCodePins,
	"n8n-nodes-base.gmail": (node, n8nNode, diag, board) => {
		setupEmailPins(node, n8nNode, diag, board);
		emitMappingWarnings(diag, n8nNode, [
			"Gmail mapped to SMTP send. The SMTP connect node needs host/port/credentials configured.",
		]);
	},
	"n8n-nodes-base.googleSheets": (node, n8nNode, diag, board) => {
		const op = n8nNode.parameters.operation;
		if (op === "append" || op === "appendOrUpdate")
			node.name = "data_google_sheets_append_rows";
		else if (op === "update")
			node.name = "data_google_sheets_write_range";
		const execIn = createPin({
			name: "exec_in",
			friendlyName: "▶",
			description: "Execution input",
			pinType: IPinType.Input,
			dataType: IVariableType.Execution,
		});
		node.pins[execIn.id] = execIn;
		setupGoogleSheetsPins(node, n8nNode, diag, board);
	},
	"n8n-nodes-base.telegram": setupTelegramPins,
	"n8n-nodes-base.discord": setupDiscordPins,
};

function getMappingCategory(mapping: ResolvedN8nMappingDef): string {
	return (
		mapping.category ??
		MAPPING_CATEGORIES[mapping.name] ??
		(mapping.name.startsWith("model_") ? "AI/Models" : "General")
	);
}

function getBuiltInMappingTypes(
	resolvedMappings: ResolvedN8nMappingDef[],
): Set<string> {
	return new Set(
		resolvedMappings
			.filter((mapping) => mapping.source === "built-in")
			.map((mapping) => mapping.n8n.type),
	);
}

function applyBuiltInSpecialCases(
	registry: Record<string, NodeMapping>,
	resolvedMappings: ResolvedN8nMappingDef[],
): void {
	const builtInTypes = getBuiltInMappingTypes(resolvedMappings);
	const applyIfBuiltIn = (
		type: string,
		updater: (entry: NodeMapping) => void,
	) => {
		if (!builtInTypes.has(type)) return;
		const entry = registry[type];
		if (entry) updater(entry);
	};

	applyIfBuiltIn("n8n-nodes-base.wait", (entry) => {
		const baseConfigure = entry.configure;
		entry.configure = (node, n8nNode, diag, board, ci) => {
			baseConfigure?.(node, n8nNode, diag, board, ci);
			const amount =
				typeof n8nNode.parameters.amount === "number"
					? n8nNode.parameters.amount
					: 1;
			setPinDefault(node, "time", amount * 1000);
		};
	});

	applyIfBuiltIn("n8n-nodes-base.googleSheets", (entry) => {
		const baseConfigure = entry.configure;
		entry.configure = (node, n8nNode, diag, board, ci) => {
			const op = n8nNode.parameters.operation;
			if (op === "append" || op === "appendOrUpdate") {
				node.name = "data_google_sheets_append_rows";
			} else if (op === "update") {
				node.name = "data_google_sheets_write_range";
			}
			baseConfigure?.(node, n8nNode, diag, board, ci);
		};
	});

	applyIfBuiltIn("n8n-nodes-base.set", (entry) => {
		const baseConfigure = entry.configure;
		entry.configure = (node, n8nNode, diag, board, ci) => {
			baseConfigure?.(node, n8nNode, diag, board, ci);
			const assignments = n8nNode.parameters.assignments as
				| { assignments?: Array<{ name: string }> }
				| undefined;
			if (assignments?.assignments?.length) {
				const fieldNames = assignments.assignments
					.map((a) => a.name)
					.join(", ");
				node.comment = `Sets fields: ${fieldNames}. Chain multiple struct_set nodes for each field.`;
			}
		};
	});

	applyIfBuiltIn("n8n-nodes-base.httpRequest", (entry) => {
		const baseConfigure = entry.configure;
		entry.configure = (node, n8nNode, diag, board, ci) => {
			baseConfigure?.(node, n8nNode, diag, board, ci);
			const url =
				typeof n8nNode.parameters.url === "string"
					? n8nNode.parameters.url
					: "";
			const method =
				typeof n8nNode.parameters.method === "string"
					? String(n8nNode.parameters.method).toUpperCase()
					: "GET";
			if (url) node.comment = `HTTP ${method} ${url}`;
		};
	});
}

function applyLegacySetup(
	registry: Record<string, NodeMapping>,
	resolvedMappings: ResolvedN8nMappingDef[],
): void {
	for (const [type, setup] of Object.entries(BUILTIN_LEGACY_SETUP)) {
		if (registry[type]) {
			registry[type].setupPins = setup;
		}
	}

	for (const mapping of resolvedMappings) {
		if (!mapping.name.startsWith("model_")) {
			continue;
		}

		if (registry[mapping.n8n.type]) {
			registry[mapping.n8n.type].setupPins = setupModelBuilderPins;
		}
	}
}

function createNodeRegistry(
	mappingOverrides: N8nManualMappingOverrides = N8N_MAPPING_OVERRIDES,
): Record<string, NodeMapping> {
	const resolvedMappings = resolveN8nMappingDefs(mappingOverrides);
	const registry: Record<string, NodeMapping> = {};

	for (const mapping of resolvedMappings) {
		const { n8n, flow } = mapping;
		const isLayer = flow.mode === "layer";
		const primaryCatalog = isLayer
			? (flow as FlowLayerDef).nodes.find((node) => node.primary)?.catalog ??
				(flow as FlowLayerDef).nodes[0].catalog
			: (flow as FlowDirectDef).catalog;

		registry[n8n.type] = {
			catalog: primaryCatalog,
			category: getMappingCategory(mapping),
			type: isLayer ? "composition" : "direct",
			isEvent: n8n.isEvent,
			skipDefaultExecPins: flow.skipExecPins,
			fallbackConfigure: buildFallbackConfigure(n8n, flow),
			configure: buildConfigure(n8n, flow),
		};
	}

	applyBuiltInSpecialCases(registry, resolvedMappings);
	applyLegacySetup(registry, resolvedMappings);

	return registry;
}

export function translateN8n(
	workflow: N8nWorkflow,
	catalog?: INode[],
	options: TranslateN8nOptions = {},
): TranslationResult {
	const diagnostics: TranslationDiagnostic[] = [];
	const board = createEmptyBoard(
		workflow.name || "Imported n8n Workflow",
		`Imported from n8n workflow${workflow.id ? ` (${workflow.id})` : ""}`,
	);
	const catalogIndex = catalog ? buildCatalogIndex(catalog) : undefined;
	const nodeRegistry = createNodeRegistry(options.mappingOverrides);

	info(diagnostics, `Starting translation of n8n workflow: ${workflow.name}`);
	if (catalogIndex) {
		info(diagnostics, `Using catalog with ${catalogIndex.size} node definitions`);
	}
	if (Object.keys(options.mappingOverrides ?? {}).length > 0) {
		info(
			diagnostics,
			`Applied ${Object.keys(options.mappingOverrides ?? {}).length} manual n8n mapping override(s)`,
		);
	}

	// Build name→node lookup for connections
	const nameToN8nNode = new Map<string, N8nNode>();
	for (const n8nNode of workflow.nodes) {
		nameToN8nNode.set(n8nNode.name, n8nNode);
	}

	// Map agent n8n name → internal agent_from_model node (for ai_languageModel routing)
	const aiFromModelNodes = new Map<string, INode>();

	// Pre-scan: find AI sub-nodes attached via ai_* connections (output parsers, tools, memory, etc.)
	// These should not create standalone TODO layers — they're absorbed into the agent composition
	const aiSubNodes = new Set<string>();
	for (const [sourceName, connectionTypes] of Object.entries(workflow.connections)) {
		for (const connType of Object.keys(connectionTypes)) {
			if (connType.startsWith("ai_") && connType !== "ai_languageModel") {
				aiSubNodes.add(sourceName);
			}
		}
	}

	// Phase 1: Translate nodes
	const n8nIdToFlowNode = new Map<string, INode>();
	const n8nNameToFlowNode = new Map<string, INode>();
	let directCount = 0;
	let composedCount = 0;
	let todoCount = 0;

	for (const n8nNode of workflow.nodes) {
		if (n8nNode.disabled) {
			info(
				diagnostics,
				`Skipping disabled node: ${n8nNode.name}`,
				n8nNode.id,
				n8nNode.name,
			);
			continue;
		}

		if (n8nNode.type === "n8n-nodes-base.stickyNote") {
			const params = n8nNode.parameters;
			const content =
				typeof params.content === "string" ? params.content : n8nNode.name;
			const stickyColors: Record<number, string> = {
				1: "#FFF9B1",
				2: "#D4EDBC",
				3: "#D0E8FF",
				4: "#FFD6E0",
				5: "#F3E2FF",
				6: "#FFE0B2",
			};
			const colorIdx =
				typeof params.color === "number" ? params.color : undefined;
			const color = colorIdx ? (stickyColors[colorIdx] ?? "#FFF9B1") : "#FFF9B1";
			const width =
				typeof params.width === "number" ? params.width : undefined;
			const height =
				typeof params.height === "number" ? params.height : undefined;

			const ts = now();
			addCommentToBoard(board, {
				id: n8nNode.id,
				content,
				comment_type: ICommentType.Text,
				coordinates: [n8nNode.position[0], n8nNode.position[1], 0],
				timestamp: ts,
				author: "n8n",
				color,
				width: width ?? null,
				height: height ?? null,
				z_index: -1,
				is_locked: null,
				layer: null,
				hash: null,
			});
			info(
				diagnostics,
				"Sticky note converted to board comment",
				n8nNode.id,
				n8nNode.name,
			);
			continue;
		}

		// AI Agent/Chain nodes → composition with builder chain
		if (N8N_AGENT_TYPES.has(n8nNode.type) && catalogIndex?.has("agent_invoke")) {
			const nodesBefore = new Set(Object.keys(board.nodes));
			const cloned = cloneNodeFromCatalog(catalogIndex, "agent_invoke", {
				friendlyName: n8nNode.name,
				description: `Imported from n8n: ${n8nNode.type}`,
				x: n8nNode.position[0],
				y: n8nNode.position[1],
				comment: `Composed from n8n ${n8nNode.type}. Review pin connections.`,
			});

			if (cloned) {
				const systemPrompt = String(
					n8nNode.parameters?.systemMessage ?? n8nNode.parameters?.text ?? "",
				);

				// Build chain: [agent_from_model] → [set_system_prompt?] → [agent_invoke]
				let feedsInto: INode = cloned;
				let feedsIntoPinName = "agent";

				if (systemPrompt) {
					const setPrompt = composeCompanion(
						board, catalogIndex, "agent_set_system_prompt", cloned,
						"agent_out", "agent", `${n8nNode.name} (Prompt)`,
						(sp) => setPinDefault(sp, "system_prompt", systemPrompt),
					);
					if (setPrompt) {
						feedsInto = setPrompt;
						feedsIntoPinName = "agent_in";
					}
				}

				const fromModel = composeCompanion(
					board, catalogIndex, "agent_from_model", feedsInto,
					"agent_out", feedsIntoPinName, `${n8nNode.name} (Builder)`,
				);

				if (fromModel) {
					aiFromModelNodes.set(n8nNode.name, fromModel);
				}

				// Group into composition layer
				const newNodeIds = Object.keys(board.nodes).filter(
					(id) => !nodesBefore.has(id),
				);
				if (newNodeIds.length > 0) {
					const layer = createCompositionLayer({
						name: n8nNode.name,
						x: n8nNode.position[0],
						y: n8nNode.position[1],
					});
					cloned.layer = layer.id;
					for (const id of newNodeIds) {
						board.nodes[id].layer = layer.id;
					}
					addLayerToBoard(board, layer);
				}

				addNodeToBoard(board, cloned);
				n8nIdToFlowNode.set(n8nNode.id, cloned);
				n8nNameToFlowNode.set(n8nNode.name, cloned);
				composedCount++;
				info(
					diagnostics,
					`AI agent mapped: ${n8nNode.type} → agent_invoke composition`,
					n8nNode.id,
					n8nNode.name,
				);
				continue;
			}
		}

		const mapping = nodeRegistry[n8nNode.type];

		if (mapping) {
			let flowNode: INode;

			// Catalog path: clone from real catalog (pins are always correct)
			if (catalogIndex && mapping.configure && catalogIndex.has(mapping.catalog)) {
				const nodesBefore = new Set(Object.keys(board.nodes));

				const cloned = cloneNodeFromCatalog(catalogIndex, mapping.catalog, {
					friendlyName: n8nNode.name,
					description: `Imported from n8n: ${n8nNode.type}`,
					x: n8nNode.position[0],
					y: n8nNode.position[1],
					comment:
						mapping.type === "composition"
							? `Composed from n8n ${n8nNode.type}. Review pin connections.`
							: undefined,
					start: mapping.isEvent ? true : undefined,
				});

				if (cloned) {
					flowNode = cloned;
					mapping.configure(flowNode, n8nNode, diagnostics, board, catalogIndex);
				} else {
					// Should not happen since we checked catalogIndex.has() above
					flowNode = createNodeFallback(mapping, n8nNode, diagnostics, board);
				}

				// Composition nodes: group all generated nodes into a named layer
				if (mapping.type === "composition") {
					const newNodeIds = Object.keys(board.nodes).filter(
						(id) => !nodesBefore.has(id),
					);
					if (newNodeIds.length > 0) {
						const layer = createCompositionLayer({
							name: n8nNode.name,
							x: n8nNode.position[0],
							y: n8nNode.position[1],
						});
						flowNode.layer = layer.id;
						for (const id of newNodeIds) {
							board.nodes[id].layer = layer.id;
						}
						addLayerToBoard(board, layer);
					}
				}
			} else {
				// Legacy fallback: manually create pins (used in tests without catalog)
				flowNode = createNodeFallback(mapping, n8nNode, diagnostics, board);
			}

			addNodeToBoard(board, flowNode);
			n8nIdToFlowNode.set(n8nNode.id, flowNode);
			n8nNameToFlowNode.set(n8nNode.name, flowNode);

			if (mapping.type === "direct") {
				directCount++;
				info(
					diagnostics,
					`Direct mapping: ${n8nNode.type} → ${mapping.catalog}`,
					n8nNode.id,
					n8nNode.name,
				);
			} else {
				composedCount++;
				warn(
					diagnostics,
					`Composition needed: ${n8nNode.type} → ${mapping.catalog}. Review and adjust.`,
					n8nNode.id,
					n8nNode.name,
				);
			}
		} else {
			// AI sub-nodes (output parsers, tools, memory) connected via ai_* → skip, they're absorbed
			if (aiSubNodes.has(n8nNode.name)) {
				info(
					diagnostics,
					`Skipping AI sub-node "${n8nNode.name}" (${n8nNode.type}) — attached to agent via ai_* connection`,
					n8nNode.id,
					n8nNode.name,
				);
				continue;
			}

			// Unknown node → create TODO layer
			todoCount++;
			const paramKeys = Object.keys(n8nNode.parameters);
			const paramSummary = paramKeys.length > 0
				? `Parameters: ${paramKeys.join(", ")}`
				: "";
			const layer = createTodoLayer({
				name: `TODO: ${n8nNode.name}`,
				comment: `n8n node "${n8nNode.type}" has no flow-like equivalent. Implement as WASM node or compose from existing nodes.${paramSummary ? `\n${paramSummary}` : ""}`,
				x: n8nNode.position[0],
				y: n8nNode.position[1],
			});

			// Create a placeholder node inside the layer
			const placeholder = createNode({
				name: "todo_placeholder",
				friendlyName: n8nNode.name,
				description: `TODO: Implement n8n node "${n8nNode.type}"`,
				category: "TODO",
				x: n8nNode.position[0],
				y: n8nNode.position[1],
				comment: `Original n8n type: ${n8nNode.type}${paramSummary ? `\n${paramSummary}` : ""}`,
				layer: layer.id,
			});

			addExecPins(placeholder);
			addDefaultParameterPins(placeholder, n8nNode, diagnostics);
			addNodeToBoard(board, placeholder);

			addLayerToBoard(board, layer);
			n8nIdToFlowNode.set(n8nNode.id, placeholder);
			n8nNameToFlowNode.set(n8nNode.name, placeholder);

			diagError(
				diagnostics,
				`No mapping for n8n node type "${n8nNode.type}". Created TODO layer.`,
				n8nNode.id,
				n8nNode.name,
			);
		}
	}

	// Phase 2: Translate connections
	let connectionCount = 0;

	for (const [sourceName, connectionTypes] of Object.entries(
		workflow.connections,
	)) {
		const sourceFlowNode = n8nNameToFlowNode.get(sourceName);
		if (!sourceFlowNode) continue;

		const sourceN8n = nameToN8nNode.get(sourceName);

		for (const [connType, outputs] of Object.entries(connectionTypes)) {
			for (let outputIdx = 0; outputIdx < outputs.length; outputIdx++) {
				const targets = outputs[outputIdx];
				if (!targets) continue;

				for (const target of targets) {
					const targetFlowNode = n8nNameToFlowNode.get(target.node);
					if (!targetFlowNode) {
						warn(
							diagnostics,
							`Connection target "${target.node}" not found (may be disabled)`,
							undefined,
							sourceName,
						);
						continue;
					}

					// Skip loop back-edges: n8n loops by connecting downstream
					// back to splitInBatches, but flow-like for_each loops internally
					const targetN8n = nameToN8nNode.get(target.node);
					if (
						targetN8n?.type === "n8n-nodes-base.splitInBatches" &&
						sourceN8n?.type !== "n8n-nodes-base.splitInBatches"
					) {
						info(
							diagnostics,
							`Skipping loop back-edge from "${sourceName}" to "${target.node}" (for_each handles iteration internally)`,
							undefined,
							sourceName,
						);
						continue;
					}

					if (connType === "main") {
						const sourceExecOut = resolveSourceExecPin(
							sourceFlowNode,
							outputIdx,
						);
						const targetExecIn = findPinByName(
							targetFlowNode,
							"exec_in",
							IPinType.Input,
						);
						if (sourceExecOut && targetExecIn) {
							connectPins(
								sourceFlowNode,
								sourceExecOut,
								targetFlowNode,
								targetExecIn,
							);
							connectionCount++;
						}
					} else if (connType === "ai_languageModel") {
						// Route model output to the agent_from_model inside the composition
						const fromModel = aiFromModelNodes.get(target.node);
						if (fromModel) {
							// Move the model builder into the agent's composition layer
							if (fromModel.layer && sourceFlowNode.layer !== fromModel.layer) {
								sourceFlowNode.layer = fromModel.layer;
								const [fmX, fmY] = fromModel.coordinates ?? [0, 0];
								sourceFlowNode.coordinates = [fmX - 300, fmY, 0];
							}
							const outPin = findPinByName(sourceFlowNode, "model", IPinType.Output);
							const inPin = findPinByName(fromModel, "model", IPinType.Input);
							if (outPin && inPin) {
								connectPins(sourceFlowNode, outPin, fromModel, inPin);
								connectionCount++;
							} else {
								warn(
									diagnostics,
									`Could not wire ai_languageModel from "${sourceName}" to "${target.node}". Pin mismatch.`,
									undefined,
									sourceName,
								);
							}
						} else {
							// Fallback: generic data pin wiring
							const sourceDataOut = findFirstDataOutputPin(sourceFlowNode);
							const targetDataIn = findFirstDataInputPin(targetFlowNode);
							if (sourceDataOut && targetDataIn) {
								connectPins(sourceFlowNode, sourceDataOut, targetFlowNode, targetDataIn);
								connectionCount++;
							}
						}
					} else if (connType === "ai_outputParser") {
						warn(
							diagnostics,
							`Output parser "${sourceName}" attached to "${target.node}". Encode format instructions in the system prompt instead.`,
							undefined,
							sourceName,
						);
					} else if (connType === "ai_tool") {
						warn(
							diagnostics,
							`Tool "${sourceName}" attached to "${target.node}". Register tools manually via agent_register_function_tools.`,
							undefined,
							sourceName,
						);
					} else {
						// Generic data connections
						const sourceDataOut = findFirstDataOutputPin(sourceFlowNode);
						const targetDataIn = findFirstDataInputPin(targetFlowNode);
						if (sourceDataOut && targetDataIn) {
							connectPins(
								sourceFlowNode,
								sourceDataOut,
								targetFlowNode,
								targetDataIn,
							);
							connectionCount++;
						} else {
							warn(
								diagnostics,
								`Could not wire ${connType} connection from "${sourceName}" to "${target.node}". Add pins manually.`,
								undefined,
								sourceName,
							);
						}
					}
				}
			}
		}
	}

	// Phase 2.5: Create bridge pins and compute layer coordinates
	createBridgePinsForBoard(board);
	computeLayerCoordinates(board);

	// Phase 3: Translate credentials to secret variables with Get Variable nodes
	let variableCount = 0;
	const credentialsSeen = new Map<string, string>();
	const AUTH_PIN_NAMES = ["api_key", "token", "password", "secret", "credentials"];

	for (const n8nNode of workflow.nodes) {
		if (!n8nNode.credentials) continue;
		const flowNode = n8nNameToFlowNode.get(n8nNode.name);

		for (const [credType, credRef] of Object.entries(n8nNode.credentials)) {
			const key = `${credType}_${credRef.name}`;

			if (!credentialsSeen.has(key)) {
				const variable = createVariable({
					name: `credential_${credType}`,
					description: `n8n credential: ${credRef.name} (${credType}). Set the actual value.`,
					dataType: IVariableType.String,
					secret: true,
					exposed: true,
					editable: true,
				});
				addVariableToBoard(board, variable);
				credentialsSeen.set(key, variable.id);
				variableCount++;
				info(
					diagnostics,
					`Created secret variable for credential: ${credType}`,
					undefined,
					n8nNode.name,
				);
			}

			const variableId = credentialsSeen.get(key)!;

			if (flowNode) {
				const [fx, fy] = flowNode.coordinates ?? [0, 0];

				let getVarNode: INode | undefined;
				if (catalogIndex?.has("variable_get")) {
					getVarNode = cloneNodeFromCatalog(catalogIndex, "variable_get", {
						friendlyName: `Get ${credRef.name}`,
						x: fx - 300,
						y: fy + 150,
					});
					if (getVarNode) {
						setPinDefault(getVarNode, "var_ref", variableId);
						getVarNode.layer = flowNode.layer ?? undefined;
						addNodeToBoard(board, getVarNode);
					}
				} else {
					getVarNode = createNode({
						name: "variable_get",
						friendlyName: `Get ${credRef.name}`,
						description: `Gets credential: ${credType}`,
						category: "Variable",
						x: fx - 300,
						y: fy + 150,
					});
					const varRefPin = createPin({
						name: "var_ref",
						friendlyName: "Variable Reference",
						description: "The reference to the variable",
						pinType: IPinType.Input,
						dataType: IVariableType.String,
						defaultValue: variableId,
					});
					getVarNode.pins[varRefPin.id] = varRefPin;
					const valuePin = createPin({
						name: "value_ref",
						friendlyName: "Value",
						description: "The value of the variable",
						pinType: IPinType.Output,
						dataType: IVariableType.Generic,
					});
					getVarNode.pins[valuePin.id] = valuePin;
					getVarNode.layer = flowNode.layer ?? undefined;
					addNodeToBoard(board, getVarNode);
				}

				// Wire variable_get output to auth pin on target node
				if (getVarNode) {
					const valueOut = findPinByName(getVarNode, "value_ref", IPinType.Output);
					if (valueOut) {
						let authPin: IPin | undefined;
						for (const name of AUTH_PIN_NAMES) {
							authPin = findPinByName(flowNode, name, IPinType.Input);
							if (authPin) break;
						}
						if (authPin) {
							connectPins(getVarNode, valueOut, flowNode, authPin);
						}
					}
				}
			}
		}
	}

	// Phase 4: Workflow settings
	if (workflow.settings?.timezone) {
		const tzVar = createVariable({
			name: "workflow_timezone",
			description: `n8n workflow timezone: ${workflow.settings.timezone}`,
			dataType: IVariableType.String,
			defaultValue: workflow.settings.timezone,
		});
		addVariableToBoard(board, tzVar);
		variableCount++;
	}

	const hasErrors = diagnostics.some((d) => d.level === "error");
	const status = hasErrors
		? directCount > 0
			? "partial"
			: "error"
		: "success";

	info(
		diagnostics,
		`Translation complete: ${directCount} direct, ${composedCount} composed, ${todoCount} TODO, ${connectionCount} connections, ${variableCount} variables`,
	);

	return {
		format: "n8n",
		status,
		board,
		diagnostics,
		stats: {
			totalNodes: workflow.nodes.filter(
				(n) =>
					!n.disabled && n.type !== "n8n-nodes-base.stickyNote",
			).length,
			directMapped: directCount,
			composed: composedCount,
			todo: todoCount,
			connections: connectionCount,
			variables: variableCount,
		},
	};
}

function createNodeFallback(
	mapping: NodeMapping,
	n8nNode: N8nNode,
	diagnostics: TranslationDiagnostic[],
	board: IBoard,
): INode {
	const flowNode = createNode({
		name: mapping.catalog,
		friendlyName: n8nNode.name,
		description: `Imported from n8n: ${n8nNode.type}`,
		category: mapping.category,
		x: n8nNode.position[0],
		y: n8nNode.position[1],
		comment:
			mapping.type === "composition"
				? `Composed from n8n ${n8nNode.type}. Review pin connections.`
				: undefined,
		start: mapping.isEvent ? true : undefined,
	});

	if (!mapping.skipDefaultExecPins) {
		addExecPins(flowNode);
	}

	if (mapping.setupPins) {
		mapping.setupPins(flowNode, n8nNode, diagnostics, board);
	} else {
		addDefaultParameterPins(flowNode, n8nNode, diagnostics);
	}

	mapping.fallbackConfigure?.(flowNode, n8nNode, diagnostics);

	return flowNode;
}

function resolveSourceExecPin(
	node: INode,
	outputIdx: number,
): IPin | undefined {
	const execOutPins = Object.values(node.pins).filter(
		(p) =>
			p.pin_type === IPinType.Output &&
			p.data_type === IVariableType.Execution,
	);
	if (execOutPins.length === 0) return undefined;

	// For branch nodes: output 0 = "true", output 1 = "false"
	const truePin = execOutPins.find((p) => p.name === "true");
	const falsePin = execOutPins.find((p) => p.name === "false");
	if (truePin && falsePin) {
		return outputIdx === 0 ? truePin : falsePin;
	}

	// For http_fetch: output 0 = "exec_success", output 1 = "exec_error"
	const successPin = execOutPins.find((p) => p.name === "exec_success");
	if (successPin) {
		const errorPin = execOutPins.find((p) => p.name === "exec_error");
		return outputIdx === 0 ? successPin : (errorPin ?? successPin);
	}

	// For for_each: output 0 = "exec_out" (loop body), output 1 = "done"
	const donePin = execOutPins.find((p) => p.name === "done");
	const execOutPin = execOutPins.find((p) => p.name === "exec_out");
	if (donePin && execOutPin) {
		return outputIdx === 0 ? execOutPin : donePin;
	}

	// Default: first exec output pin
	return execOutPins[0];
}

function addDefaultParameterPins(
	node: INode,
	n8nNode: N8nNode,
	diag: TranslationDiagnostic[],
): void {
	for (const [key, value] of Object.entries(n8nNode.parameters)) {
		if (value === undefined || value === null) continue;
		if (typeof value === "object" && !Array.isArray(value)) continue; // Skip complex nested params

		const dataType =
			typeof value === "number"
				? IVariableType.Float
				: typeof value === "boolean"
					? IVariableType.Boolean
					: IVariableType.String;

		const pin = createPin({
			name: `param_${key}`,
			friendlyName: key,
			description: `n8n parameter: ${key}`,
			pinType: IPinType.Input,
			dataType,
			defaultValue: value,
		});
		node.pins[pin.id] = pin;
	}
}

function findFirstDataOutputPin(node: INode): IPin | undefined {
	return Object.values(node.pins).find(
		(p) =>
			p.pin_type === IPinType.Output && p.data_type !== IVariableType.Execution,
	);
}

function findFirstDataInputPin(node: INode): IPin | undefined {
	return Object.values(node.pins).find(
		(p) =>
			p.pin_type === IPinType.Input && p.data_type !== IVariableType.Execution,
	);
}
