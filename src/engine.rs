use chrono::Utc;
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use uuid::Uuid;

use crate::crypto::{self, Keyring};
use crate::error::{AppError, FieldError, RecordError};
use crate::models::*;
use crate::query::{self, QueryRequest};
use crate::schema::{self, Mode, TableSchema, UniqueIndex};
use crate::state::AppState;
use crate::storage;

macro_rules! ops {
    ($op:expr; $($key:ident = $value:expr),* $(,)?) => {
        tracing::info!(target: "lunadb::ops", op = %$op, $($key = %$value),*);
    };
}

/// Cap on records returned per page (listing and query), avoiding
/// unbounded responses that would load the whole table into memory.
pub const MAX_PAGE_SIZE: usize = 1000;

#[derive(Debug, Clone, Deserialize, Default)]
pub struct ListParams {
    pub limit: Option<usize>,
    pub offset: Option<usize>,
    /// Opaque keyset cursor returned as `next_cursor` by a previous page.
    pub cursor: Option<String>,
}

pub struct ListOutcome {
    pub total: usize,
    pub records: Vec<Value>,
    pub next_cursor: Option<String>,
}

pub fn validate_name(name: &str) -> Result<(), AppError> {
    let valid = !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if valid {
        Ok(())
    } else {
        Err(AppError::BadRequest(format!("invalid name: '{name}'")))
    }
}

pub fn validate_grant(grant: &Grant) -> Result<(), AppError> {
    if grant.db == "*" {
        Ok(())
    } else {
        validate_name(&grant.db)
    }
}

pub async fn roles(state: &AppState) -> Result<RolesFile, AppError> {
    Ok(storage::read_json(&state.paths.roles_file()).await?)
}

async fn dbs(state: &AppState) -> Result<DbsFile, AppError> {
    Ok(storage::read_json(&state.paths.dbs_registry()).await?)
}

/// Reads the database registry under the metadata read lock, so the snapshot
/// never races with a create/delete of a database.
async fn dbs_consistent(state: &AppState) -> Result<DbsFile, AppError> {
    let _guard = state.lock_meta_read().await;
    dbs(state).await
}

fn granted_level(grants: &[Grant], db: &str) -> Option<Level> {
    grants
        .iter()
        .filter(|grant| grant.db == db || grant.db == "*")
        .map(|grant| grant.level)
        .max()
}

fn effective_level_with(roles: &RolesFile, user: &User, db: &str) -> Option<Level> {
    if user.role == ROLE_ADMIN {
        return Some(Level::Admin);
    }
    let role_level = roles
        .roles
        .iter()
        .find(|role| role.name == user.role)
        .and_then(|role| granted_level(&role.permissions, db));
    let user_level = granted_level(&user.grants, db);
    match (role_level, user_level) {
        (Some(a), Some(b)) => Some(a.max(b)),
        (Some(a), None) => Some(a),
        (None, b) => b,
    }
}

pub async fn effective_level(
    state: &AppState,
    user: &User,
    db: &str,
) -> Result<Option<Level>, AppError> {
    if user.role == ROLE_ADMIN {
        return Ok(Some(Level::Admin));
    }
    let roles = roles(state).await?;
    Ok(effective_level_with(&roles, user, db))
}

pub async fn require_level(
    state: &AppState,
    user: &User,
    db: &str,
    min: Level,
) -> Result<(), AppError> {
    match effective_level(state, user, db).await? {
        Some(level) if level >= min => Ok(()),
        Some(_) => Err(AppError::Forbidden(format!(
            "permission '{}' required on db '{db}'",
            min.as_str()
        ))),
        None => Err(AppError::Forbidden(format!("no access to db '{db}'"))),
    }
}

async fn ensure_db_exists(state: &AppState, db: &str) -> Result<(), AppError> {
    let dbs = dbs(state).await?;
    if dbs.dbs.iter().any(|meta| meta.name == db) {
        Ok(())
    } else {
        Err(AppError::NotFound(format!("db '{db}'")))
    }
}

