package sdk

import (
	"strconv"
	"strings"
)

const ABIVersion = 1

const (
	LogLevelDebug = 0
	LogLevelInfo  = 1
	LogLevelWarn  = 2
	LogLevelError = 3
	LogLevelFatal = 4
)

const (
	DataTypeExec    = "Exec"
	DataTypeString  = "String"
	DataTypeI64     = "I64"
	DataTypeF64     = "F64"
	DataTypeBool    = "Bool"
	DataTypeGeneric = "Generic"
	DataTypeBytes   = "Bytes"
	DataTypeDate    = "Date"
	DataTypePathBuf = "PathBuf"
	DataTypeStruct  = "Struct"
)

const (
	ValueTypeNormal  = "Normal"
	ValueTypeArray   = "Array"
	ValueTypeHashMap = "HashMap"
	ValueTypeHashSet = "HashSet"
)

type NodeScores struct {
	Privacy     uint8 `json:"privacy"`
	Security    uint8 `json:"security"`
	Performance uint8 `json:"performance"`
	Governance  uint8 `json:"governance"`
	Reliability uint8 `json:"reliability"`
	Cost        uint8 `json:"cost"`
}

func (s *NodeScores) ToJSON() string {
	var b strings.Builder
	b.WriteString(`{"privacy":`)
	b.WriteString(strconv.Itoa(int(s.Privacy)))
	b.WriteString(`,"security":`)
	b.WriteString(strconv.Itoa(int(s.Security)))
	b.WriteString(`,"performance":`)
	b.WriteString(strconv.Itoa(int(s.Performance)))
	b.WriteString(`,"governance":`)
	b.WriteString(strconv.Itoa(int(s.Governance)))
	b.WriteString(`,"reliability":`)
	b.WriteString(strconv.Itoa(int(s.Reliability)))
	b.WriteString(`,"cost":`)
	b.WriteString(strconv.Itoa(int(s.Cost)))
	b.WriteString("}")
	return b.String()
}

type PinDefinition struct {
	Name         string  `json:"name"`
	FriendlyName string  `json:"friendly_name"`
	Description  string  `json:"description"`
	PinType      string  `json:"pin_type"`
	DataType     string  `json:"data_type"`
	DefaultValue             *string  `json:"default_value,omitempty"`
	ValueType                *string  `json:"value_type,omitempty"`
	Schema                   *string  `json:"schema,omitempty"`
	Step                     *float64 `json:"step,omitempty"`
	Sensitive                *bool    `json:"sensitive,omitempty"`
	EnforceSchema            *bool    `json:"enforce_schema,omitempty"`
	EnforceGenericValueType  *bool    `json:"enforce_generic_value_type,omitempty"`
}

func InputPin(name, friendlyName, description, dataType string) PinDefinition {
	return PinDefinition{
		Name:         name,
		FriendlyName: friendlyName,
		Description:  description,
		PinType:      "Input",
		DataType:     dataType,
	}
}

func OutputPin(name, friendlyName, description, dataType string) PinDefinition {
	return PinDefinition{
		Name:         name,
		FriendlyName: friendlyName,
		Description:  description,
		PinType:      "Output",
		DataType:     dataType,
	}
}

func (p PinDefinition) WithDefault(value string) PinDefinition {
	p.DefaultValue = &value
	return p
}

// WithValueType sets the value type (e.g. "Array", "HashMap", "HashSet") on a pin.
func (p PinDefinition) WithValueType(valueType string) PinDefinition {
	p.ValueType = &valueType
	return p
}

// WithSchema attaches a raw JSON Schema string to a pin.
func (p PinDefinition) WithSchema(schema string) PinDefinition {
	p.Schema = &schema
	return p
}

func (p PinDefinition) WithStep(step float64) PinDefinition {
	p.Step = &step
	return p
}

func (p PinDefinition) WithSensitive(sensitive bool) PinDefinition {
	p.Sensitive = &sensitive
	return p
}

