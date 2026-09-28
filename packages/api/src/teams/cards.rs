use crate::error::ApiError;
use flow_like_types::interaction::{FormField, FormFieldType, InteractionRequest, InteractionType};
use serde_json::{Map, Value, json};

fn invalid(message: &str) -> ApiError {
    ApiError::bad_request(message)
}

fn properties(request: &InteractionRequest) -> Result<Vec<(String, Value, bool)>, ApiError> {
    let InteractionType::Form { schema, fields } = &request.interaction_type else {
        return Ok(vec![]);
    };
    if let Some(schema) = schema {
        let props = schema["properties"]
            .as_object()
            .ok_or_else(|| invalid("Teams forms require an object schema with properties"))?;
        if props.len() > 30 {
            return Err(invalid("Teams forms support up to 30 fields"));
        }
        return Ok(props
            .iter()
            .map(|(id, property)| {
                (
                    id.clone(),
                    property.clone(),
                    schema["required"]
                        .as_array()
                        .is_some_and(|required| required.iter().any(|v| v == id)),
                )
            })
            .collect());
    }
    if fields.len() > 30 {
        return Err(invalid("Teams forms support up to 30 fields"));
    }
    Ok(fields
        .iter()
        .map(|field| {
            let schema = field_schema(field);
            (field.id.clone(), schema, field.required)
        })
        .collect())
}

fn field_schema(field: &FormField) -> Value {
    let mut schema = json!({"title":field.label,"default":field.default_value,
        "type":match field.field_type{FormFieldType::Number=>"number",FormFieldType::Boolean=>"boolean",_=>"string"}});
    if let Some(description) = &field.description {
        schema["description"] = json!(description);
    }
    if matches!(field.field_type, FormFieldType::Select) {
        schema["enum"] = json!(field.options.iter().map(|o| &o.id).collect::<Vec<_>>());
        schema["x-choice-labels"] =
            json!(field.options.iter().map(|o| &o.label).collect::<Vec<_>>());
    }
    schema
}

pub(super) fn render(request: &InteractionRequest, id: &str) -> Result<Value, ApiError> {
    let mut body = vec![
        json!({"type":"TextBlock","text":request.name,"weight":"Bolder","wrap":true}),
        json!({"type":"TextBlock","text":request.description,"wrap":true}),
    ];
    match &request.interaction_type {
        InteractionType::SingleChoice {
            options,
            allow_freeform,
        } => {
            body.push(json!({"type":"Input.ChoiceSet","id":"choice","label":"Choose an option","style":"expanded","choices":options.iter().map(|o|json!({"title":o.label,"value":o.id})).collect::<Vec<_>>()}));
            if *allow_freeform || options.iter().any(|o| o.freeform) {
                body.push(json!({"type":"Input.Text","id":"freeform","label":"Your answer","isMultiline":true,"maxLength":8000}));
            }
        }
        InteractionType::MultipleChoice {
            options,
            min_selections,
            ..
        } => {
            body.push(json!({"type":"Input.ChoiceSet","id":"choice","label":"Choose options","style":"expanded","isMultiSelect":true,"isRequired":*min_selections>0,"errorMessage":"Choose the requested number of options.","choices":options.iter().map(|o|json!({"title":o.label,"value":o.id})).collect::<Vec<_>>()}));
        }
        InteractionType::Form { .. } => {
            for (index, (id, schema, required)) in properties(request)?.iter().enumerate() {
                let label = schema["title"].as_str().unwrap_or(id);
                let mut input = json!({"id":format!("field_{index}"),"label":label,"isRequired":required,"errorMessage":format!("Enter a valid value for {label}.")});
                if let Some(choices) = schema["enum"].as_array() {
                    input["type"] = json!("Input.ChoiceSet");
                    input["choices"]=json!(choices.iter().enumerate().map(|(i,v)|json!({"title":schema["x-choice-labels"][i].as_str().or_else(||v.as_str()).map(str::to_owned).unwrap_or_else(||v.to_string()),"value":i.to_string()})).collect::<Vec<_>>());
                    if let Some(i) = choices.iter().position(|v| *v == schema["default"]) {
                        input["value"] = json!(i.to_string());
                    }
                } else {
                    match schema["type"].as_str().unwrap_or("string") {
                        "boolean" => {
                            input["type"] = json!("Input.Toggle");
                            input["title"] = json!(label);
                            input["valueOn"] = json!("true");
                            input["valueOff"] = json!("false");
                            input["value"] =
                                json!(schema["default"].as_bool().unwrap_or(false).to_string());
                            input["isRequired"] = json!(false);
                        }
                        "integer" | "number" => {
                            input["type"] = json!("Input.Number");
                            if schema["default"].is_number() {
                                input["value"] = schema["default"].clone();
                            }
                        }
                        kind => {
                            input["type"] = json!("Input.Text");
                            input["maxLength"] = json!(8000);
                            if kind == "object" || kind == "array" {
                                input["label"] = json!(format!("{label} (JSON)"));
                                input["isMultiline"] = json!(true);
                            }
                            if let Some(value) = schema.get("default").filter(|v| !v.is_null()) {
                                input["value"] = json!(
                                    value
                                        .as_str()
                                        .map(str::to_owned)
                                        .unwrap_or_else(|| value.to_string())
                                );
                            }
                        }
                    }
                }
                body.push(input);
            }
        }
    }
    let data = json!({"flow_like_action":id});
    Ok(
        json!({"type":"AdaptiveCard","$schema":"http://adaptivecards.io/schemas/adaptive-card.json","version":"1.4","body":body,
        "actions":[{"type":"Action.Execute","title":"Submit","verb":"flow_like.interaction","data":data,
        "fallback":{"type":"Action.Submit","title":"Submit","data":data}}]}),
    )
}