pub async fn list_dbs(state: &AppState, user: &User) -> Result<Vec<DbMeta>, AppError> {
    let dbs = dbs_consistent(state).await?;
    let metas = if user.role == ROLE_ADMIN {
        dbs.dbs
    } else {
        let roles = roles(state).await?;
        dbs.dbs
            .into_iter()
            .filter(|meta| effective_level_with(&roles, user, &meta.name).is_some())
            .collect()
    };
    ops!("list_dbs"; user = user.username, count = metas.len());
    Ok(metas)
}

pub async fn get_db(state: &AppState, user: &User, name: &str) -> Result<DbMeta, AppError> {
    validate_name(name)?;
    let dbs = dbs_consistent(state).await?;
    let meta = dbs
        .dbs
        .into_iter()
        .find(|meta| meta.name == name)
        .ok_or_else(|| AppError::NotFound(format!("db '{name}'")))?;
    require_level(state, user, name, Level::Read).await?;
    ops!("get_db"; user = user.username, db = name);
    Ok(meta)
}

pub async fn create_db(state: &AppState, user: &User, name: &str) -> Result<DbMeta, AppError> {
    if user.role != ROLE_ADMIN {
        return Err(AppError::Forbidden(
            "only administrators can create dbs".to_string(),
        ));
    }
    validate_name(name)?;
    let _guard = state.lock_meta_write().await;
    let mut dbs = dbs(state).await?;
    if dbs.dbs.iter().any(|meta| meta.name == name) {
        return Err(AppError::Conflict(format!("db '{name}' already exists")));
    }
    let meta = DbMeta {
        name: name.to_string(),
        owner_id: user.id.clone(),
        created_at: Utc::now(),
    };
    tokio::fs::create_dir_all(state.paths.db_dir(name)).await?;
    dbs.dbs.push(meta.clone());
    storage::write_json(&state.paths.dbs_registry(), &dbs).await?;
    ops!("create_db"; user = user.username, db = name);
    Ok(meta)
}

pub async fn delete_db(state: &AppState, user: &User, name: &str) -> Result<(), AppError> {
    validate_name(name)?;
    require_level(state, user, name, Level::Admin).await?;
    // The database lock excludes in-flight table operations; the metadata lock
    // serializes the registry update against create_db. Always in this order.
    let _db_guard = state.lock_db_write(name).await;
    let _meta_guard = state.lock_meta_write().await;
    let mut dbs = dbs(state).await?;
    let index = dbs
        .dbs
        .iter()
        .position(|meta| meta.name == name)
        .ok_or_else(|| AppError::NotFound(format!("db '{name}'")))?;
    let dir = state.paths.db_dir(name);
    if dir.exists() {
        tokio::fs::remove_dir_all(&dir).await?;
    }
    dbs.dbs.remove(index);
    storage::write_json(&state.paths.dbs_registry(), &dbs).await?;
    ops!("delete_db"; user = user.username, db = name);
    Ok(())
}

pub async fn list_tables(state: &AppState, user: &User, db: &str) -> Result<Vec<String>, AppError> {
    validate_name(db)?;
    ensure_db_exists(state, db).await?;
    require_level(state, user, db, Level::Read).await?;
    let mut names = Vec::new();
    let mut entries = tokio::fs::read_dir(state.paths.db_dir(db)).await?;
    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        let extension = path.extension().and_then(|ext| ext.to_str());
        if matches!(extension, Some("json") | Some("msgpack"))
            && let Some(stem) = path.file_stem().and_then(|stem| stem.to_str())
        {
            names.push(stem.to_string());
        }
    }
    names.sort();
    names.dedup();
    ops!("list_tables"; user = user.username, db = db, count = names.len());
    Ok(names)
}

