use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value};
use std::cmp::Ordering;
use std::collections::BTreeMap;

use crate::error::AppError;

/// Sort applied when the caller does not provide one: records keyed by `_id`,
/// which is guaranteed to be unique and stable.
pub const ID_SORT_FIELD: &str = "_id";

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct QueryRequest {
    #[serde(default)]
    pub filter: Option<Value>,
    #[serde(default, deserialize_with = "deserialize_sort")]
    pub sort: Vec<SortSpec>,
    #[serde(default)]
    pub fields: Option<Vec<String>>,
    #[serde(default)]
    pub limit: Option<usize>,
    #[serde(default)]
    pub offset: Option<usize>,
    /// Opaque keyset cursor returned as `next_cursor` by a previous page.
    #[serde(default)]
    pub cursor: Option<String>,
}

/// Decoded keyset cursor: the values of the sort tuple (including the `_id`
/// tiebreaker) of the last record of the previous page.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CursorData {
    pub k: Vec<Value>,
}

/// The sort used for a page: the requested one, or `_id` ascending by default.
pub fn effective_sort(sort: &[SortSpec]) -> Vec<SortSpec> {
    if sort.is_empty() {
        return vec![SortSpec {
            field: ID_SORT_FIELD.to_string(),
            dir: SortDir::Asc,
        }];
    }
    sort.to_vec()
}

/// Appends the sort, `_id` ascending, to the key so ties are broken deterministically.
pub fn cursor_key(record: &Value, sort: &[SortSpec]) -> Result<Vec<Value>, AppError> {
    let mut key = Vec::with_capacity(sort.len() + 1);
    for spec in sort {
        if spec.field == ID_SORT_FIELD {
            continue;
        }
        key.push(
            get_path(record, &spec.field)
                .filter(|value| !value.is_null())
                .cloned()
                .unwrap_or(Value::Null),
        );
    }
    key.push(
        record
            .get(ID_SORT_FIELD)
            .filter(|value| !value.is_null())
            .cloned()
            .unwrap_or(Value::Null),
    );
    Ok(key)
}

