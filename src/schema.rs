use chrono::{DateTime, NaiveDate, SecondsFormat, Utc};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

use crate::error::{AppError, FieldError};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FieldType {
    String,
    Int,
    Float,
    Bool,
    Datetime,
    Date,
    Uuid,
    Object,
    Array,
    Any,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AutoField {
    CreatedAt,
    UpdatedAt,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum UniqueKey {
    Single(String),
    Compound(Vec<String>),
}

impl UniqueKey {
    pub fn fields(&self) -> Vec<String> {
        match self {
            UniqueKey::Single(field) => vec![field.clone()],
            UniqueKey::Compound(fields) => fields.clone(),
        }
    }

    pub fn label(&self) -> String {
        self.fields().join("+")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldSchema {
    #[serde(rename = "type")]
    pub field_type: FieldType,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub nullable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<Value>,
    #[serde(default)]
    pub encrypted: bool,
    #[serde(default)]
    pub unique: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_length: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_length: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pattern: Option<String>,
    #[serde(default, rename = "enum", skip_serializing_if = "Option::is_none")]
    pub enum_values: Option<Vec<Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub items: Option<Box<FieldSchema>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fields: Option<BTreeMap<String, FieldSchema>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto: Option<AutoField>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TableSchema {
    #[serde(default)]
    pub strict: bool,
    #[serde(default)]
    pub fields: BTreeMap<String, FieldSchema>,
    #[serde(default)]
    pub unique: Vec<UniqueKey>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SchemaPatch {
    #[serde(default)]
    pub add: BTreeMap<String, Value>,
    #[serde(default)]
    pub remove: Vec<String>,
    #[serde(default)]
    pub update: BTreeMap<String, Value>,
    #[serde(default)]
    pub unique_add: Vec<UniqueKey>,
    #[serde(default)]
    pub unique_remove: Vec<UniqueKey>,
    #[serde(default)]
    pub strict: Option<bool>,
}

fn patch_error(message: impl Into<String>) -> AppError {
    AppError::BadRequest(format!("schema patch: {}", message.into()))
}

fn merge_json(base: &mut Value, patch: &Value) {
    match (base, patch) {
        (Value::Object(base_map), Value::Object(patch_map)) => {
            for (key, value) in patch_map {
                base_map.insert(key.clone(), value.clone());
            }
        }
        (base, patch) => *base = patch.clone(),
    }
}

pub fn apply_patch(schema: &mut TableSchema, patch: &SchemaPatch) -> Result<(), AppError> {
    for name in &patch.remove {
        if schema.fields.remove(name).is_none() {
            return Err(patch_error(format!("field '{name}' does not exist")));
        }
    }
    for (name, changes) in &patch.update {
        let Some(existing) = schema.fields.get_mut(name) else {
            return Err(patch_error(format!(
                "field '{name}' does not exist (use add)"
            )));
        };
        let mut base = serde_json::to_value(&*existing)?;
        merge_json(&mut base, changes);
        let updated: FieldSchema = serde_json::from_value(base)
            .map_err(|err| patch_error(format!("invalid field '{name}': {err}")))?;
        *existing = updated;
    }
    for (name, value) in &patch.add {
        if schema.fields.contains_key(name) {
            return Err(patch_error(format!(
                "field '{name}' already exists (use update)"
            )));
        }
        let field: FieldSchema = serde_json::from_value(value.clone())
            .map_err(|err| patch_error(format!("invalid field '{name}': {err}")))?;
        schema.fields.insert(name.clone(), field);
    }
    for key in &patch.unique_add {
        let label = key.label();
        if schema
            .unique
            .iter()
            .any(|existing| existing.label() == label)
        {
            return Err(patch_error(format!("unique '{label}' already exists")));
        }
        schema.unique.push(key.clone());
    }
    for key in &patch.unique_remove {
        let label = key.label();
        let Some(index) = schema
            .unique
            .iter()
            .position(|existing| existing.label() == label)
        else {
            return Err(patch_error(format!("unique '{label}' does not exist")));
        };
        schema.unique.remove(index);
    }
    if let Some(strict) = patch.strict {
        schema.strict = strict;
    }
    compile(schema)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Create,
    Replace,
    Migrate,
}

pub type UniqueIndex = UniqueStore;

pub fn canonical(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_default()
}

fn schema_error(message: impl Into<String>) -> AppError {
    AppError::BadRequest(format!("invalid schema: {}", message.into()))
}

#[derive(Debug, Default)]
pub struct UniqueStore {
    singles: BTreeMap<String, BTreeSet<String>>,
    compounds: BTreeMap<String, BTreeSet<String>>,
}

impl UniqueStore {
    pub fn new() -> Self {
        Self::default()
    }

    fn compound_label(fields: &[String]) -> String {
        fields.join("\u{1f}")
    }

    pub fn add_single(&mut self, field: &str, value: String) {
        self.singles
            .entry(field.to_string())
            .or_default()
            .insert(value);
    }

    pub fn has_single(&self, field: &str, value: &str) -> bool {
        self.singles
            .get(field)
            .is_some_and(|set| set.contains(value))
    }

    pub fn add_compound(&mut self, fields: &[String], value: String) {
        self.compounds
            .entry(Self::compound_label(fields))
            .or_default()
            .insert(value);
    }

    pub fn has_compound(&self, fields: &[String], value: &str) -> bool {
        self.compounds
            .get(&Self::compound_label(fields))
            .is_some_and(|set| set.contains(value))
    }
}

pub fn compile(schema: &TableSchema) -> Result<(), AppError> {
    for (name, field) in &schema.fields {
        compile_field(name, field, true)?;
    }
    let mut seen = BTreeSet::new();
    for key in &schema.unique {
        let fields = key.fields();
        if fields.is_empty() {
            return Err(schema_error("unique: empty key"));
        }
        let mut components = BTreeSet::new();
        for name in &fields {
            if name.is_empty() {
                return Err(schema_error("unique: empty field name"));
            }
            if !components.insert(name.clone()) {
                return Err(schema_error(format!(
                    "unique: field '{name}' repeated in key '{}'",
                    key.label()
                )));
            }
            let Some(field) = schema.fields.get(name) else {
                return Err(schema_error(format!(
                    "unique: field '{name}' not declared in schema"
                )));
            };
            if field.encrypted {
                return Err(schema_error(format!(
                    "unique: field '{name}' cannot be encrypted"
                )));
            }
        }
        let label = key.label();
        if !seen.insert(label.clone()) {
            return Err(schema_error(format!("unique: duplicate key '{label}'")));
        }
    }
    Ok(())
}

fn compile_field(path: &str, field: &FieldSchema, top: bool) -> Result<(), AppError> {
    let leaf = path.rsplit('.').next().unwrap_or(path);
    if leaf.starts_with('_') {
        return Err(schema_error(format!(
            "field '{path}': names starting with '_' are reserved"
        )));
    }
    if !top {
        if field.encrypted {
            return Err(schema_error(format!(
                "field '{path}': encrypted is only supported on top-level fields"
            )));
        }
        if field.unique {
            return Err(schema_error(format!(
                "field '{path}': unique is only supported on top-level fields"
            )));
        }
        if field.auto.is_some() {
            return Err(schema_error(format!(
                "field '{path}': auto is only supported on top-level fields"
            )));
        }
    }
    if field.encrypted && field.unique {
        return Err(schema_error(format!(
            "field '{path}': unique is not supported on encrypted fields"
        )));
    }
    if field.encrypted && field.auto.is_some() {
        return Err(schema_error(format!(
            "field '{path}': auto cannot be encrypted"
        )));
    }
    match field.field_type {
        FieldType::Int | FieldType::Float => {
            if field.min_length.is_some() || field.max_length.is_some() {
                return Err(schema_error(format!(
                    "field '{path}': min_length/max_length only apply to string/array"
                )));
            }
            if field.pattern.is_some() {
                return Err(schema_error(format!(
                    "field '{path}': pattern only applies to string"
                )));
            }
            if let (Some(min), Some(max)) = (field.min, field.max)
                && min > max
            {
                return Err(schema_error(format!(
                    "field '{path}': min greater than max"
                )));
            }
        }
        FieldType::String | FieldType::Array => {
            if field.min.is_some() || field.max.is_some() {
                return Err(schema_error(format!(
                    "field '{path}': min/max only apply to int/float"
                )));
            }
            if let (Some(min), Some(max)) = (field.min_length, field.max_length)
                && min > max
            {
                return Err(schema_error(format!(
                    "field '{path}': min_length greater than max_length"
                )));
            }
            if field.field_type == FieldType::Array && field.pattern.is_some() {
                return Err(schema_error(format!(
                    "field '{path}': pattern only applies to string"
                )));
            }
            if let Some(pattern) = &field.pattern {
                Regex::new(pattern).map_err(|err| {
                    schema_error(format!("field '{path}': invalid pattern: {err}"))
                })?;
            }
        }
        _ => {
            if field.min.is_some()
                || field.max.is_some()
                || field.min_length.is_some()
                || field.max_length.is_some()
                || field.pattern.is_some()
            {
                return Err(schema_error(format!(
                    "field '{path}': numeric/text constraints do not apply to the declared type"
                )));
            }
        }
    }
    if field.items.is_some() && field.field_type != FieldType::Array {
        return Err(schema_error(format!(
            "field '{path}': items only apply to array"
        )));
    }
    if field.fields.is_some() && field.field_type != FieldType::Object {
        return Err(schema_error(format!(
            "field '{path}': fields only apply to object"
        )));
    }
    if let Some(items) = &field.items {
        compile_field(&format!("{path}[]"), items, false)?;
    }
    if let Some(fields) = &field.fields {
        for (name, sub) in fields {
            compile_field(&format!("{path}.{name}"), sub, false)?;
        }
    }
    if let Some(enum_values) = &field.enum_values {
        if enum_values.is_empty() {
            return Err(schema_error(format!("field '{path}': empty enum")));
        }
        for value in enum_values {
            if !type_matches(field.field_type, value) {
                return Err(schema_error(format!(
                    "field '{path}': enum value incompatible with the type"
                )));
            }
        }
    }
    if let Some(auto) = &field.auto
        && field.field_type != FieldType::Datetime
    {
        let _ = auto;
        return Err(schema_error(format!(
            "field '{path}': auto only applies to datetime"
        )));
    }
    if let Some(default) = &field.default {
        let mut copy = default.clone();
        let mut errors = Vec::new();
        validate_value(path, field, &mut copy, &mut errors);
        if let Some(first) = errors.first() {
            return Err(schema_error(format!(
                "field '{path}': invalid default ({})",
                first.message
            )));
        }
    }
    Ok(())
}

fn type_matches(field_type: FieldType, value: &Value) -> bool {
    match field_type {
        FieldType::String => value.is_string(),
        FieldType::Int => value.as_i64().is_some() || value.as_u64().is_some(),
        FieldType::Float => value.is_f64() || value.as_i64().is_some() || value.as_u64().is_some(),
        FieldType::Bool => value.is_boolean(),
        FieldType::Datetime => value
            .as_str()
            .is_some_and(|text| DateTime::parse_from_rfc3339(text).is_ok()),
        FieldType::Date => value
            .as_str()
            .is_some_and(|text| NaiveDate::parse_from_str(text, "%Y-%m-%d").is_ok()),
        FieldType::Uuid => value
            .as_str()
            .is_some_and(|text| Uuid::parse_str(text).is_ok()),
        FieldType::Object => value.is_object(),
        FieldType::Array => value.is_array(),
        FieldType::Any => true,
    }
}

fn type_name(field_type: FieldType) -> &'static str {
    match field_type {
        FieldType::String => "string",
        FieldType::Int => "int",
        FieldType::Float => "float",
        FieldType::Bool => "bool",
        FieldType::Datetime => "datetime",
        FieldType::Date => "date",
        FieldType::Uuid => "uuid",
        FieldType::Object => "object",
        FieldType::Array => "array",
        FieldType::Any => "any",
    }
}

fn ferr(field: impl Into<String>, message: impl Into<String>) -> FieldError {
    FieldError {
        field: field.into(),
        message: message.into(),
        unique: false,
    }
}

fn ferr_unique(field: impl Into<String>, message: impl Into<String>) -> FieldError {
    FieldError {
        field: field.into(),
        message: message.into(),
        unique: true,
    }
}

fn now_value() -> Value {
    json!(Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true))
}

fn validate_value(
    path: &str,
    field: &FieldSchema,
    value: &mut Value,
    errors: &mut Vec<FieldError>,
) {
    if !type_matches(field.field_type, value) {
        errors.push(ferr(
            path,
            format!("invalid type: expected '{}'", type_name(field.field_type)),
        ));
        return;
    }
    match field.field_type {
        FieldType::Int | FieldType::Float => {
            if let Some(number) = value.as_f64() {
                if let Some(min) = field.min
                    && number < min
                {
                    errors.push(ferr(path, format!("value less than the minimum ({min})")));
                }
                if let Some(max) = field.max
                    && number > max
                {
                    errors.push(ferr(
                        path,
                        format!("value greater than the maximum ({max})"),
                    ));
                }
            }
        }
        FieldType::String => {
            if let Some(text) = value.as_str() {
                let len = text.chars().count();
                if let Some(min) = field.min_length
                    && len < min
                {
                    errors.push(ferr(path, format!("text shorter than min_length ({min})")));
                }
                if let Some(max) = field.max_length
                    && len > max
                {
                    errors.push(ferr(path, format!("text longer than max_length ({max})")));
                }
                if let Some(pattern) = &field.pattern
                    && let Ok(regex) = crate::regex_cache::compiled(pattern)
                    && !regex.is_match(text)
                {
                    errors.push(ferr(path, "value does not match the pattern"));
                }
            }
        }
        FieldType::Array => {
            if let Some(items) = value.as_array_mut() {
                let len = items.len();
                if let Some(min) = field.min_length
                    && len < min
                {
                    errors.push(ferr(path, format!("array shorter than min_length ({min})")));
                }
                if let Some(max) = field.max_length
                    && len > max
                {
                    errors.push(ferr(path, format!("array longer than max_length ({max})")));
                }
                if let Some(item_schema) = &field.items {
                    for (index, item) in items.iter_mut().enumerate() {
                        validate_value(&format!("{path}[{index}]"), item_schema, item, errors);
                    }
                }
            }
        }
        FieldType::Object => {
            if let Some(fields) = &field.fields
                && let Some(object) = value.as_object_mut()
            {
                for (name, sub) in fields {
                    let sub_path = format!("{path}.{name}");
                    match object.get_mut(name) {
                        None => {
                            if let Some(default) = &sub.default {
                                object.insert(name.clone(), default.clone());
                            } else if sub.required {
                                errors.push(ferr(sub_path, "required field missing"));
                            }
                        }
                        Some(sub_value) => {
                            if sub_value.is_null() {
                                if !sub.nullable {
                                    errors.push(ferr(sub_path, "cannot be null"));
                                }
                            } else {
                                validate_value(&sub_path, sub, sub_value, errors);
                            }
                        }
                    }
                }
            }
        }
        _ => {}
    }
    if let Some(enum_values) = &field.enum_values
        && !enum_values.contains(value)
    {
        errors.push(ferr(path, "value outside the enum"));
    }
}

pub fn prepare_record(
    schema: &TableSchema,
    record: &mut Value,
    mode: Mode,
    previous: Option<&Value>,
    unique: &UniqueIndex,
) -> Result<(), Vec<FieldError>> {
    let mut errors = Vec::new();
    let Some(object) = record.as_object_mut() else {
        return Err(vec![ferr("_", "record must be a JSON object")]);
    };
    for (name, field) in &schema.fields {
        if let Some(auto) = field.auto {
            let value = match (auto, mode) {
                (AutoField::CreatedAt, Mode::Create) => Some(now_value()),
                (AutoField::CreatedAt, Mode::Replace) => Some(
                    previous
                        .and_then(|prev| prev.get(name))
                        .cloned()
                        .unwrap_or_else(now_value),
                ),
                (AutoField::UpdatedAt, Mode::Create | Mode::Replace) => Some(now_value()),
                (_, Mode::Migrate) => None,
            };
            if let Some(value) = value {
                object.insert(name.clone(), value);
            }
        }
        if object.get(name).is_none() {
            if let Some(default) = &field.default {
                object.insert(name.clone(), default.clone());
            } else if field.required {
                errors.push(ferr(name, "required field missing"));
                continue;
            } else {
                continue;
            }
        }
        let value = object.get_mut(name).expect("inserted above");
        if value.is_null() {
            if !field.nullable {
                errors.push(ferr(name, "cannot be null"));
            }
            continue;
        }
        validate_value(name, field, value, &mut errors);
        if field.unique {
            let key = canonical(value);
            if unique.has_single(name, &key) {
                errors.push(ferr_unique(name, "value already exists (unique)"));
            }
        }
    }
    for key in &schema.unique {
        let fields = key.fields();
        let Some(combo) = unique_combo(object, &fields) else {
            continue;
        };
        if unique.has_compound(&fields, &combo) {
            errors.push(ferr_unique(
                key.label(),
                "combination already exists (unique)",
            ));
        }
    }
    if schema.strict {
        for key in object.keys() {
            if !schema.fields.contains_key(key) {
                errors.push(ferr(key, "unknown field (strict schema)"));
            }
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

fn unique_combo(object: &Map<String, Value>, fields: &[String]) -> Option<String> {
    let mut values = Vec::with_capacity(fields.len());
    for name in fields {
        let value = object.get(name)?;
        if value.is_null() {
            return None;
        }
        values.push(value.clone());
    }
    Some(canonical(&Value::Array(values)))
}

pub fn index_unique_fields(index: &mut UniqueIndex, schema: &TableSchema, record: &Value) {
    for (name, field) in &schema.fields {
        if !field.unique {
            continue;
        }
        if let Some(value) = record.get(name)
            && !value.is_null()
        {
            index.add_single(name, canonical(value));
        }
    }
    if let Some(object) = record.as_object() {
        for key in &schema.unique {
            let fields = key.fields();
            if let Some(combo) = unique_combo(object, &fields) {
                index.add_compound(&fields, combo);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn table_schema(value: Value) -> TableSchema {
        serde_json::from_value(value).expect("valid schema")
    }

    fn base_schema() -> TableSchema {
        table_schema(json!({
            "unique": ["cpf", ["tenant", "email"]],
            "fields": {
                "tenant": {"type": "string"},
                "email": {"type": "string"},
                "cpf": {"type": "string", "nullable": true},
                "name": {"type": "string", "required": true},
                "age": {"type": "int", "min": 0, "max": 150},
                "active": {"type": "bool", "default": true},
                "password": {"type": "string", "encrypted": true},
                "tags": {"type": "array", "items": {"type": "string"}},
                "address": {"type": "object", "fields": {"city": {"type": "string", "required": true}}},
                "created": {"type": "datetime", "auto": "created_at"},
                "updated": {"type": "datetime", "auto": "updated_at"}
            }
        }))
    }

    #[test]
    fn compile_accepts_valid_schema_and_labels() {
        let schema = base_schema();
        assert!(compile(&schema).is_ok());
        assert_eq!(schema.unique[0].label(), "cpf");
        assert_eq!(schema.unique[1].label(), "tenant+email");
    }

    #[test]
    fn compile_rejects_bad_unique_keys() {
        let unknown = table_schema(json!({"unique": ["nope"], "fields": {}}));
        assert!(compile(&unknown).is_err());
        let encrypted = table_schema(json!({
            "unique": ["password"],
            "fields": {"password": {"type": "string", "encrypted": true}}
        }));
        assert!(compile(&encrypted).is_err());
        let repeated = table_schema(json!({
            "unique": [["a", "a"]],
            "fields": {"a": {"type": "string"}}
        }));
        assert!(compile(&repeated).is_err());
        let duplicated = table_schema(json!({
            "unique": ["a", ["a"]],
            "fields": {"a": {"type": "string"}}
        }));
        assert!(compile(&duplicated).is_err());
    }

    #[test]
    fn compile_rejects_bad_field_constraints() {
        let pattern = table_schema(json!({
            "fields": {"a": {"type": "string", "pattern": "(unclosed"}}
        }));
        assert!(compile(&pattern).is_err());
        let range = table_schema(json!({
            "fields": {"a": {"type": "int", "min": 10, "max": 1}}
        }));
        assert!(compile(&range).is_err());
        let nested_encrypted = table_schema(json!({
            "fields": {"a": {"type": "object", "fields": {"b": {"type": "string", "encrypted": true}}}}
        }));
        assert!(compile(&nested_encrypted).is_err());
        let bad_default = table_schema(json!({
            "fields": {"a": {"type": "int", "default": "text"}}
        }));
        assert!(compile(&bad_default).is_err());
    }

    #[test]
    fn prepare_applies_defaults_auto_and_required() {
        let schema = base_schema();
        let mut record = json!({"name": "Ana", "password": "s", "tenant": "t", "email": "a@x"});
        prepare_record(
            &schema,
            &mut record,
            Mode::Create,
            None,
            &UniqueIndex::new(),
        )
        .unwrap();
        assert_eq!(record["active"], json!(true));
        assert!(record["created"].is_string());
        assert!(record["updated"].is_string());

        let missing = prepare_record(
            &schema,
            &mut json!({}),
            Mode::Create,
            None,
            &UniqueIndex::new(),
        )
        .unwrap_err();
        assert!(missing.iter().any(|error| error.field == "name"));
    }

    #[test]
    fn prepare_rejects_wrong_types_and_strict_fields() {
        let schema = base_schema();
        let mut record = json!({
            "name": "Ana",
            "age": 200,
            "tags": [1, 2],
            "address": {},
            "extra": true
        });
        let errors = prepare_record(
            &schema,
            &mut record,
            Mode::Create,
            None,
            &UniqueIndex::new(),
        )
        .unwrap_err();
        let fields = errors
            .iter()
            .map(|error| error.field.as_str())
            .collect::<Vec<_>>();
        assert!(fields.contains(&"age"));
        assert!(fields.contains(&"tags[0]"));
        assert!(fields.contains(&"address.city"));

        let strict = table_schema(json!({
            "strict": true,
            "fields": {"name": {"type": "string"}}
        }));
        let errors = prepare_record(
            &strict,
            &mut json!({"name": "Ana", "extra": 1}),
            Mode::Create,
            None,
            &UniqueIndex::new(),
        )
        .unwrap_err();
        assert!(errors.iter().any(|error| error.field == "extra"));
    }

    #[test]
    fn unique_single_compound_and_nulls() {
        let schema = base_schema();
        let mut index = UniqueIndex::new();
        index_unique_fields(
            &mut index,
            &schema,
            &json!({"cpf": "111", "tenant": "t1", "email": "a@x"}),
        );
        assert!(index.has_compound(&["cpf".to_string()], &canonical(&json!(["111"]))));
        let fields = vec!["tenant".to_string(), "email".to_string()];
        assert!(index.has_compound(&fields, &canonical(&json!(["t1", "a@x"]))));

        let errors = prepare_record(
            &schema,
            &mut json!({"name": "B", "cpf": "111"}),
            Mode::Create,
            None,
            &index,
        )
        .unwrap_err();
        assert!(
            errors
                .iter()
                .any(|error| error.unique && error.field == "cpf")
        );

        let errors = prepare_record(
            &schema,
            &mut json!({"name": "B", "cpf": "222", "tenant": "t1", "email": "a@x"}),
            Mode::Create,
            None,
            &index,
        )
        .unwrap_err();
        assert!(
            errors
                .iter()
                .any(|error| error.unique && error.field == "tenant+email")
        );

        let errors = prepare_record(
            &schema,
            &mut json!({"name": "B", "cpf": null, "tenant": "t1", "email": "a@x"}),
            Mode::Create,
            None,
            &index,
        )
        .unwrap_err();
        assert!(
            errors.iter().all(|error| error.field != "cpf"),
            "cpf null should not violate unique: {errors:?}"
        );
        assert!(
            errors
                .iter()
                .any(|error| error.unique && error.field == "tenant+email")
        );

        assert!(
            prepare_record(
                &schema,
                &mut json!({"name": "B", "cpf": null, "tenant": "t2", "email": "b@x"}),
                Mode::Create,
                None,
                &index,
            )
            .is_ok(),
            "null components do not participate in unique"
        );
    }

    #[test]
    fn replace_preserves_created_at() {
        let schema = base_schema();
        let mut created = json!({"name": "Ana"});
        prepare_record(
            &schema,
            &mut created,
            Mode::Create,
            None,
            &UniqueIndex::new(),
        )
        .unwrap();
        let created_at = created["created"].clone();

        let mut replaced = json!({"name": "Ana Maria"});
        prepare_record(
            &schema,
            &mut replaced,
            Mode::Replace,
            Some(&created),
            &UniqueIndex::new(),
        )
        .unwrap();
        assert_eq!(replaced["created"], created_at);
    }

    fn patch(value: Value) -> SchemaPatch {
        serde_json::from_value(value).expect("valid patch")
    }

    #[test]
    fn patch_creates_schema_from_scratch() {
        let mut schema = TableSchema::default();
        let operation = patch(json!({
            "add": {"name": {"type": "string", "required": true}},
            "unique_add": ["name"],
            "strict": true
        }));
        apply_patch(&mut schema, &operation).unwrap();
        assert!(schema.strict);
        assert_eq!(schema.fields.len(), 1);
        assert_eq!(schema.fields["name"].field_type, FieldType::String);
        assert_eq!(schema.unique[0].label(), "name");
    }

    #[test]
    fn patch_rejects_invalid_operations() {
        let mut schema = base_schema();
        let add_existing = patch(json!({"add": {"name": {"type": "string"}}}));
        assert!(apply_patch(&mut schema, &add_existing).is_err());

        let remove_missing = patch(json!({"remove": ["does_not_exist"]}));
        assert!(apply_patch(&mut schema, &remove_missing).is_err());

        let update_missing = patch(json!({"update": {"does_not_exist": {"required": true}}}));
        assert!(apply_patch(&mut schema, &update_missing).is_err());

        let unique_exists = patch(json!({"unique_add": ["cpf"]}));
        assert!(apply_patch(&mut schema, &unique_exists).is_err());

        let unique_missing = patch(json!({"unique_remove": ["does_not_exist"]}));
        assert!(apply_patch(&mut schema, &unique_missing).is_err());

        let bad_field = patch(json!({"add": {"x": {"type": "nonexistent"}}}));
        assert!(apply_patch(&mut schema, &bad_field).is_err());

        let unknown_unique_field =
            patch(json!({"add": {"y": {"type": "string"}}, "unique_add": ["not_declared"]}));
        assert!(apply_patch(&mut schema, &unknown_unique_field).is_err());
    }

    #[test]
    fn patch_update_merges_and_clears_properties() {
        let mut schema = base_schema();
        let operation = patch(json!({
            "update": {
                "age": {"min": 5, "max": null},
                "cpf": {"required": true},
                "name": {"min_length": 3, "max_length": 40}
            }
        }));
        apply_patch(&mut schema, &operation).unwrap();
        assert_eq!(schema.fields["age"].min, Some(5.0));
        assert_eq!(schema.fields["age"].max, None);
        assert!(schema.fields["cpf"].required);
        assert_eq!(schema.fields["name"].min_length, Some(3));
        assert_eq!(schema.fields["name"].max_length, Some(40));

        let bad_type = patch(json!({"update": {"age": {"type": "does_not_exist"}}}));
        assert!(apply_patch(&mut schema, &bad_type).is_err());
    }

    #[test]
    fn patch_remove_field_and_unique() {
        let mut schema = base_schema();
        let operation = patch(json!({
            "remove": ["tags", "address"],
            "unique_remove": ["cpf"]
        }));
        apply_patch(&mut schema, &operation).unwrap();
        assert!(!schema.fields.contains_key("tags"));
        assert!(!schema.fields.contains_key("address"));
        assert_eq!(schema.unique.len(), 1);
        assert_eq!(schema.unique[0].label(), "tenant+email");
    }
}