pub async fn create_table(
    state: &AppState,
    user: &User,
    db: &str,
    table: &str,
    schema: Option<TableSchema>,
) -> Result<String, AppError> {
    validate_name(db)?;
    validate_name(table)?;
    ensure_db_exists(state, db).await?;
    require_level(state, user, db, Level::Write).await?;
    if let Some(schema) = &schema {
        schema::compile(schema)?;
    }
    let _guard = state.lock_table_write(db, table).await;
    if state.paths.table_file_existing(db, table).is_some() {
        return Err(AppError::Conflict(format!(
            "table '{table}' already exists in db '{db}'"
        )));
    }
    let has_schema = schema.is_some();
    let file = TableFile {
        name: table.to_string(),
        created_at: Some(Utc::now()),
        schema,
        records: Default::default(),
    };
    write_table(state, db, table, &file).await?;
    ops!("create_table"; user = user.username, db = db, table = table, schema = has_schema);
    Ok(table.to_string())
}

pub async fn delete_table(
    state: &AppState,
    user: &User,
    db: &str,
    table: &str,
) -> Result<(), AppError> {
    validate_name(db)?;
    validate_name(table)?;
    ensure_db_exists(state, db).await?;
    require_level(state, user, db, Level::Write).await?;
    let _guard = state.lock_table_write(db, table).await;
    let path = state
        .paths
        .table_file_existing(db, table)
        .ok_or_else(|| AppError::NotFound(format!("table '{db}.{table}'")))?;
    match tokio::fs::remove_file(&path).await {
        Ok(()) => {
            ops!("delete_table"; user = user.username, db = db, table = table);
            Ok(())
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            Err(AppError::NotFound(format!("table '{db}.{table}'")))
        }
        Err(err) => Err(err.into()),
    }
}

/// Reads a table without taking any lock. Callers hold either the table read
/// lock or the table write lock.
async fn read_table_unlocked(
    state: &AppState,
    db: &str,
    table: &str,
) -> Result<TableFile, AppError> {
    let path = state
        .paths
        .table_file_existing(db, table)
        .ok_or_else(|| AppError::NotFound(format!("table '{db}.{table}'")))?;
    Ok(storage::read_value(&path).await?)
}

/// Reads a table under its read lock, so the snapshot never mixes with a
/// concurrent write to the same file.
async fn read_table(state: &AppState, db: &str, table: &str) -> Result<TableFile, AppError> {
    let _guard = state.lock_table_read(db, table).await;
    read_table_unlocked(state, db, table).await
}

async fn write_table(
    state: &AppState,
    db: &str,
    table: &str,
    file: &TableFile,
) -> Result<(), AppError> {
    let configured = state.paths.table_file(db, table);
    storage::write_value(&configured, file).await?;
    for path in state.paths.table_paths(db, table) {
        if path != configured && path.exists() {
            let _ = tokio::fs::remove_file(&path).await;
        }
    }
    Ok(())
}

fn with_id(id: &str, value: &Value) -> Value {
    let mut value = value.clone();
    if let Some(map) = value.as_object_mut() {
        map.insert("_id".to_string(), json!(id));
    }
    value
}

fn strip_id(value: &mut Value) {
    if let Some(map) = value.as_object_mut() {
        map.remove("_id");
    }
}

/// Merge patch (RFC 7386): objects are merged recursively; `null` removes the
/// key; any other value replaces the field.
fn merge_patch(target: &mut Value, patch: &Value) {
    match patch {
        Value::Object(patch_map) => {
            if !target.is_object() {
                *target = Value::Object(serde_json::Map::new());
            }
            let Some(target_map) = target.as_object_mut() else {
                return;
            };
            for (key, value) in patch_map {
                if value.is_null() {
                    target_map.remove(key);
                } else {
                    merge_patch(target_map.entry(key.clone()).or_insert(Value::Null), value);
                }
            }
        }
        other => *target = other.clone(),
    }
}