pub fn encode_cursor(key: &[Value]) -> Result<String, AppError> {
    let data = CursorData { k: key.to_vec() };
    let bytes = serde_json::to_vec(&data)
        .map_err(|err| AppError::BadRequest(format!("could not encode cursor: {err}")))?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

/// Decodes a cursor. The key shape is `filter_sorts + _id`, so its arity is the
/// number of sort fields (excluding `_id`) plus one.
pub fn decode_cursor(cursor: &str, sort: &[SortSpec]) -> Result<CursorData, AppError> {
    let invalid = || AppError::BadRequest("invalid cursor".to_string());
    let bytes = URL_SAFE_NO_PAD
        .decode(cursor.as_bytes())
        .map_err(|_| invalid())?;
    let data: CursorData = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
    if data.k.len() != filter_sorts(sort).len() + 1 {
        return Err(invalid());
    }
    Ok(data)
}

/// The requested sort without the `_id` tiebreaker, which is always appended last.
pub fn filter_sorts(sort: &[SortSpec]) -> Vec<SortSpec> {
    sort.iter()
        .filter(|spec| spec.field != ID_SORT_FIELD)
        .cloned()
        .collect()
}

/// Compares a cursor key against a record key following `sort` order, applying
/// the `_id` tiebreaker (ascending) at the end.
pub fn key_after(key: &[Value], after: &[Value], sort: &[SortSpec]) -> bool {
    for (index, spec) in filter_sorts(sort).iter().enumerate() {
        let ordering = compare_component(
            key.get(index).unwrap_or(&Value::Null),
            after.get(index).unwrap_or(&Value::Null),
            spec.dir,
        );
        if ordering != Ordering::Equal {
            return ordering == Ordering::Greater;
        }
    }
    let last = filter_sorts(sort).len();
    let ordering = compare_component(
        key.get(last).unwrap_or(&Value::Null),
        after.get(last).unwrap_or(&Value::Null),
        SortDir::Asc,
    );
    ordering == Ordering::Greater
}

fn compare_component(left: &Value, right: &Value, dir: SortDir) -> Ordering {
    let ordering = match (left.is_null(), right.is_null()) {
        (true, true) => Ordering::Equal,
        (true, false) => Ordering::Greater,
        (false, true) => Ordering::Less,
        (false, false) => value_cmp(left, right),
    };
    match dir {
        SortDir::Asc => ordering,
        SortDir::Desc => ordering.reverse(),
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SortDir {
    #[default]
    Asc,
    Desc,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SortSpec {
    pub field: String,
    #[serde(default, alias = "direction", alias = "order")]
    pub dir: SortDir,
}

fn deserialize_sort<'de, D>(deserializer: D) -> Result<Vec<SortSpec>, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Input {
        Field(String),
        Spec(SortSpec),
    }
    let inputs = Vec::<Input>::deserialize(deserializer)?;
    Ok(inputs
        .into_iter()
        .map(|input| match input {
            Input::Spec(spec) => spec,
            Input::Field(field) => match field.strip_prefix('-') {
                Some(rest) => SortSpec {
                    field: rest.to_string(),
                    dir: SortDir::Desc,
                },
                None => SortSpec {
                    field,
                    dir: SortDir::Asc,
                },
            },
        })
        .collect())
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct AggregateRequest {
    #[serde(default)]
    pub filter: Option<Value>,
    #[serde(default)]
    pub group_by: Vec<String>,
    pub aggregates: BTreeMap<String, AggregateSpec>,
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AggregateSpec {
    pub op: AggOp,
    #[serde(default)]
    pub field: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AggOp {
    Count,
    Sum,
    Avg,
    Min,
    Max,
}

fn aggregate_error(message: impl Into<String>) -> AppError {
    AppError::BadRequest(format!("invalid aggregation: {}", message.into()))
}

fn group_key(record: &Value, group_by: &[String]) -> Value {
    if group_by.is_empty() {
        return Value::Object(Map::new());
    }
    let mut object = Map::new();
    for path in group_by {
        let value = get_path(record, path).cloned().unwrap_or(Value::Null);
        object.insert(path.clone(), value);
    }
    Value::Object(object)
}

fn numeric(value: f64) -> Value {
    if value.fract() == 0.0 && value >= i64::MIN as f64 && value <= i64::MAX as f64 {
        serde_json::json!(value as i64)
    } else {
        serde_json::Number::from_f64(value)
            .map(Value::Number)
            .unwrap_or(Value::Null)
    }
}

#[derive(Debug)]
struct Accumulator {
    op: AggOp,
    sum: f64,
    count: u64,
    min: Option<f64>,
    max: Option<f64>,
}

impl Accumulator {
    fn new(op: AggOp) -> Self {
        Self {
            op,
            sum: 0.0,
            count: 0,
            min: None,
            max: None,
        }
    }

    fn push(&mut self, value: Option<&Value>) {
        if self.op == AggOp::Count {
            return;
        }
        let Some(number) = value.and_then(Value::as_f64) else {
            return;
        };
        self.sum += number;
        self.count += 1;
        self.min = Some(self.min.map_or(number, |current| current.min(number)));
        self.max = Some(self.max.map_or(number, |current| current.max(number)));
    }

    fn value(&self) -> Value {
        match self.op {
            AggOp::Count => Value::Null,
            AggOp::Sum => {
                if self.count == 0 {
                    Value::Null
                } else {
                    numeric(self.sum)
                }
            }
            AggOp::Avg => {
                if self.count == 0 {
                    Value::Null
                } else {
                    numeric(self.sum / self.count as f64)
                }
            }
            AggOp::Min => self.min.map(numeric).unwrap_or(Value::Null),
            AggOp::Max => self.max.map(numeric).unwrap_or(Value::Null),
        }
    }
}

struct AggregateGroup {
    key: Value,
    count: u64,
    accumulators: Vec<Accumulator>,
}

/// Groups `records` by `group_by` and applies the requested aggregations.
pub fn aggregate(records: &[Value], request: &AggregateRequest) -> Result<Vec<Value>, AppError> {
    let specs = request.aggregates.iter().collect::<Vec<_>>();
    for (name, spec) in &specs {
        if spec.op != AggOp::Count && spec.field.is_none() {
            return Err(aggregate_error(format!(
                "'{name}': operation {:?} requires the 'field' field",
                spec.op
            )));
        }
    }
    let mut groups: BTreeMap<String, AggregateGroup> = BTreeMap::new();
    for record in records {
        let key = group_key(record, &request.group_by);
        let group = groups
            .entry(canonical(&key))
            .or_insert_with(|| AggregateGroup {
                key: key.clone(),
                count: 0,
                accumulators: specs
                    .iter()
                    .map(|(_, spec)| Accumulator::new(spec.op))
                    .collect(),
            });
        group.count += 1;
        for ((_, spec), accumulator) in specs.iter().zip(group.accumulators.iter_mut()) {
            let value = spec
                .field
                .as_deref()
                .and_then(|field| get_path(record, field));
            accumulator.push(value);
        }
    }
    let mut output = Vec::with_capacity(groups.len());
    for group in groups.into_values() {
        let mut object = Map::new();
        object.insert("key".to_string(), group.key);
        object.insert("count".to_string(), serde_json::json!(group.count));
        for ((name, spec), accumulator) in specs.iter().zip(group.accumulators.iter()) {
            let value = if spec.op == AggOp::Count {
                serde_json::json!(group.count)
            } else {
                accumulator.value()
            };
            object.insert((*name).clone(), value);
        }
        output.push(Value::Object(object));
    }
    if let Some(limit) = request.limit {
        output.truncate(limit);
    }
    Ok(output)
}

fn filter_error(message: impl Into<String>) -> AppError {
    AppError::BadRequest(format!("invalid filter: {}", message.into()))
}

pub fn get_path<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    let mut current = value;
    for part in path.split('.') {
        current = match current {
            Value::Object(map) => map.get(part)?,
            Value::Array(items) => items.get(part.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }
    Some(current)
}

pub fn matches(filter: &Value, record: &Value) -> Result<bool, AppError> {
    let Some(object) = filter.as_object() else {
        return Err(filter_error("must be a JSON object"));
    };
    for (key, condition) in object {
        let matched = match key.as_str() {
            "$and" => {
                let list = condition
                    .as_array()
                    .ok_or_else(|| filter_error("$and expects an array"))?;
                let mut result = true;
                for item in list {
                    if !matches(item, record)? {
                        result = false;
                        break;
                    }
                }
                result
            }
            "$or" => {
                let list = condition
                    .as_array()
                    .ok_or_else(|| filter_error("$or expects an array"))?;
                let mut result = false;
                for item in list {
                    if matches(item, record)? {
                        result = true;
                        break;
                    }
                }
                result
            }
            "$not" => !matches(condition, record)?,
            other if other.starts_with('$') => {
                return Err(filter_error(format!("unknown operator '{other}'")));
            }
            field => match_path(record, field, condition)?,
        };
        if !matched {
            return Ok(false);
        }
    }
    Ok(true)
}

fn match_path(record: &Value, path: &str, condition: &Value) -> Result<bool, AppError> {
    let value = get_path(record, path);
    match condition {
        Value::Object(map) if map.keys().any(|key| key.starts_with('$')) => {
            for (op, operand) in map {
                if !op.starts_with('$') {
                    return Err(filter_error(format!(
                        "field '{path}': do not mix operators and values"
                    )));
                }
                if !apply_op(path, value, op, operand)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        other => Ok(value == Some(other)),
    }
}

fn apply_op(
    path: &str,
    value: Option<&Value>,
    op: &str,
    operand: &Value,
) -> Result<bool, AppError> {
    match op {
        "$eq" => Ok(value == Some(operand)),
        "$ne" => Ok(value != Some(operand)),
        "$exists" => {
            let expected = operand
                .as_bool()
                .ok_or_else(|| filter_error(format!("field '{path}': $exists expects boolean")))?;
            let exists = value.is_some_and(|item| !item.is_null());
            Ok(exists == expected)
        }
        "$gt" | "$gte" | "$lt" | "$lte" => {
            let Some(current) = value else {
                return Ok(false);
            };
            let Some(ordering) = compare(current, operand) else {
                return Ok(false);
            };
            Ok(match op {
                "$gt" => ordering == Ordering::Greater,
                "$gte" => ordering != Ordering::Less,
                "$lt" => ordering == Ordering::Less,
                _ => ordering != Ordering::Greater,
            })
        }
        "$in" => {
            let list = operand
                .as_array()
                .ok_or_else(|| filter_error(format!("field '{path}': $in expects array")))?;
            Ok(list.iter().any(|item| value == Some(item)))
        }
        "$nin" => {
            let list = operand
                .as_array()
                .ok_or_else(|| filter_error(format!("field '{path}': $nin expects array")))?;
            Ok(!list.iter().any(|item| value == Some(item)))
        }
        "$contains" => match value {
            Some(Value::String(text)) => {
                let needle = operand.as_str().ok_or_else(|| {
                    filter_error(format!("field '{path}': $contains expects string"))
                })?;
                Ok(text.contains(needle))
            }
            Some(Value::Array(items)) => Ok(items.iter().any(|item| item == operand)),
            _ => Ok(false),
        },
        "$starts_with" | "$ends_with" => {
            let Some(Value::String(text)) = value else {
                return Ok(false);
            };
            let needle = operand
                .as_str()
                .ok_or_else(|| filter_error(format!("field '{path}': {op} expects string")))?;
            Ok(match op {
                "$starts_with" => text.starts_with(needle),
                _ => text.ends_with(needle),
            })
        }
        "$regex" => {
            let pattern = operand
                .as_str()
                .ok_or_else(|| filter_error(format!("field '{path}': $regex expects string")))?;
            let regex = crate::regex_cache::compiled(pattern)
                .map_err(|err| filter_error(format!("field '{path}': invalid $regex: {err}")))?;
            let Some(Value::String(text)) = value else {
                return Ok(false);
            };
            Ok(regex.is_match(text))
        }
        other => Err(filter_error(format!("unknown operator '{other}'"))),
    }
}

fn compare(a: &Value, b: &Value) -> Option<Ordering> {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x.as_f64()?.partial_cmp(&y.as_f64()?),
        (Value::String(x), Value::String(y)) => Some(x.cmp(y)),
        (Value::Bool(x), Value::Bool(y)) => Some(x.cmp(y)),
        _ => None,
    }
}

pub fn sort_entries<T>(entries: &mut [T], sort: &[SortSpec], value_of: impl Fn(&T) -> &Value) {
    if sort.is_empty() {
        return;
    }
    entries.sort_by(|a, b| {
        let a = value_of(a);
        let b = value_of(b);
        for spec in sort {
            // `_id` is unique: it cannot tie, so it is the total tiebreaker.
            if spec.field == ID_SORT_FIELD {
                continue;
            }
            let left = get_path(a, &spec.field).filter(|value| !value.is_null());
            let right = get_path(b, &spec.field).filter(|value| !value.is_null());
            let ordering = match (left, right) {
                (None, None) => Ordering::Equal,
                (None, Some(_)) => Ordering::Greater,
                (Some(_), None) => Ordering::Less,
                (Some(left), Some(right)) => {
                    let ordering = value_cmp(left, right);
                    match spec.dir {
                        SortDir::Asc => ordering,
                        SortDir::Desc => ordering.reverse(),
                    }
                }
            };
            if ordering != Ordering::Equal {
                return ordering;
            }
        }
        // `_id` ascending, so paging by cursor never repeats or skips a record
        let left = a.get(ID_SORT_FIELD).filter(|value| !value.is_null());
        let right = b.get(ID_SORT_FIELD).filter(|value| !value.is_null());
        match (left, right) {
            (None, None) => Ordering::Equal,
            (None, Some(_)) => Ordering::Greater,
            (Some(_), None) => Ordering::Less,
            (Some(left), Some(right)) => value_cmp(left, right),
        }
    });
}

fn value_cmp(a: &Value, b: &Value) -> Ordering {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x
            .as_f64()
            .partial_cmp(&y.as_f64())
            .unwrap_or(Ordering::Equal),
        (Value::String(x), Value::String(y)) => x.cmp(y),
        (Value::Bool(x), Value::Bool(y)) => x.cmp(y),
        (Value::Array(x), Value::Array(y)) => x
            .len()
            .cmp(&y.len())
            .then_with(|| canonical(a).cmp(&canonical(b))),
        _ => type_rank(a)
            .cmp(&type_rank(b))
            .then_with(|| canonical(a).cmp(&canonical(b))),
    }
}

fn type_rank(value: &Value) -> u8 {
    match value {
        Value::Null => 0,
        Value::Bool(_) => 1,
        Value::Number(_) => 2,
        Value::String(_) => 3,
        Value::Array(_) => 4,
        Value::Object(_) => 5,
    }
}

fn canonical(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_default()
}

pub fn project(record: &Value, fields: &[String]) -> Value {
    let mut output = Map::new();
    for field in fields {
        if field == "_id" {
            continue;
        }
        if let Some(value) = get_path(record, field) {
            set_path(&mut output, field, value.clone());
        }
    }
    Value::Object(output)
}

fn set_path(output: &mut Map<String, Value>, path: &str, value: Value) {
    match path.split_once('.') {
        None => {
            output.insert(path.to_string(), value);
        }
        Some((head, rest)) => {
            let entry = output
                .entry(head.to_string())
                .or_insert_with(|| Value::Object(Map::new()));
            if !entry.is_object() {
                *entry = Value::Object(Map::new());
            }
            if let Some(map) = entry.as_object_mut() {
                set_path(map, rest, value);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn record() -> Value {
        json!({
            "name": "Ana",
            "age": 30,
            "active": true,
            "tags": ["dev", "vip"],
            "address": {"city": "SP", "zip": "01000-000"},
            "note": null
        })
    }

    #[test]
    fn equality_and_comparison() {
        let record = record();
        assert!(matches(&json!({"name": "Ana"}), &record).unwrap());
        assert!(!matches(&json!({"name": "Bia"}), &record).unwrap());
        assert!(matches(&json!({"age": {"$gt": 18}}), &record).unwrap());
        assert!(!matches(&json!({"age": {"$gt": 30}}), &record).unwrap());
        assert!(matches(&json!({"age": {"$gte": 30}}), &record).unwrap());
        assert!(matches(&json!({"age": {"$lt": 40, "$gte": 30}}), &record).unwrap());
        assert!(matches(&json!({"age": {"$ne": 31}}), &record).unwrap());
    }

    #[test]
    fn membership_and_string_ops() {
        let record = record();
        assert!(matches(&json!({"age": {"$in": [30, 40]}}), &record).unwrap());
        assert!(matches(&json!({"age": {"$nin": [1, 2]}}), &record).unwrap());
        assert!(matches(&json!({"name": {"$contains": "na"}}), &record).unwrap());
        assert!(matches(&json!({"tags": {"$contains": "vip"}}), &record).unwrap());
        assert!(!matches(&json!({"tags": {"$contains": "adm"}}), &record).unwrap());
        assert!(matches(&json!({"name": {"$starts_with": "A"}}), &record).unwrap());
        assert!(matches(&json!({"name": {"$ends_with": "na"}}), &record).unwrap());
        assert!(matches(&json!({"name": {"$regex": "^A.a$"}}), &record).unwrap());
        assert!(!matches(&json!({"name": {"$regex": "^B"}}), &record).unwrap());
    }

    #[test]
    fn exists_and_null() {
        let record = record();
        assert!(matches(&json!({"note": {"$exists": false}}), &record).unwrap());
        assert!(matches(&json!({"missing": {"$exists": false}}), &record).unwrap());
        assert!(matches(&json!({"name": {"$exists": true}}), &record).unwrap());
        assert!(!matches(&json!({"note": {"$exists": true}}), &record).unwrap());
    }

    #[test]
    fn logical_operators_and_paths() {
        let record = record();
        assert!(
            matches(
                &json!({"$or": [{"name": "Bia"}, {"age": {"$gte": 18}}]}),
                &record
            )
            .unwrap()
        );
        assert!(
            matches(
                &json!({"$and": [{"active": true}, {"$not": {"name": "Bia"}}]}),
                &record
            )
            .unwrap()
        );
        assert!(matches(&json!({"address.city": "SP"}), &record).unwrap());
        assert!(!matches(&json!({"address.city": "RJ"}), &record).unwrap());
        assert!(matches(&json!({"tags.1": "vip"}), &record).unwrap());
    }

    #[test]
    fn invalid_filters_error() {
        let record = record();
        assert!(matches(&json!({"age": {"$foo": 1}}), &record).is_err());
        assert!(matches(&json!({"$or": "not-array"}), &record).is_err());
        assert!(matches(&json!({"age": {"$in": "not-array"}}), &record).is_err());
        assert!(matches(&json!("not-object"), &record).is_err());
    }

    #[test]
    fn sort_asc_desc_and_nulls_last() {
        let mut entries = vec![
            ("a".to_string(), json!({"age": 30})),
            ("b".to_string(), json!({"age": 20})),
            ("c".to_string(), json!({})),
            ("d".to_string(), json!({"age": 25})),
        ];
        let asc = vec![SortSpec {
            field: "age".to_string(),
            dir: SortDir::Asc,
        }];
        sort_entries(&mut entries, &asc, |entry| &entry.1);
        assert_eq!(
            entries
                .iter()
                .map(|entry| entry.0.as_str())
                .collect::<Vec<_>>(),
            vec!["b", "d", "a", "c"]
        );
        let desc = vec![SortSpec {
            field: "age".to_string(),
            dir: SortDir::Desc,
        }];
        sort_entries(&mut entries, &desc, |entry| &entry.1);
        assert_eq!(
            entries
                .iter()
                .map(|entry| entry.0.as_str())
                .collect::<Vec<_>>(),
            vec!["a", "d", "b", "c"]
        );
    }

    #[test]
    fn sort_deserializes_strings_and_objects() {
        let request: QueryRequest =
            serde_json::from_value(json!({"sort": ["name", "-age"]})).unwrap();
        assert_eq!(request.sort[0].field, "name");
        assert_eq!(request.sort[0].dir, SortDir::Asc);
        assert_eq!(request.sort[1].field, "age");
        assert_eq!(request.sort[1].dir, SortDir::Desc);
        let request: QueryRequest =
            serde_json::from_value(json!({"sort": [{"field": "name", "direction": "desc"}]}))
                .unwrap();
        assert_eq!(request.sort[0].dir, SortDir::Desc);
    }

    #[test]
    fn sort_breaks_ties_by_id() {
        let mut entries = vec![
            json!({"_id": "b", "age": 30}),
            json!({"_id": "a", "age": 30}),
            json!({"_id": "c", "age": 30}),
        ];
        let sort = vec![SortSpec {
            field: "age".to_string(),
            dir: SortDir::Asc,
        }];
        sort_entries(&mut entries, &sort, |entry| entry);
        let ids: Vec<&str> = entries
            .iter()
            .map(|entry| entry["_id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, vec!["a", "b", "c"]);
    }

    #[test]
    fn default_sort_is_id_ascending() {
        let sort = effective_sort(&[]);
        assert_eq!(sort.len(), 1);
        assert_eq!(sort[0].field, ID_SORT_FIELD);
        assert_eq!(sort[0].dir, SortDir::Asc);
        let explicit = vec![SortSpec {
            field: "name".to_string(),
            dir: SortDir::Desc,
        }];
        assert_eq!(effective_sort(&explicit).len(), 1);
    }

    #[test]
    fn cursor_roundtrip_and_validation() {
        let sort = vec![
            SortSpec {
                field: "age".to_string(),
                dir: SortDir::Asc,
            },
            SortSpec {
                field: "name".to_string(),
                dir: SortDir::Desc,
            },
        ];
        let key = vec![json!(30), json!("Ana"), json!("id-1")];
        let cursor = encode_cursor(&key).unwrap();
        assert!(
            !cursor.contains('='),
            "cursor must be url-safe without padding"
        );
        let decoded = decode_cursor(&cursor, &sort).unwrap();
        assert_eq!(decoded.k, key);

        assert!(decode_cursor("not-base64!", &sort).is_err());
        assert!(
            decode_cursor(&cursor, &sort[..1]).is_err(),
            "arity mismatch"
        );
        assert!(decode_cursor(&encode_cursor(&key[..2]).unwrap(), &sort).is_err());
    }

    #[test]
    fn key_after_walks_sort_then_id() {
        let asc = vec![SortSpec {
            field: "age".to_string(),
            dir: SortDir::Asc,
        }];
        assert!(key_after(
            &[json!(31), json!("a")],
            &[json!(30), json!("z")],
            &asc
        ));
        assert!(!key_after(
            &[json!(30), json!("a")],
            &[json!(30), json!("z")],
            &asc
        ));
        // same age: the _id tiebreaker still advances
        assert!(key_after(
            &[json!(30), json!("z")],
            &[json!(30), json!("a")],
            &asc
        ));

        let desc = vec![SortSpec {
            field: "age".to_string(),
            dir: SortDir::Desc,
        }];
        assert!(key_after(
            &[json!(29), json!("a")],
            &[json!(30), json!("z")],
            &desc
        ));
    }

    #[test]
    fn key_after_handles_nulls() {
        let asc = vec![SortSpec {
            field: "age".to_string(),
            dir: SortDir::Asc,
        }];
        // nulls sort last, so a null key is after any present value
        assert!(key_after(
            &[json!(null), json!("a")],
            &[json!(1), json!("z")],
            &asc
        ));
        assert!(!key_after(
            &[json!(1), json!("a")],
            &[json!(null), json!("z")],
            &asc
        ));
    }

    #[test]
    fn projection_nested_and_id() {
        let record = record();
        let projected = project(
            &record,
            &[
                "name".to_string(),
                "address.city".to_string(),
                "_id".to_string(),
            ],
        );
        assert_eq!(projected, json!({"name": "Ana", "address": {"city": "SP"}}));
    }
}
