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
    AppError::BadRequest(format!("schema invalido: {}", message.into()))
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
            return Err(schema_error("unique: chave vazia"));
        }
        let mut components = BTreeSet::new();
        for name in &fields {
            if name.is_empty() {
                return Err(schema_error("unique: nome de campo vazio"));
            }
            if !components.insert(name.clone()) {
                return Err(schema_error(format!(
                    "unique: campo '{name}' repetido na chave '{}'",
                    key.label()
                )));
            }
            let Some(field) = schema.fields.get(name) else {
                return Err(schema_error(format!(
                    "unique: campo '{name}' nao declarado no schema"
                )));
            };
            if field.encrypted {
                return Err(schema_error(format!(
                    "unique: campo '{name}' nao pode ser encrypted"
                )));
            }
        }
        let label = key.label();
        if !seen.insert(label.clone()) {
            return Err(schema_error(format!("unique: chave '{label}' duplicada")));
        }
    }
    Ok(())
}

fn compile_field(path: &str, field: &FieldSchema, top: bool) -> Result<(), AppError> {
    let leaf = path.rsplit('.').next().unwrap_or(path);
    if leaf.starts_with('_') {
        return Err(schema_error(format!(
            "campo '{path}': nomes iniciados por '_' sao reservados"
        )));
    }
    if !top {
        if field.encrypted {
            return Err(schema_error(format!(
                "campo '{path}': encrypted so e suportado em campos de primeiro nivel"
            )));
        }
        if field.unique {
            return Err(schema_error(format!(
                "campo '{path}': unique so e suportado em campos de primeiro nivel"
            )));
        }
        if field.auto.is_some() {
            return Err(schema_error(format!(
                "campo '{path}': auto so e suportado em campos de primeiro nivel"
            )));
        }
    }
    if field.encrypted && field.unique {
        return Err(schema_error(format!(
            "campo '{path}': unique nao e suportado em campo encrypted"
        )));
    }
    if field.encrypted && field.auto.is_some() {
        return Err(schema_error(format!(
            "campo '{path}': auto nao pode ser encrypted"
        )));
    }
    match field.field_type {
        FieldType::Int | FieldType::Float => {
            if field.min_length.is_some() || field.max_length.is_some() {
                return Err(schema_error(format!(
                    "campo '{path}': min_length/max_length so valem para string/array"
                )));
            }
            if field.pattern.is_some() {
                return Err(schema_error(format!(
                    "campo '{path}': pattern so vale para string"
                )));
            }
            if let (Some(min), Some(max)) = (field.min, field.max)
                && min > max
            {
                return Err(schema_error(format!("campo '{path}': min maior que max")));
            }
        }
        FieldType::String | FieldType::Array => {
            if field.min.is_some() || field.max.is_some() {
                return Err(schema_error(format!(
                    "campo '{path}': min/max so valem para int/float"
                )));
            }
            if let (Some(min), Some(max)) = (field.min_length, field.max_length)
                && min > max
            {
                return Err(schema_error(format!(
                    "campo '{path}': min_length maior que max_length"
                )));
            }
            if field.field_type == FieldType::Array && field.pattern.is_some() {
                return Err(schema_error(format!(
                    "campo '{path}': pattern so vale para string"
                )));
            }
            if let Some(pattern) = &field.pattern {
                Regex::new(pattern).map_err(|err| {
                    schema_error(format!("campo '{path}': pattern invalido: {err}"))
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
                    "campo '{path}': constraints numericas/de texto nao valem para o tipo declarado"
                )));
            }
        }
    }
    if field.items.is_some() && field.field_type != FieldType::Array {
        return Err(schema_error(format!(
            "campo '{path}': items so vale para array"
        )));
    }
    if field.fields.is_some() && field.field_type != FieldType::Object {
        return Err(schema_error(format!(
            "campo '{path}': fields so vale para object"
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
            return Err(schema_error(format!("campo '{path}': enum vazio")));
        }
        for value in enum_values {
            if !type_matches(field.field_type, value) {
                return Err(schema_error(format!(
                    "campo '{path}': valor do enum incompativel com o tipo"
                )));
            }
        }
    }
    if let Some(auto) = &field.auto
        && field.field_type != FieldType::Datetime
    {
        let _ = auto;
        return Err(schema_error(format!(
            "campo '{path}': auto so vale para datetime"
        )));
    }
    if let Some(default) = &field.default {
        let mut copy = default.clone();
        let mut errors = Vec::new();
        validate_value(path, field, &mut copy, &mut errors);
        if let Some(first) = errors.first() {
            return Err(schema_error(format!(
                "campo '{path}': default invalido ({})",
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
        FieldType::Float => {
            value.is_f64() || value.as_i64().is_some() || value.as_u64().is_some()
        }
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

fn validate_value(path: &str, field: &FieldSchema, value: &mut Value, errors: &mut Vec<FieldError>) {
    if !type_matches(field.field_type, value) {
        errors.push(ferr(
            path,
            format!("tipo invalido: esperado '{}'", type_name(field.field_type)),
        ));
        return;
    }
    match field.field_type {
        FieldType::Int | FieldType::Float => {
            if let Some(number) = value.as_f64() {
                if let Some(min) = field.min
                    && number < min
                {
                    errors.push(ferr(path, format!("valor menor que o minimo ({min})")));
                }
                if let Some(max) = field.max
                    && number > max
                {
                    errors.push(ferr(path, format!("valor maior que o maximo ({max})")));
                }
            }
        }
        FieldType::String => {
            if let Some(text) = value.as_str() {
                let len = text.chars().count();
                if let Some(min) = field.min_length
                    && len < min
                {
                    errors.push(ferr(path, format!("texto menor que min_length ({min})")));
                }
                if let Some(max) = field.max_length
                    && len > max
                {
                    errors.push(ferr(path, format!("texto maior que max_length ({max})")));
                }
                if let Some(pattern) = &field.pattern
                    && let Ok(regex) = Regex::new(pattern)
                    && !regex.is_match(text)
                {
                    errors.push(ferr(path, "valor nao corresponde ao pattern"));
                }
            }
        }
        FieldType::Array => {
            if let Some(items) = value.as_array_mut() {
                let len = items.len();
                if let Some(min) = field.min_length
                    && len < min
                {
                    errors.push(ferr(path, format!("array menor que min_length ({min})")));
                }
                if let Some(max) = field.max_length
                    && len > max
                {
                    errors.push(ferr(path, format!("array maior que max_length ({max})")));
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
                                errors.push(ferr(sub_path, "campo obrigatorio ausente"));
                            }
                        }
                        Some(sub_value) => {
                            if sub_value.is_null() {
                                if !sub.nullable {
                                    errors.push(ferr(sub_path, "nao pode ser nulo"));
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
        errors.push(ferr(path, "valor fora do enum"));
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
        return Err(vec![ferr("_", "registro deve ser um objeto JSON")]);
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
                errors.push(ferr(name, "campo obrigatorio ausente"));
                continue;
            } else {
                continue;
            }
        }
        let value = object.get_mut(name).expect("inserido acima");
        if value.is_null() {
            if !field.nullable {
                errors.push(ferr(name, "nao pode ser nulo"));
            }
            continue;
        }
        validate_value(name, field, value, &mut errors);
        if field.unique {
            let key = canonical(value);
            if unique.has_single(name, &key) {
                errors.push(ferr_unique(name, "valor ja existe (unique)"));
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
                "combinacao ja existe (unique)",
            ));
        }
    }
    if schema.strict {
        for key in object.keys() {
            if !schema.fields.contains_key(key) {
                errors.push(ferr(key, "campo desconhecido (schema strict)"));
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