fn decode_record(
    keyring: &Keyring,
    db: &str,
    schema: Option<&TableSchema>,
    raw: &Value,
) -> Result<Value, AppError> {
    let Some(schema) = schema else {
        return Ok(raw.clone());
    };
    let mut value = raw.clone();
    if let Some(object) = value.as_object_mut() {
        for (name, field) in &schema.fields {
            if !field.encrypted {
                continue;
            }
            let Some(field_value) = object.get_mut(name) else {
                continue;
            };
            let Some(token) = field_value
                .as_str()
                .filter(|text| crypto::is_encrypted_str(text))
            else {
                continue;
            };
            let plaintext = keyring.decrypt_value(db, token).map_err(|err| {
                AppError::Crypto(format!("failed to decrypt field '{name}': {err}"))
            })?;
            *field_value = plaintext;
        }
    }
    Ok(value)
}

fn encode_record(
    keyring: &Keyring,
    db: &str,
    schema: Option<&TableSchema>,
    plaintext: &Value,
) -> Result<Value, AppError> {
    let Some(schema) = schema else {
        return Ok(plaintext.clone());
    };
    let mut value = plaintext.clone();
    if let Some(object) = value.as_object_mut() {
        for (name, field) in &schema.fields {
            if !field.encrypted {
                continue;
            }
            let Some(field_value) = object.get_mut(name) else {
                continue;
            };
            if field_value.is_null() || crypto::is_encrypted_value(field_value) {
                continue;
            }
            *field_value = keyring.encrypt_value(db, field_value).map_err(|err| {
                AppError::Crypto(format!("failed to encrypt field '{name}': {err}"))
            })?;
        }
    }
    Ok(value)
}

fn validation_error(errors: Vec<FieldError>) -> AppError {
    if errors.iter().any(|error| error.unique) {
        AppError::UniqueViolation(errors)
    } else {
        AppError::Validation(errors)
    }
}

fn build_unique_index(
    schema: &TableSchema,
    records: &BTreeMap<String, Value>,
    exclude: Option<&str>,
) -> UniqueIndex {
    let mut index = UniqueIndex::new();
    for (id, raw) in records {
        if exclude == Some(id.as_str()) {
            continue;
        }
        schema::index_unique_fields(&mut index, schema, raw);
    }
    index
}

pub async fn list_records(
    state: &AppState,
    user: &User,
    db: &str,
    table: &str,
    params: ListParams,
) -> Result<ListOutcome, AppError> {
    validate_name(db)?;
    validate_name(table)?;
    require_level(state, user, db, Level::Read).await?;
    let file = read_table(state, db, table).await?;
    let total = file.records.len();
    let limit = params.limit.unwrap_or(MAX_PAGE_SIZE).min(MAX_PAGE_SIZE);
    // Without an explicit sort, records follow `_id` ascending, which keeps the
    // cursor walk stable because `_id` is unique.
    let sort = query::effective_sort(&[]);
    let cursor = match params.cursor.as_deref() {
        Some(cursor) => Some(query::decode_cursor(cursor, &sort)?),
        None => None,
    };
    let offset = params.offset.unwrap_or(0);
    let mut records = Vec::new();
    let mut last_key: Option<Vec<Value>> = None;
    for (id, raw) in file.records.iter() {
        let decoded = decode_record(&state.keyring, db, file.schema.as_ref(), raw)?;
        let value = with_id(id, &decoded);
        if let Some(cursor) = &cursor {
            let key = query::cursor_key(&value, &sort)?;
            if !query::key_after(&key, &cursor.k, &sort) {
                continue;
            }
        }
        if records.len() >= limit {
            break;
        }
        last_key = Some(query::cursor_key(&value, &sort)?);
        records.push(value);
    }
    let page_full = records.len() == limit;
    if cursor.is_none() && offset > 0 {
        records.drain(..offset.min(records.len()));
    }
    let next_cursor = if page_full {
        last_key.map(|key| query::encode_cursor(&key)).transpose()?
    } else {
        None
    };
    ops!("list_records"; user = user.username, db = db, table = table, count = records.len());
    Ok(ListOutcome {
        total,
        records,
        next_cursor,
    })
}