func (p PinDefinition) WithEnforceSchema(enforce bool) PinDefinition {
	p.EnforceSchema = &enforce
	return p
}

func (p PinDefinition) WithEnforceGenericValueType(enforce bool) PinDefinition {
	p.EnforceGenericValueType = &enforce
	return p
}

func (p *PinDefinition) ToJSON() string {
	var b strings.Builder
	b.WriteString(`{"name":`)
	b.WriteString(jsonString(p.Name))
	b.WriteString(`,"friendly_name":`)
	b.WriteString(jsonString(p.FriendlyName))
	b.WriteString(`,"description":`)
	b.WriteString(jsonString(p.Description))
	b.WriteString(`,"pin_type":"`)
	b.WriteString(p.PinType)
	b.WriteString(`","data_type":"`)
	b.WriteString(p.DataType)
	b.WriteByte('"')
	if p.DefaultValue != nil {
		b.WriteString(`,"default_value":`)
		b.WriteString(*p.DefaultValue)
	}
	if p.ValueType != nil {
		b.WriteString(`,"value_type":`)
		b.WriteString(jsonString(*p.ValueType))
	}
	if p.Schema != nil {
		b.WriteString(`,"schema":`)
		b.WriteString(jsonString(*p.Schema))
	}
	if p.Step != nil {
		b.WriteString(`,"step":`)
		b.WriteString(strconv.FormatFloat(*p.Step, 'f', -1, 64))
	}
	if p.Sensitive != nil {
		b.WriteString(`,"sensitive":`)
		if *p.Sensitive {
			b.WriteString("true")
		} else {
			b.WriteString("false")
		}
	}
	if p.EnforceSchema != nil {
		b.WriteString(`,"enforce_schema":`)
		if *p.EnforceSchema {
			b.WriteString("true")
		} else {
			b.WriteString("false")
		}
	}
	if p.EnforceGenericValueType != nil {
		b.WriteString(`,"enforce_generic_value_type":`)
		if *p.EnforceGenericValueType {
			b.WriteString("true")
		} else {
			b.WriteString("false")
		}
	}
	b.WriteByte('}')
	return b.String()
}

type NodeDefinition struct {
	Name         string         `json:"name"`
	FriendlyName string         `json:"friendly_name"`
	Description  string         `json:"description"`
	Category     string         `json:"category"`
	Icon         *string        `json:"icon,omitempty"`
	Pins         []PinDefinition `json:"pins"`
	Scores       *NodeScores    `json:"scores,omitempty"`
	LongRunning  bool           `json:"long_running"`
	Docs         *string        `json:"docs,omitempty"`
	Permissions  []string       `json:"permissions,omitempty"`
	ABIVersion   int            `json:"abi_version"`
}

func NewNodeDefinition() NodeDefinition {
	return NodeDefinition{
		ABIVersion: ABIVersion,
	}
}

func (n *NodeDefinition) AddPin(pin PinDefinition) *NodeDefinition {
	n.Pins = append(n.Pins, pin)
	return n
}

func (n *NodeDefinition) SetScores(scores NodeScores) *NodeDefinition {
	n.Scores = &scores
	return n
}

func (n *NodeDefinition) AddPermission(perm string) *NodeDefinition {
	n.Permissions = append(n.Permissions, perm)
	return n
}

