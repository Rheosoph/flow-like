package com.flowlike.sdk;

import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;

public final class Types {

    public static final int ABI_VERSION = 1;

    // DataType constants
    public static final String DATA_TYPE_EXEC = "Exec";
    public static final String DATA_TYPE_STRING = "String";
    public static final String DATA_TYPE_I64 = "I64";
    public static final String DATA_TYPE_F64 = "F64";
    public static final String DATA_TYPE_BOOL = "Bool";
    public static final String DATA_TYPE_GENERIC = "Generic";
    public static final String DATA_TYPE_BYTES = "Bytes";
    public static final String DATA_TYPE_DATE = "Date";
    public static final String DATA_TYPE_PATH_BUF = "PathBuf";
    public static final String DATA_TYPE_STRUCT = "Struct";

    // ValueType constants
    public static final String VALUE_TYPE_NORMAL = "Normal";
    public static final String VALUE_TYPE_ARRAY = "Array";
    public static final String VALUE_TYPE_HASH_MAP = "HashMap";
    public static final String VALUE_TYPE_HASH_SET = "HashSet";

    // PinType constants
    public static final String PIN_TYPE_INPUT = "Input";
    public static final String PIN_TYPE_OUTPUT = "Output";

    // LogLevel constants
    public static final int LOG_LEVEL_DEBUG = 0;
    public static final int LOG_LEVEL_INFO = 1;
    public static final int LOG_LEVEL_WARN = 2;
    public static final int LOG_LEVEL_ERROR = 3;
    public static final int LOG_LEVEL_FATAL = 4;

    private Types() {}

    public static final class NodeScores {
        public int privacy;
        public int security;
        public int performance;
        public int governance;
        public int reliability;
        public int cost;

        public NodeScores(int privacy, int security, int performance,
                          int governance, int reliability, int cost) {
            this.privacy = privacy;
            this.security = security;
            this.performance = performance;
            this.governance = governance;
            this.reliability = reliability;
            this.cost = cost;
        }

        public String toJson() {
            StringBuilder b = new StringBuilder();
            b.append("{\"privacy\":").append(privacy);
            b.append(",\"security\":").append(security);
            b.append(",\"performance\":").append(performance);
            b.append(",\"governance\":").append(governance);
            b.append(",\"reliability\":").append(reliability);
            b.append(",\"cost\":").append(cost);
            b.append('}');
            return b.toString();
        }
    }

    public static final class PinDefinition {
        public String name;
        public String friendlyName;
        public String description;
        public String pinType;
        public String dataType;
        public String defaultValue;
        public String valueType;
        public String schema;
        public Double step;
        public Boolean sensitive;
        public Boolean enforceSchema;
        public Boolean enforceGenericValueType;

        private PinDefinition(String name, String friendlyName, String description,
                              String pinType, String dataType) {
            this.name = name;
            this.friendlyName = friendlyName;
            this.description = description;
            this.pinType = pinType;
            this.dataType = dataType;
        }

        public PinDefinition withDefault(String value) {
            this.defaultValue = value;
            return this;
        }

        public PinDefinition withValueType(String valueType) {
            this.valueType = valueType;
            return this;
        }

        public PinDefinition withSchema(String schema) {
            this.schema = schema;
            return this;
        }

        public PinDefinition withStep(double step) {
            this.step = step;
            return this;
        }

        public PinDefinition withSensitive(boolean sensitive) {
            this.sensitive = sensitive;
            return this;
        }

        public PinDefinition withEnforceSchema(boolean enforce) {
            this.enforceSchema = enforce;
            return this;
        }

        public PinDefinition withEnforceGenericValueType(boolean enforce) {
            this.enforceGenericValueType = enforce;
            return this;
        }

        public String toJson() {
            StringBuilder b = new StringBuilder();
            b.append("{\"name\":").append(Json.quote(name));
            b.append(",\"friendly_name\":").append(Json.quote(friendlyName));
            b.append(",\"description\":").append(Json.quote(description));
            b.append(",\"pin_type\":\"").append(pinType).append('"');
            b.append(",\"data_type\":\"").append(dataType).append('"');
            if (defaultValue != null) {
                b.append(",\"default_value\":").append(defaultValue);
            }
            if (valueType != null) {
                b.append(",\"value_type\":").append(Json.quote(valueType));
            }
            if (schema != null) {
                b.append(",\"schema\":").append(Json.quote(schema));
            }
            if (step != null) {
                b.append(",\"step\":").append(step);
            }
            if (sensitive != null) {
                b.append(",\"sensitive\":").append(sensitive ? "true" : "false");
            }
            if (enforceSchema != null) {
                b.append(",\"enforce_schema\":").append(enforceSchema ? "true" : "false");
            }
            if (enforceGenericValueType != null) {
                b.append(",\"enforce_generic_value_type\":").append(enforceGenericValueType ? "true" : "false");
            }
            b.append('}');
            return b.toString();
        }
    }