pub async fn create_record(
    state: &AppState,
    user: &User,
    db: &str,
    table: &str,
    mut body: Value,
) -> Result<Value, AppError> {
    validate_name(db)?;
    validate_name(table)?;
    require_level(state, user, db, Level::Write).await?;
    if !body.is_object() {
        return Err(AppError::BadRequest(
            "record must be a JSON object".to_string(),
        ));
    }
    strip_id(&mut body);
    let _guard = state.lock_table_write(db, table).await;
    let mut file = read_table_unlocked(state, db, table).await?;
    let id = Uuid::new_v4().to_string();
    let plaintext = if let Some(schema) = file.schema.clone() {
        let unique = build_unique_index(&schema, &file.records, None);
        schema::prepare_record(&schema, &mut body, Mode::Create, None, &unique)
            .map_err(validation_error)?;
        body
    } else {
        body
    };
    let stored = encode_record(&state.keyring, db, file.schema.as_ref(), &plaintext)?;
    file.records.insert(id.clone(), stored);
    write_table(state, db, table, &file).await?;
    ops!("create_record"; user = user.username, db = db, table = table, id = id);
    Ok(with_id(&id, &plaintext))
}

pub async fn get_record(
    state: &AppState,
    user: &User,
    db: &str,
    table: &str,
    id: &str,
) -> Result<Value, AppError> {
    validate_name(db)?;
    validate_name(table)?;
    require_level(state, user, db, Level::Read).await?;
    let file = read_table(state, db, table).await?;
    let raw = file
        .records
        .get(id)
        .ok_or_else(|| AppError::NotFound(format!("record '{id}'")))?;
    let value = decode_record(&state.keyring, db, file.schema.as_ref(), raw)?;
    ops!("get_record"; user = user.username, db = db, table = table, id = id);
    Ok(with_id(id, &value))
}

pub async fn replace_record(
    state: &AppState,
    user: &User,
    db: &str,
    table: &str,
    id: &str,
    mut body: Value,
) -> Result<Value, AppError> {
    validate_name(db)?;
    validate_name(table)?;
    require_level(state, user, db, Level::Write).await?;
    if !body.is_object() {
        return Err(AppError::BadRequest(
            "record must be a JSON object".to_string(),
        ));
    }
    strip_id(&mut body);
    let _guard = state.lock_table_write(db, table).await;
    let mut file = read_table_unlocked(state, db, table).await?;
    let previous = file
        .records
        .get(id)
        .cloned()
        .ok_or_else(|| AppError::NotFound(format!("record '{id}'")))?;
    let plaintext = if let Some(schema) = file.schema.clone() {
        let unique = build_unique_index(&schema, &file.records, Some(id));
        schema::prepare_record(&schema, &mut body, Mode::Replace, Some(&previous), &unique)
            .map_err(validation_error)?;
        body
    } else {
        body
    };
    let stored = encode_record(&state.keyring, db, file.schema.as_ref(), &plaintext)?;
    file.records.insert(id.to_string(), stored);
    write_table(state, db, table, &file).await?;
    ops!("replace_record"; user = user.username, db = db, table = table, id = id);
    Ok(with_id(id, &plaintext))
}

pub async fn patch_record(
    state: &AppState,
    user: &User,
    db: &str,
    table: &str,
    id: &str,
    mut body: Value,
) -> Result<Value, AppError> {
    validate_name(db)?;
    validate_name(table)?;
    require_level(state, user, db, Level::Write).await?;
    if !body.is_object() {
        return Err(AppError::BadRequest(
            "patch must be a JSON object".to_string(),
        ));
    }
    strip_id(&mut body);
    let _guard = state.lock_table_write(db, table).await;
    let mut file = read_table_unlocked(state, db, table).await?;
    let previous = file
        .records
        .get(id)
        .ok_or_else(|| AppError::NotFound(format!("record '{id}'")))?;
    let mut candidate = decode_record(&state.keyring, db, file.schema.as_ref(), previous)?;
    merge_patch(&mut candidate, &body);
    let snapshot = candidate.clone();
    let plaintext = if let Some(schema) = file.schema.clone() {
        let unique = build_unique_index(&schema, &file.records, Some(id));
        schema::prepare_record(
            &schema,
            &mut candidate,
            Mode::Replace,
            Some(&snapshot),
            &unique,
        )
        .map_err(validation_error)?;
        candidate
    } else {
        candidate
    };
    let stored = encode_record(&state.keyring, db, file.schema.as_ref(), &plaintext)?;
    file.records.insert(id.to_string(), stored);
    write_table(state, db, table, &file).await?;
    ops!("patch_record"; user = user.username, db = db, table = table, id = id);
    Ok(with_id(id, &plaintext))
}

