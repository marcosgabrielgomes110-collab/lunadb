use regex::Regex;
use serde::{Deserialize, Deserializer};
use serde_json::{Map, Value};
use std::cmp::Ordering;

use crate::error::AppError;

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

fn filter_error(message: impl Into<String>) -> AppError {
    AppError::BadRequest(format!("filter invalido: {}", message.into()))
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
        return Err(filter_error("deve ser um objeto JSON"));
    };
    for (key, condition) in object {
        let matched = match key.as_str() {
            "$and" => {
                let list = condition
                    .as_array()
                    .ok_or_else(|| filter_error("$and espera um array"))?;
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
                    .ok_or_else(|| filter_error("$or espera um array"))?;
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
                return Err(filter_error(format!("operador desconhecido '{other}'")));
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
                        "campo '{path}': nao misture operadores e valores"
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

fn apply_op(path: &str, value: Option<&Value>, op: &str, operand: &Value) -> Result<bool, AppError> {
    match op {
        "$eq" => Ok(value == Some(operand)),
        "$ne" => Ok(value != Some(operand)),
        "$exists" => {
            let expected = operand.as_bool().ok_or_else(|| {
                filter_error(format!("campo '{path}': $exists espera boolean"))
            })?;
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
                .ok_or_else(|| filter_error(format!("campo '{path}': $in espera array")))?;
            Ok(list.iter().any(|item| value == Some(item)))
        }
        "$nin" => {
            let list = operand
                .as_array()
                .ok_or_else(|| filter_error(format!("campo '{path}': $nin espera array")))?;
            Ok(!list.iter().any(|item| value == Some(item)))
        }
        "$contains" => match value {
            Some(Value::String(text)) => {
                let needle = operand.as_str().ok_or_else(|| {
                    filter_error(format!("campo '{path}': $contains espera string"))
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
            let needle = operand.as_str().ok_or_else(|| {
                filter_error(format!("campo '{path}': {op} espera string"))
            })?;
            Ok(match op {
                "$starts_with" => text.starts_with(needle),
                _ => text.ends_with(needle),
            })
        }
        "$regex" => {
            let pattern = operand
                .as_str()
                .ok_or_else(|| filter_error(format!("campo '{path}': $regex espera string")))?;
            let regex = Regex::new(pattern).map_err(|err| {
                filter_error(format!("campo '{path}': $regex invalido: {err}"))
            })?;
            let Some(Value::String(text)) = value else {
                return Ok(false);
            };
            Ok(regex.is_match(text))
        }
        other => Err(filter_error(format!("operador desconhecido '{other}'"))),
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
        Ordering::Equal
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
        (Value::Array(x), Value::Array(y)) => {
            x.len().cmp(&y.len()).then_with(|| {
                canonical(a).cmp(&canonical(b))
            })
        }
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
            "nome": "Ana",
            "idade": 30,
            "ativo": true,
            "tags": ["dev", "vip"],
            "endereco": {"cidade": "SP", "cep": "01000-000"},
            "nota": null
        })
    }

    #[test]
    fn equality_and_comparison() {
        let record = record();
        assert!(matches(&json!({"nome": "Ana"}), &record).unwrap());
        assert!(!matches(&json!({"nome": "Bia"}), &record).unwrap());
        assert!(matches(&json!({"idade": {"$gt": 18}}), &record).unwrap());
        assert!(!matches(&json!({"idade": {"$gt": 30}}), &record).unwrap());
        assert!(matches(&json!({"idade": {"$gte": 30}}), &record).unwrap());
        assert!(matches(&json!({"idade": {"$lt": 40, "$gte": 30}}), &record).unwrap());
        assert!(matches(&json!({"idade": {"$ne": 31}}), &record).unwrap());
    }

    #[test]
    fn membership_and_string_ops() {
        let record = record();
        assert!(matches(&json!({"idade": {"$in": [30, 40]}}), &record).unwrap());
        assert!(matches(&json!({"idade": {"$nin": [1, 2]}}), &record).unwrap());
        assert!(matches(&json!({"nome": {"$contains": "na"}}), &record).unwrap());
        assert!(matches(&json!({"tags": {"$contains": "vip"}}), &record).unwrap());
        assert!(!matches(&json!({"tags": {"$contains": "adm"}}), &record).unwrap());
        assert!(matches(&json!({"nome": {"$starts_with": "A"}}), &record).unwrap());
        assert!(matches(&json!({"nome": {"$ends_with": "na"}}), &record).unwrap());
        assert!(matches(&json!({"nome": {"$regex": "^A.a$"}}), &record).unwrap());
        assert!(!matches(&json!({"nome": {"$regex": "^B"}}), &record).unwrap());
    }

    #[test]
    fn exists_and_null() {
        let record = record();
        assert!(matches(&json!({"nota": {"$exists": false}}), &record).unwrap());
        assert!(matches(&json!({"inexistente": {"$exists": false}}), &record).unwrap());
        assert!(matches(&json!({"nome": {"$exists": true}}), &record).unwrap());
        assert!(!matches(&json!({"nota": {"$exists": true}}), &record).unwrap());
    }

    #[test]
    fn logical_operators_and_paths() {
        let record = record();
        assert!(
            matches(
                &json!({"$or": [{"nome": "Bia"}, {"idade": {"$gte": 18}}]}),
                &record
            )
            .unwrap()
        );
        assert!(
            matches(
                &json!({"$and": [{"ativo": true}, {"$not": {"nome": "Bia"}}]}),
                &record
            )
            .unwrap()
        );
        assert!(matches(&json!({"endereco.cidade": "SP"}), &record).unwrap());
        assert!(!matches(&json!({"endereco.cidade": "RJ"}), &record).unwrap());
        assert!(matches(&json!({"tags.1": "vip"}), &record).unwrap());
    }

    #[test]
    fn invalid_filters_error() {
        let record = record();
        assert!(matches(&json!({"idade": {"$foo": 1}}), &record).is_err());
        assert!(matches(&json!({"$or": "nao-array"}), &record).is_err());
        assert!(matches(&json!({"idade": {"$in": "nao-array"}}), &record).is_err());
        assert!(matches(&json!("nao-objeto"), &record).is_err());
    }

    #[test]
    fn sort_asc_desc_and_nulls_last() {
        let mut entries = vec![
            ("a".to_string(), json!({"idade": 30})),
            ("b".to_string(), json!({"idade": 20})),
            ("c".to_string(), json!({})),
            ("d".to_string(), json!({"idade": 25})),
        ];
        let asc = vec![SortSpec {
            field: "idade".to_string(),
            dir: SortDir::Asc,
        }];
        sort_entries(&mut entries, &asc, |entry| &entry.1);
        assert_eq!(
            entries.iter().map(|entry| entry.0.as_str()).collect::<Vec<_>>(),
            vec!["b", "d", "a", "c"]
        );
        let desc = vec![SortSpec {
            field: "idade".to_string(),
            dir: SortDir::Desc,
        }];
        sort_entries(&mut entries, &desc, |entry| &entry.1);
        assert_eq!(
            entries.iter().map(|entry| entry.0.as_str()).collect::<Vec<_>>(),
            vec!["a", "d", "b", "c"]
        );
    }

    #[test]
    fn sort_deserializes_strings_and_objects() {
        let request: QueryRequest =
            serde_json::from_value(json!({"sort": ["nome", "-idade"]})).unwrap();
        assert_eq!(request.sort[0].field, "nome");
        assert_eq!(request.sort[0].dir, SortDir::Asc);
        assert_eq!(request.sort[1].field, "idade");
        assert_eq!(request.sort[1].dir, SortDir::Desc);
        let request: QueryRequest = serde_json::from_value(
            json!({"sort": [{"field": "nome", "direction": "desc"}]}),
        )
        .unwrap();
        assert_eq!(request.sort[0].dir, SortDir::Desc);
    }

    #[test]
    fn projection_nested_and_id() {
        let record = record();
        let projected = project(&record, &["nome".to_string(), "endereco.cidade".to_string(), "_id".to_string()]);
        assert_eq!(projected, json!({"nome": "Ana", "endereco": {"cidade": "SP"}}));
    }
}