func (n *NodeDefinition) ToJSON() string {
	var b strings.Builder
	b.WriteString(`{"name":`)
	b.WriteString(jsonString(n.Name))
	b.WriteString(`,"friendly_name":`)
	b.WriteString(jsonString(n.FriendlyName))
	b.WriteString(`,"description":`)
	b.WriteString(jsonString(n.Description))
	b.WriteString(`,"category":`)
	b.WriteString(jsonString(n.Category))
	b.WriteString(`,"pins":[`)
	for i := range n.Pins {
		if i > 0 {
			b.WriteByte(',')
		}
		b.WriteString(n.Pins[i].ToJSON())
	}
	b.WriteString(`],"long_running":`)
	if n.LongRunning {
		b.WriteString("true")
	} else {
		b.WriteString("false")
	}
	b.WriteString(`,"abi_version":`)
	b.WriteString(strconv.Itoa(n.ABIVersion))
	if n.Icon != nil {
		b.WriteString(`,"icon":`)
		b.WriteString(jsonString(*n.Icon))
	}
	if n.Scores != nil {
		b.WriteString(`,"scores":`)
		b.WriteString(n.Scores.ToJSON())
	}
	if n.Docs != nil {
		b.WriteString(`,"docs":`)
		b.WriteString(jsonString(*n.Docs))
	}
	if len(n.Permissions) > 0 {
		b.WriteString(`,"permissions":[`)
		for i, p := range n.Permissions {
			if i > 0 {
				b.WriteByte(',')
			}
			b.WriteString(jsonString(p))
		}
		b.WriteByte(']')
	}
	b.WriteByte('}')
	return b.String()
}

type ExecutionInput struct {
	Inputs      map[string]string `json:"inputs"`
	NodeID      string            `json:"node_id"`
	NodeName    string            `json:"node_name"`
	RunID       string            `json:"run_id"`
	AppID       string            `json:"app_id"`
	BoardID     string            `json:"board_id"`
	UserID      string            `json:"user_id"`
	StreamState bool              `json:"stream_state"`
	LogLevel    uint8             `json:"log_level"`
}

type ExecutionResult struct {
	Outputs      map[string]string `json:"outputs"`
	Error        *string           `json:"error,omitempty"`
	ActivateExec []string          `json:"activate_exec"`
	Pending      bool              `json:"pending"`
}

func SuccessResult() ExecutionResult {
	return ExecutionResult{
		Outputs:      make(map[string]string),
		ActivateExec: []string{},
	}
}

func FailResult(message string) ExecutionResult {
	return ExecutionResult{
		Outputs:      make(map[string]string),
		Error:        &message,
		ActivateExec: []string{},
	}
}

func (r *ExecutionResult) SetOutput(name, value string) *ExecutionResult {
	r.Outputs[name] = value
	return r
}

func (r *ExecutionResult) ActivateExecPin(pinName string) *ExecutionResult {
	r.ActivateExec = append(r.ActivateExec, pinName)
	return r
}

func (r *ExecutionResult) SetPending(pending bool) *ExecutionResult {
	r.Pending = pending
	return r
}

func (r *ExecutionResult) ToJSON() string {
	var b strings.Builder
	b.WriteString(`{"outputs":{`)
	first := true
	for k, v := range r.Outputs {
		if !first {
			b.WriteByte(',')
		}
		first = false
		b.WriteString(jsonString(k))
		b.WriteByte(':')
		b.WriteString(v)
	}
	b.WriteString(`},"activate_exec":[`)
	for i, e := range r.ActivateExec {
		if i > 0 {
			b.WriteByte(',')
		}
		b.WriteString(jsonString(e))
	}
	b.WriteString(`],"pending":`)
	if r.Pending {
		b.WriteString("true")
	} else {
		b.WriteString("false")
	}
	if r.Error != nil {
		b.WriteString(`,"error":`)
		b.WriteString(jsonString(*r.Error))
	}
	b.WriteByte('}')
	return b.String()
}

func jsonString(s string) string {
	var b strings.Builder
	b.WriteByte('"')
	for i := 0; i < len(s); i++ {
		c := s[i]
		switch c {
		case '"':
			b.WriteString(`\"`)
		case '\\':
			b.WriteString(`\\`)
		case '\n':
			b.WriteString(`\n`)
		case '\r':
			b.WriteString(`\r`)
		case '\t':
			b.WriteString(`\t`)
		default:
			b.WriteByte(c)
		}
	}
	b.WriteByte('"')
	return b.String()
}

// JSONString exports the jsonString helper for use in node implementations.
func JSONString(s string) string {
	return jsonString(s)
}