pub async fn delete_record(
    state: &AppState,
    user: &User,
    db: &str,
    table: &str,
    id: &str,
) -> Result<(), AppError> {
    validate_name(db)?;
    validate_name(table)?;
    require_level(state, user, db, Level::Write).await?;
    let _guard = state.lock_table_write(db, table).await;
    let mut file = read_table_unlocked(state, db, table).await?;
    if file.records.remove(id).is_none() {
        return Err(AppError::NotFound(format!("record '{id}'")));
    }
    write_table(state, db, table, &file).await?;
    ops!("delete_record"; user = user.username, db = db, table = table, id = id);
    Ok(())
}

pub struct QueryOutcome {
    pub total: usize,
    pub records: Vec<Value>,
    pub next_cursor: Option<String>,
}

pub struct AggregateOutcome {
    pub total: usize,
    pub groups: Vec<Value>,
}

pub async fn query_records(
    state: &AppState,
    user: &User,
    db: &str,
    table: &str,
    request: QueryRequest,
) -> Result<QueryOutcome, AppError> {
    validate_name(db)?;
    validate_name(table)?;
    require_level(state, user, db, Level::Read).await?;
    let file = read_table(state, db, table).await?;
    let sort = query::effective_sort(&request.sort);
    let cursor = match request.cursor.as_deref() {
        Some(cursor) => Some(query::decode_cursor(cursor, &sort)?),
        None => None,
    };
    let mut entries: Vec<(String, Value, Vec<Value>)> = Vec::new();
    for (id, raw) in &file.records {
        let decoded = decode_record(&state.keyring, db, file.schema.as_ref(), raw)?;
        let value = with_id(id, &decoded);
        if let Some(filter) = &request.filter
            && !query::matches(filter, &value)?
        {
            continue;
        }
        let key = query::cursor_key(&value, &sort)?;
        entries.push((id.clone(), value, key));
    }
    let total = entries.len();
    let limit = request.limit.unwrap_or(MAX_PAGE_SIZE).min(MAX_PAGE_SIZE);
    query::sort_entries(&mut entries, &sort, |entry| &entry.1);
    // `total` above is the count of matching records, before the cursor/offset
    // window, so it stays stable across pages.
    if let Some(cursor) = &cursor {
        entries.retain(|(_, _, key)| query::key_after(key, &cursor.k, &sort));
    } else {
        let offset = request.offset.unwrap_or(0);
        if offset > 0 {
            entries.drain(..offset.min(entries.len()));
        }
    }
    entries.truncate(limit);
    let page_full = entries.len() == limit;
    let next_cursor = match (page_full, entries.last()) {
        (true, Some((_, _, key))) => Some(query::encode_cursor(key)?),
        _ => None,
    };
    let records = entries
        .into_iter()
        .map(|(id, value, _)| {
            let value = match &request.fields {
                Some(fields) => query::project(&value, fields),
                None => value,
            };
            with_id(&id, &value)
        })
        .collect::<Vec<_>>();
    ops!("query_records"; user = user.username, db = db, table = table, total = total, returned = records.len());
    Ok(QueryOutcome {
        total,
        records,
        next_cursor,
    })
}