    public static PinDefinition inputPin(String name, String friendlyName,
                                          String description, String dataType) {
        return new PinDefinition(name, friendlyName, description, PIN_TYPE_INPUT, dataType);
    }

    public static PinDefinition outputPin(String name, String friendlyName,
                                           String description, String dataType) {
        return new PinDefinition(name, friendlyName, description, PIN_TYPE_OUTPUT, dataType);
    }

    public static PinDefinition inputExec(String name) {
        return new PinDefinition(name, name, "", PIN_TYPE_INPUT, DATA_TYPE_EXEC);
    }

    public static PinDefinition outputExec(String name) {
        return new PinDefinition(name, name, "", PIN_TYPE_OUTPUT, DATA_TYPE_EXEC);
    }

    public static final class NodeDefinition {
        public String name = "";
        public String friendlyName = "";
        public String description = "";
        public String category = "";
        public String icon;
        public final List<PinDefinition> pins = new ArrayList<>();
        public NodeScores scores;
        public boolean longRunning;
        public String docs;
        public int abiVersion = ABI_VERSION;

        public NodeDefinition setName(String name) {
            this.name = name;
            return this;
        }

        public NodeDefinition setFriendlyName(String friendlyName) {
            this.friendlyName = friendlyName;
            return this;
        }

        public NodeDefinition setDescription(String description) {
            this.description = description;
            return this;
        }

        public NodeDefinition setCategory(String category) {
            this.category = category;
            return this;
        }

        public NodeDefinition setIcon(String icon) {
            this.icon = icon;
            return this;
        }

        public NodeDefinition setLongRunning(boolean longRunning) {
            this.longRunning = longRunning;
            return this;
        }

        public NodeDefinition setDocs(String docs) {
            this.docs = docs;
            return this;
        }

        public NodeDefinition setScores(NodeScores scores) {
            this.scores = scores;
            return this;
        }

        public NodeDefinition addPin(PinDefinition pin) {
            this.pins.add(pin);
            return this;
        }

        public String toJson() {
            StringBuilder b = new StringBuilder();
            b.append("{\"name\":").append(Json.quote(name));
            b.append(",\"friendly_name\":").append(Json.quote(friendlyName));
            b.append(",\"description\":").append(Json.quote(description));
            b.append(",\"category\":").append(Json.quote(category));
            b.append(",\"pins\":[");
            for (int i = 0; i < pins.size(); i++) {
                if (i > 0) b.append(',');
                b.append(pins.get(i).toJson());
            }
            b.append("],\"long_running\":").append(longRunning ? "true" : "false");
            b.append(",\"abi_version\":").append(abiVersion);
            if (icon != null) {
                b.append(",\"icon\":").append(Json.quote(icon));
            }
            if (scores != null) {
                b.append(",\"scores\":").append(scores.toJson());
            }
            if (docs != null) {
                b.append(",\"docs\":").append(Json.quote(docs));
            }
            b.append('}');
            return b.toString();
        }
    }

    public static final class ExecutionInput {
        public final Map<String, String> inputs;
        public String nodeId = "";
        public String nodeName = "";
        public String runId = "";
        public String appId = "";
        public String boardId = "";
        public String userId = "";
        public boolean streamState;
        public int logLevel = LOG_LEVEL_INFO;

        public ExecutionInput() {
            this.inputs = new HashMap<>();
        }
    }

    public static final class ExecutionResult {
        public final Map<String, String> outputs;
        public String error;
        public final List<String> activateExec;
        public boolean pending;

        public ExecutionResult() {
            this.outputs = new HashMap<>();
            this.activateExec = new ArrayList<>();
        }

        public ExecutionResult setOutput(String name, String value) {
            outputs.put(name, value);
            return this;
        }

        public ExecutionResult activateExecPin(String pinName) {
            activateExec.add(pinName);
            return this;
        }

        public ExecutionResult setPending(boolean pending) {
            this.pending = pending;
            return this;
        }

        public String toJson() {
            StringBuilder b = new StringBuilder();
            b.append("{\"outputs\":{");
            boolean first = true;
            for (Map.Entry<String, String> entry : outputs.entrySet()) {
                if (!first) b.append(',');
                first = false;
                b.append(Json.quote(entry.getKey())).append(':').append(entry.getValue());
            }
            b.append("},\"activate_exec\":[");
            for (int i = 0; i < activateExec.size(); i++) {
                if (i > 0) b.append(',');
                b.append(Json.quote(activateExec.get(i)));
            }
            b.append("],\"pending\":").append(pending ? "true" : "false");
            if (error != null) {
                b.append(",\"error\":").append(Json.quote(error));
            }
            b.append('}');
            return b.toString();
        }
    }

    public static ExecutionResult successResult() {
        return new ExecutionResult();
    }

    public static ExecutionResult failResult(String message) {
        ExecutionResult r = new ExecutionResult();
        r.error = message;
        return r;
    }
}
