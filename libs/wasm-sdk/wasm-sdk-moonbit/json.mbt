///| Minimal self-contained JSON parser for the Flow-Like WASM SDK.

///| JSON value type
pub enum JsonValue {
  Null
  Bool(Bool)
  Number(Double)
  Str(String)
  Arr(Array[JsonValue])
  Obj(Array[(String, JsonValue)])
}

pub fn JsonValue::get(self : JsonValue, key : String) -> JsonValue? {
  match self {
    Obj(entries) => {
      for i = 0; i < entries.length(); i = i + 1 {
        let (k, v) = entries[i]
        if k == key {
          return Some(v)
        }
      }
      None
    }
    _ => None
  }
}

pub fn JsonValue::as_string(self : JsonValue) -> String? {
  match self {
    Str(s) => Some(s)
    _ => None
  }
}

pub fn JsonValue::as_number(self : JsonValue) -> Double? {
  match self {
    Number(n) => Some(n)
    _ => None
  }
}

pub fn JsonValue::as_int(self : JsonValue) -> Int? {
  match self {
    Number(n) => Some(n.to_int())
    _ => None
  }
}

pub fn JsonValue::as_bool(self : JsonValue) -> Bool? {
  match self {
    Bool(b) => Some(b)
    _ => None
  }
}

pub fn JsonValue::as_array(self : JsonValue) -> Array[JsonValue]? {
  match self {
    Arr(a) => Some(a)
    _ => None
  }
}

pub fn JsonValue::to_raw_json(self : JsonValue) -> String {
  match self {
    Null => "null"
    Bool(true) => "true"
    Bool(false) => "false"
    Number(n) => n.to_string()
    Str(s) => "\"" + json_escape_string(s) + "\""
    Arr(items) => {
      let mut s = "["
      for i = 0; i < items.length(); i = i + 1 {
        if i > 0 { s = s + "," }
        s = s + items[i].to_raw_json()
      }
      s + "]"
    }
    Obj(entries) => {
      let mut s = "{"
      for i = 0; i < entries.length(); i = i + 1 {
        if i > 0 { s = s + "," }
        let (k, v) = entries[i]
        s = s + "\"" + json_escape_string(k) + "\":" + v.to_raw_json()
      }
      s + "}"
    }
  }
}

// ---------------------------------------------------------------------------
// Recursive-descent parser
// ---------------------------------------------------------------------------

priv struct JsonParser {
  chars : Array[Char]
  mut pos : Int
}

fn JsonParser::new(s : String) -> JsonParser {
  let chars : Array[Char] = []
  for c in s {
    chars.push(c)
  }
  { chars, pos: 0 }
}

fn JsonParser::peek(self : JsonParser) -> Char {
  if self.pos < self.chars.length() {
    self.chars[self.pos]
  } else {
    '\u0000'
  }
}

fn JsonParser::advance(self : JsonParser) -> Char {
  let c = self.peek()
  self.pos = self.pos + 1
  c
}

fn JsonParser::skip_ws(self : JsonParser) -> Unit {
  while self.pos < self.chars.length() {
    let c = self.chars[self.pos]
    if c == ' ' || c == '\t' || c == '\n' || c == '\r' {
      self.pos = self.pos + 1
    } else {
      break
    }
  }
}

fn JsonParser::parse_value(self : JsonParser) -> JsonValue {
  self.skip_ws()
  let c = self.peek()
  if c == '"' {
    return Str(self.parse_string())
  }
  if c == '{' {
    return Obj(self.parse_object())
  }
  if c == '[' {
    return Arr(self.parse_array())
  }
  if c == 't' {
    self.pos = self.pos + 4
    return Bool(true)
  }
  if c == 'f' {
    self.pos = self.pos + 5
    return Bool(false)
  }
  if c == 'n' {
    self.pos = self.pos + 4
    return Null
  }
  Number(self.parse_number())
}

fn JsonParser::parse_string(self : JsonParser) -> String {
  let _ = self.advance() // consume opening "
  let mut result = ""
  while self.peek() != '"' && self.pos < self.chars.length() {
    let c = self.advance()
    if c == '\\' {
      let esc = self.advance()
      if esc == '"' {
        result = result + "\""
      } else if esc == '\\' {
        result = result + "\\"
      } else if esc == '/' {
        result = result + "/"
      } else if esc == 'n' {
        result = result + "\n"
      } else if esc == 'r' {
        result = result + "\r"
      } else if esc == 't' {
        result = result + "\t"
      } else if esc == 'b' {
        result = result + "\u0008"
      } else if esc == 'f' {
        result = result + "\u000C"
      } else if esc == 'u' {
        let mut code = 0
        for i = 0; i < 4; i = i + 1 {
          code = code * 16 + hex_digit_value(self.advance())
        }
        result = result + Int::unsafe_to_char(code).to_string()
      } else {
        result = result + esc.to_string()
      }
    } else {
      result = result + c.to_string()
    }
  }
  let _ = self.advance() // consume closing "
  result
}