pub async fn aggregate_records(
    state: &AppState,
    user: &User,
    db: &str,
    table: &str,
    request: query::AggregateRequest,
) -> Result<AggregateOutcome, AppError> {
    validate_name(db)?;
    validate_name(table)?;
    require_level(state, user, db, Level::Read).await?;
    let file = read_table(state, db, table).await?;
    let mut entries: Vec<Value> = Vec::new();
    for (id, raw) in &file.records {
        let decoded = decode_record(&state.keyring, db, file.schema.as_ref(), raw)?;
        let value = with_id(id, &decoded);
        if let Some(filter) = &request.filter
            && !query::matches(filter, &value)?
        {
            continue;
        }
        entries.push(value);
    }
    let total = entries.len();
    let groups = query::aggregate(&entries, &request)?;
    ops!("aggregate_records"; user = user.username, db = db, table = table, total = total, groups = groups.len());
    Ok(AggregateOutcome { total, groups })
}

pub async fn get_schema(
    state: &AppState,
    user: &User,
    db: &str,
    table: &str,
) -> Result<TableSchema, AppError> {
    validate_name(db)?;
    validate_name(table)?;
    require_level(state, user, db, Level::Read).await?;
    let file = read_table(state, db, table).await?;
    let schema = file
        .schema
        .ok_or_else(|| AppError::NotFound(format!("schema of table '{db}.{table}'")))?;
    ops!("get_schema"; user = user.username, db = db, table = table, fields = schema.fields.len());
    Ok(schema)
}

pub async fn set_schema(
    state: &AppState,
    user: &User,
    db: &str,
    table: &str,
    new_schema: TableSchema,
    force: bool,
) -> Result<TableSchema, AppError> {
    validate_name(db)?;
    validate_name(table)?;
    ensure_db_exists(state, db).await?;
    require_level(state, user, db, Level::Write).await?;
    schema::compile(&new_schema)?;
    let _guard = state.lock_table_write(db, table).await;
    let outcome = migrate_and_store(
        state,
        db,
        table,
        new_schema,
        force,
        "set_schema",
        &user.username,
    )
    .await?;
    ops!("set_schema"; user = user.username, db = db, table = table, force = force, records = outcome.1);
    Ok(outcome.0)
}

pub async fn patch_schema(
    state: &AppState,
    user: &User,
    db: &str,
    table: &str,
    patch: schema::SchemaPatch,
    force: bool,
) -> Result<TableSchema, AppError> {
    validate_name(db)?;
    validate_name(table)?;
    ensure_db_exists(state, db).await?;
    require_level(state, user, db, Level::Write).await?;
    let _guard = state.lock_table_write(db, table).await;
    let file = read_table_unlocked(state, db, table).await?;
    let mut current = file.schema.clone().unwrap_or_default();
    schema::apply_patch(&mut current, &patch)?;
    let outcome = migrate_and_store(
        state,
        db,
        table,
        current,
        force,
        "patch_schema",
        &user.username,
    )
    .await?;
    ops!("patch_schema"; user = user.username, db = db, table = table, force = force, records = outcome.1);
    Ok(outcome.0)
}

async fn migrate_and_store(
    state: &AppState,
    db: &str,
    table: &str,
    new_schema: TableSchema,
    force: bool,
    op: &str,
    username: &str,
) -> Result<(TableSchema, usize), AppError> {
    // Both callers hold the table write lock.
    let mut file = read_table_unlocked(state, db, table).await?;
    let old = file.schema.take();
    let mut migrated = BTreeMap::new();
    let mut unique = UniqueIndex::new();
    let mut rejected: Vec<RecordError> = Vec::new();
    for (id, raw) in &file.records {
        let decoded = decode_record(&state.keyring, db, old.as_ref(), raw)?;
        let mut candidate = decoded.clone();
        match schema::prepare_record(&new_schema, &mut candidate, Mode::Migrate, None, &unique) {
            Ok(()) => {
                let stored = encode_record(&state.keyring, db, Some(&new_schema), &candidate)?;
                schema::index_unique_fields(&mut unique, &new_schema, &candidate);
                migrated.insert(id.clone(), stored);
            }
            Err(_) if force => {
                let stored = encode_record(&state.keyring, db, Some(&new_schema), &decoded)?;
                schema::index_unique_fields(&mut unique, &new_schema, &decoded);
                migrated.insert(id.clone(), stored);
            }
            Err(fields) => rejected.push(RecordError {
                id: id.clone(),
                fields,
            }),
        }
    }
    if !rejected.is_empty() {
        tracing::warn!(
            target: "lunadb::ops",
            op = op,
            user = username,
            db = db,
            table = table,
            rejected = rejected.len(),
            "schema rejected by existing records"
        );
        return Err(AppError::SchemaRejected(rejected));
    }
    file.schema = Some(new_schema.clone());
    file.records = migrated;
    let records = file.records.len();
    write_table(state, db, table, &file).await?;
    Ok((new_schema, records))
}