pub(super) fn response(request: &InteractionRequest, data: &Value) -> Result<Value, ApiError> {
    match &request.interaction_type {
        InteractionType::SingleChoice {
            options,
            allow_freeform,
        } => {
            let selected = data["choice"].as_str();
            let option = options.iter().find(|o| Some(o.id.as_str()) == selected);
            if selected.is_some_and(|id| !id.is_empty()) && option.is_none() {
                return Err(invalid("Choose a valid option"));
            }
            let free = data["freeform"].as_str().filter(|s| !s.trim().is_empty());
            if let Some(free) = free {
                if !*allow_freeform && !option.is_some_and(|o| o.freeform) {
                    return Err(invalid("A custom answer is not allowed"));
                }
                if free.len() > 8000 {
                    return Err(invalid("The answer is too long"));
                }
                return Ok(json!({"selected_id":selected,"freeform_value":free}));
            }
            let option = option.ok_or_else(|| invalid("Choose an option"))?;
            if option.freeform {
                return Err(invalid("Enter your answer"));
            }
            Ok(json!({"selected_id":option.id,"freeform_value":null}))
        }
        InteractionType::MultipleChoice {
            options,
            min_selections,
            max_selections,
        } => {
            let ids: Vec<&str> = data["choice"]
                .as_str()
                .unwrap_or_default()
                .split(',')
                .filter(|s| !s.is_empty())
                .collect();
            let unique: std::collections::BTreeSet<_> = ids.iter().copied().collect();
            if unique.len() != ids.len()
                || ids.len() < *min_selections
                || ids.len() > *max_selections
                || ids.iter().any(|id| !options.iter().any(|o| &o.id == id))
            {
                return Err(invalid("Choose the requested number of valid options"));
            }
            Ok(json!({"selected_ids":ids}))
        }
        InteractionType::Form { schema, .. } => {
            let mut values = Map::new();
            for (index, (id, property, required)) in properties(request)?.iter().enumerate() {
                let raw = &data[format!("field_{index}")];
                if raw.is_null() || raw.as_str() == Some("") {
                    if *required {
                        return Err(invalid(&format!("Complete {id}")));
                    }
                    continue;
                }
                let value = if let Some(choices) = property["enum"].as_array() {
                    let index = raw
                        .as_str()
                        .and_then(|s| s.parse::<usize>().ok())
                        .ok_or_else(|| invalid("Invalid selection"))?;
                    choices
                        .get(index)
                        .cloned()
                        .ok_or_else(|| invalid("Invalid selection"))?
                } else {
                    match property["type"].as_str().unwrap_or("string") {
                        "boolean" => match raw.as_str() {
                            Some("true") => json!(true),
                            Some("false") => json!(false),
                            _ => return Err(invalid("Invalid checkbox value")),
                        },
                        "integer" | "number" | "object" | "array" => {
                            if let Some(s) = raw.as_str() {
                                serde_json::from_str(s).map_err(|_| {
                                    invalid(&format!("Enter a valid value for {id}"))
                                })?
                            } else {
                                raw.clone()
                            }
                        }
                        _ => raw.clone(),
                    }
                };
                let validator = jsonschema::validator_for(property)
                    .map_err(|_| invalid("This form has an invalid schema"))?;
                if !validator.is_valid(&value) {
                    return Err(invalid(&format!("Enter a valid value for {id}")));
                }
                values.insert(id.clone(), value);
            }
            let value = Value::Object(values);
            if let Some(schema) = schema {
                let validator = jsonschema::validator_for(schema)
                    .map_err(|_| invalid("This form has an invalid schema"))?;
                if !validator.is_valid(&value) {
                    return Err(invalid("Check the form values and try again"));
                }
            }
            Ok(value)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_types::interaction::{ChoiceOption, InteractionStatus};
    fn choice() -> InteractionRequest {
        InteractionRequest {
            id: "request".into(),
            name: "Approve?".into(),
            description: "Review".into(),
            interaction_type: InteractionType::SingleChoice {
                options: vec![ChoiceOption {
                    id: "yes".into(),
                    label: "Approve".into(),
                    description: None,
                    freeform: false,
                }],
                allow_freeform: false,
            },
            status: InteractionStatus::Pending,
            ttl_seconds: 60,
            expires_at: 100,
            run_id: None,
            app_id: None,
            channel: None,
        }
    }
    #[test]
    fn cards_keep_channel_credentials_on_the_server() {
        let card = render(&choice(), "opaque-action").unwrap();
        assert_eq!(
            card["actions"][0]["data"],
            json!({"flow_like_action":"opaque-action"})
        );
        assert!(!card.to_string().contains("channel"));
    }
    #[test]
    fn rejects_forged_choices_and_unrequested_freeform() {
        assert_eq!(
            response(&choice(), &json!({"choice":"yes"})).unwrap()["selected_id"],
            "yes"
        );
        assert!(response(&choice(), &json!({"choice":"forged"})).is_err());
        assert!(response(&choice(), &json!({"choice":"yes","freeform":"bypass"})).is_err());
    }

    #[test]
    fn validates_form_values_and_preserves_option_labels() {
        let mut request = choice();
        request.interaction_type = InteractionType::Form {
            schema: Some(json!({
                "type":"object", "properties":{"count":{"type":"integer","minimum":1},"approved":{"type":"boolean"}}, "required":["count","approved"]
            })),
            fields: vec![],
        };
        let properties = properties(&request).unwrap();
        let mut data = json!({});
        for (i, (id, _, _)) in properties.iter().enumerate() {
            data[format!("field_{i}")] = json!(if id == "count" { "2" } else { "false" });
        }
        assert_eq!(
            response(&request, &data).unwrap(),
            json!({"count":2,"approved":false})
        );
        let i = properties
            .iter()
            .position(|(id, _, _)| id == "count")
            .unwrap();
        data[format!("field_{i}")] = json!("0");
        assert!(response(&request, &data).is_err());
        request.interaction_type = InteractionType::Form {
            schema: None,
            fields: vec![FormField {
                id: "priority".into(),
                label: "Priority".into(),
                description: None,
                field_type: FormFieldType::Select,
                required: true,
                default_value: None,
                options: vec![ChoiceOption {
                    id: "p1".into(),
                    label: "Urgent".into(),
                    description: None,
                    freeform: false,
                }],
            }],
        };
        assert_eq!(
            render(&request, "opaque").unwrap()["body"][2]["choices"][0]["title"],
            "Urgent"
        );
        assert_eq!(
            response(&request, &json!({"field_0":"0"})).unwrap(),
            json!({"priority":"p1"})
        );
        assert!(response(&request, &json!({"field_0":"100"})).is_err());
    }

    #[test]
    fn enforces_multiple_choice_cardinality_and_uniqueness() {
        let mut request = choice();
        let InteractionType::SingleChoice { options, .. } = request.interaction_type else {
            unreachable!()
        };
        request.interaction_type = InteractionType::MultipleChoice {
            options,
            min_selections: 1,
            max_selections: 1,
        };
        assert!(response(&request, &json!({"choice":"yes"})).is_ok());
        for choice in ["", "yes,yes", "other"] {
            assert!(response(&request, &json!({"choice":choice})).is_err());
        }
    }
}