fn hex_digit_value(c : Char) -> Int {
  let code = c.to_int()
  if code >= 48 && code <= 57 {
    code - 48
  } else if code >= 65 && code <= 70 {
    code - 55
  } else if code >= 97 && code <= 102 {
    code - 87
  } else {
    0
  }
}

fn JsonParser::parse_number(self : JsonParser) -> Double {
  let start = self.pos
  if self.peek() == '-' {
    self.pos = self.pos + 1
  }
  while self.pos < self.chars.length() && is_json_digit(self.peek()) {
    self.pos = self.pos + 1
  }
  if self.pos < self.chars.length() && self.peek() == '.' {
    self.pos = self.pos + 1
    while self.pos < self.chars.length() && is_json_digit(self.peek()) {
      self.pos = self.pos + 1
    }
  }
  if self.pos < self.chars.length() && (self.peek() == 'e' || self.peek() == 'E') {
    self.pos = self.pos + 1
    if self.pos < self.chars.length() && (self.peek() == '+' || self.peek() == '-') {
      self.pos = self.pos + 1
    }
    while self.pos < self.chars.length() && is_json_digit(self.peek()) {
      self.pos = self.pos + 1
    }
  }
  // Manual number conversion
  let mut negative = false
  let mut idx = start
  if idx < self.pos && self.chars[idx] == '-' {
    negative = true
    idx = idx + 1
  }
  let mut int_part = 0.0
  while idx < self.pos && self.chars[idx] != '.' && self.chars[idx] != 'e' && self.chars[idx] != 'E' {
    int_part = int_part * 10.0 + (self.chars[idx].to_int() - 48).to_double()
    idx = idx + 1
  }
  let mut frac_part = 0.0
  if idx < self.pos && self.chars[idx] == '.' {
    idx = idx + 1
    let mut factor = 0.1
    while idx < self.pos && self.chars[idx] != 'e' && self.chars[idx] != 'E' {
      frac_part = frac_part + (self.chars[idx].to_int() - 48).to_double() * factor
      factor = factor * 0.1
      idx = idx + 1
    }
  }
  let mut result = int_part + frac_part
  if idx < self.pos && (self.chars[idx] == 'e' || self.chars[idx] == 'E') {
    idx = idx + 1
    let mut exp_neg = false
    if idx < self.pos && self.chars[idx] == '-' {
      exp_neg = true
      idx = idx + 1
    } else if idx < self.pos && self.chars[idx] == '+' {
      idx = idx + 1
    }
    let mut exp = 0.0
    while idx < self.pos {
      exp = exp * 10.0 + (self.chars[idx].to_int() - 48).to_double()
      idx = idx + 1
    }
    let mut multiplier = 1.0
    let exp_int = exp.to_int()
    for _i = 0; _i < exp_int; _i = _i + 1 {
      multiplier = multiplier * 10.0
    }
    if exp_neg {
      result = result / multiplier
    } else {
      result = result * multiplier
    }
  }
  if negative {
    result = -result
  }
  result
}

fn is_json_digit(c : Char) -> Bool {
  c >= '0' && c <= '9'
}

fn JsonParser::parse_object(self : JsonParser) -> Array[(String, JsonValue)] {
  let _ = self.advance() // consume {
  self.skip_ws()
  let entries : Array[(String, JsonValue)] = []
  if self.peek() == '}' {
    let _ = self.advance()
    return entries
  }
  while true {
    self.skip_ws()
    let key = self.parse_string()
    self.skip_ws()
    let _ = self.advance() // consume :
    self.skip_ws()
    let value = self.parse_value()
    entries.push((key, value))
    self.skip_ws()
    if self.peek() == ',' {
      let _ = self.advance()
    } else {
      break
    }
  }
  self.skip_ws()
  if self.peek() == '}' {
    let _ = self.advance()
  }
  entries
}

fn JsonParser::parse_array(self : JsonParser) -> Array[JsonValue] {
  let _ = self.advance() // consume [
  self.skip_ws()
  let items : Array[JsonValue] = []
  if self.peek() == ']' {
    let _ = self.advance()
    return items
  }
  while true {
    self.skip_ws()
    items.push(self.parse_value())
    self.skip_ws()
    if self.peek() == ',' {
      let _ = self.advance()
    } else {
      break
    }
  }
  self.skip_ws()
  if self.peek() == ']' {
    let _ = self.advance()
  }
  items
}

///| Parse a JSON string into a JsonValue
pub fn parse_json(s : String) -> JsonValue {
  let parser = JsonParser::new(s)
  parser.parse_value()
}