pub async fn delete_schema(
    state: &AppState,
    user: &User,
    db: &str,
    table: &str,
) -> Result<(), AppError> {
    validate_name(db)?;
    validate_name(table)?;
    ensure_db_exists(state, db).await?;
    require_level(state, user, db, Level::Write).await?;
    let _guard = state.lock_table_write(db, table).await;
    let mut file = read_table_unlocked(state, db, table).await?;
    let old = file
        .schema
        .take()
        .ok_or_else(|| AppError::NotFound(format!("schema of table '{db}.{table}'")))?;
    let mut records = BTreeMap::new();
    for (id, raw) in &file.records {
        let decoded = decode_record(&state.keyring, db, Some(&old), raw)?;
        records.insert(id.clone(), decoded);
    }
    file.records = records;
    write_table(state, db, table, &file).await?;
    ops!("delete_schema"; user = user.username, db = db, table = table, records = file.records.len());
    Ok(())
}

pub async fn validate_record(
    state: &AppState,
    user: &User,
    db: &str,
    table: &str,
    mut body: Value,
) -> Result<Value, AppError> {
    validate_name(db)?;
    validate_name(table)?;
    require_level(state, user, db, Level::Write).await?;
    if !body.is_object() {
        return Err(AppError::BadRequest(
            "record must be a JSON object".to_string(),
        ));
    }
    strip_id(&mut body);
    let file = read_table(state, db, table).await?;
    let prepared = match file.schema.as_ref() {
        Some(schema) => {
            let unique = build_unique_index(schema, &file.records, None);
            schema::prepare_record(schema, &mut body, Mode::Create, None, &unique)
                .map_err(validation_error)?;
            body
        }
        None => body,
    };
    ops!("validate_record"; user = user.username, db = db, table = table);
    Ok(prepared)
}

#[derive(Debug, Default)]
pub struct MigrateReport {
    pub converted: Vec<String>,
    pub skipped: Vec<String>,
}

pub async fn migrate_tables(
    paths: &crate::config::Paths,
    target: crate::storage::Format,
) -> anyhow::Result<MigrateReport> {
    let dbs: DbsFile = storage::read_json(&paths.dbs_registry()).await?;
    let mut report = MigrateReport::default();
    for meta in &dbs.dbs {
        let dir = paths.db_dir(&meta.name);
        if !dir.exists() {
            continue;
        }
        let mut entries = tokio::fs::read_dir(&dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            let extension = path.extension().and_then(|ext| ext.to_str());
            if !matches!(extension, Some("json") | Some("msgpack")) {
                continue;
            }
            let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
                continue;
            };
            let label = format!("{}.{}", meta.name, stem);
            if storage::format_of(&path) == target {
                report.skipped.push(label);
                continue;
            }
            let value: Value = storage::read_value(&path).await?;
            let target_path = dir.join(format!("{stem}.{}", target.extension()));
            storage::write_value(&target_path, &value).await?;
            tokio::fs::remove_file(&path).await?;
            tracing::info!(
                target: "lunadb::ops",
                op = "migrate_table",
                table = %label,
                format = target.extension(),
            );
            report.converted.push(label);
        }
    }
    Ok(report)
}
