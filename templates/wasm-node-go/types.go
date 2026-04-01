package main

import (
	"strconv"
	"strings"
)

const ABIVersion uint32 = 1

// ── Node Definition ─────────────────────────────────────────────────────

type NodeDefinition struct {
	Name         string
	FriendlyName string
	Description  string
	Category     string
	Pins         []PinDefinition
	Permissions  []string
	LongRunning  bool
	ABIVersion   uint32
}

func NewNodeDefinition(name, friendlyName, description, category string) NodeDefinition {
	return NodeDefinition{
		Name:         name,
		FriendlyName: friendlyName,
		Description:  description,
		Category:     category,
		ABIVersion:   ABIVersion,
	}
}

func (n *NodeDefinition) AddPin(pin PinDefinition) {
	n.Pins = append(n.Pins, pin)
}

func (n *NodeDefinition) AddPermission(perm string) {
	n.Permissions = append(n.Permissions, perm)
}

func (n NodeDefinition) ToJSON() string {
	var b strings.Builder
	b.WriteString(`{"name":`)
	b.WriteString(jsonQuote(n.Name))
	b.WriteString(`,"friendly_name":`)
	b.WriteString(jsonQuote(n.FriendlyName))
	b.WriteString(`,"description":`)
	b.WriteString(jsonQuote(n.Description))
	b.WriteString(`,"category":`)
	b.WriteString(jsonQuote(n.Category))
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
	b.WriteString(strconv.FormatUint(uint64(n.ABIVersion), 10))
	if len(n.Permissions) > 0 {
		b.WriteString(`,"permissions":[`)
		for i, p := range n.Permissions {
			if i > 0 {
				b.WriteByte(',')
			}
			b.WriteString(jsonQuote(p))
		}
		b.WriteByte(']')
	}
	b.WriteByte('}')
	return b.String()
}

// ── Pin Definition ──────────────────────────────────────────────────────

type PinDefinition struct {
	Name         string
	FriendlyName string
	Description  string
	PinType      string
	DataType     string
	DefaultValue *string
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

func InputExecPin(name string) PinDefinition {
	return InputPin(name, humanize(name), "", "Exec")
}

func OutputExecPin(name string) PinDefinition {
	return OutputPin(name, humanize(name), "", "Exec")
}

func (p PinDefinition) WithDefault(value string) PinDefinition {
	p.DefaultValue = &value
	return p
}

func (p PinDefinition) ToJSON() string {
	var b strings.Builder
	b.WriteString(`{"name":`)
	b.WriteString(jsonQuote(p.Name))
	b.WriteString(`,"friendly_name":`)
	b.WriteString(jsonQuote(p.FriendlyName))
	b.WriteString(`,"description":`)
	b.WriteString(jsonQuote(p.Description))
	b.WriteString(`,"pin_type":"`)
	b.WriteString(p.PinType)
	b.WriteString(`","data_type":"`)
	b.WriteString(p.DataType)
	b.WriteByte('"')
	if p.DefaultValue != nil {
		b.WriteString(`,"default_value":`)
		b.WriteString(*p.DefaultValue)
	}
	b.WriteByte('}')
	return b.String()
}

// ── Execution Result ────────────────────────────────────────────────────

type ExecutionResult struct {
	Outputs      map[string]string
	Error        *string
	ActivateExec []string
	Pending      bool
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

func (r ExecutionResult) ToJSON() string {
	var b strings.Builder
	b.WriteString(`{"outputs":{`)
	first := true
	for k, v := range r.Outputs {
		if !first {
			b.WriteByte(',')
		}
		first = false
		b.WriteString(jsonQuote(k))
		b.WriteByte(':')
		b.WriteString(v)
	}
	b.WriteString(`},"activate_exec":[`)
	for i, e := range r.ActivateExec {
		if i > 0 {
			b.WriteByte(',')
		}
		b.WriteString(jsonQuote(e))
	}
	b.WriteString(`],"pending":`)
	if r.Pending {
		b.WriteString("true")
	} else {
		b.WriteString("false")
	}
	if r.Error != nil {
		b.WriteString(`,"error":`)
		b.WriteString(jsonQuote(*r.Error))
	}
	b.WriteByte('}')
	return b.String()
}

// ── JSON Helpers ────────────────────────────────────────────────────────

func jsonQuote(s string) string {
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

func humanize(name string) string {
	parts := strings.Split(name, "_")
	for i, p := range parts {
		if len(p) > 0 {
			parts[i] = strings.ToUpper(p[:1]) + p[1:]
		}
	}
	return strings.Join(parts, " ")
}
