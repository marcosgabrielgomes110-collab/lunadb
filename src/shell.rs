use anyhow::{Context, anyhow};
use rustyline::DefaultEditor;
use rustyline::error::ReadlineError;
use serde_json::{Value, json};

use crate::client::Client;
use crate::config::Paths;

const HELP: &str = "\
comandos:
  login <user> [senha]            autentica (senha oculta se omitida)
  whoami | status | key rotate
  dbs                             lista databases
  db create <db> | db rm <db>
  tables <db>                     lista tabelas
  table create <db> <t> [schema-json] | table rm <db> <t>
  schema get <db> <t> | schema set <db> <t> <json> [--force] | schema rm <db> <t>
  schema patch <db> <t> <json> [--force]     add/remove/update/unique_add/unique_remove/strict
  schema add-field <db> <t> <nome> <json> [--force]
  schema rm-field <db> <t> <campo>... [--force]
  schema update-field <db> <t> <campo> <json> [--force]
  schema unique add|rm <db> <t> <campo|campo1+campo2> [--force]
  list <db> <t> [limit] [offset]  lista registros
  get <db> <t> <id>
  add <db> <t> '<json>'           json entre aspas simples
  put <db> <t> <id> '<json>'
  rm <db> <t> <id>
  query <db> <t> '<json>'         filtro/sort/fields/limit/offset
  users | user add <user> <senha> [role] [db:level ...] | user rm <id>
  grants <user-id> '<json>'
  apikey create <user> [--name <nome>] | apikey list [user] | apikey revoke <user> <id|prefixo>
  help | exit";

pub fn run(paths: &Paths, base_url: String, api_key: Option<String>) -> anyhow::Result<()> {
    let mut client = Client::new(base_url.clone(), api_key);
    println!("LunaDB shell ({base_url}) - 'help' para comandos, 'exit' para sair");
    let mut editor = DefaultEditor::new().context("inicializar editor")?;
    let history_path = paths.infra.join("shell_history");
    let _ = editor.load_history(&history_path);
    loop {
        match editor.readline("lunadb> ") {
            Ok(line) => {
                let line = line.trim().to_string();
                if line.is_empty() {
                    continue;
                }
                let _ = editor.add_history_entry(line.as_str());
                if matches!(line.as_str(), "exit" | "quit") {
                    break;
                }
                match execute(&mut client, &line) {
                    Ok(Some(output)) => println!("{output}"),
                    Ok(None) => {}
                    Err(err) => eprintln!("erro: {err}"),
                }
            }
            Err(ReadlineError::Interrupted) => continue,
            Err(ReadlineError::Eof) => break,
            Err(err) => return Err(err.into()),
        }
    }
    let _ = editor.save_history(&history_path);
    Ok(())
}

fn execute(client: &mut Client, line: &str) -> anyhow::Result<Option<String>> {
    let tokens = shlex::split(line).ok_or_else(|| anyhow!("aspas nao balanceadas"))?;
    let refs: Vec<&str> = tokens.iter().map(String::as_str).collect();
    let Some((command, args)) = refs.split_first() else {
        return Ok(None);
    };
    let output = match (*command, args) {
        ("help", _) => Some(HELP.to_string()),
        ("login", [username, rest @ ..]) => {
            let password = match rest {
                [password] => (*password).to_string(),
                [] => rpassword::prompt_password(format!("senha de {username}: "))?,
                _ => return Err(anyhow!("uso: login <usuario> [senha]")),
            };
            client.login(username, &password)?;
            Some(format!("logado como {username}"))
        }
        ("whoami", []) => Some(pretty(client.get("/me")?)),
        ("status", []) => Some(pretty(client.get("/status")?)),
        ("key", ["rotate"]) => {
            let value = client.post("/auth/rotate-key", json!({}))?;
            Some(format!(
                "nova api key (guarde agora): {}",
                value["api_key"].as_str().unwrap_or("")
            ))
        }
        ("dbs", []) => Some(pretty(client.get("/dbs")?)),
        ("db", ["create", name]) => {
            client.post("/dbs", json!({ "name": name }))?;
            Some(format!("db '{name}' criado"))
        }
        ("db", ["rm", name]) => {
            client.delete(&format!("/dbs/{name}"))?;
            Some(format!("db '{name}' removido"))
        }
        ("tables", [db]) => Some(pretty(client.get(&format!("/dbs/{db}/tables"))?)),
        ("table", ["create", db, table, rest @ ..]) => {
            let schema = match rest {
                [schema] => Some(json_arg(schema)?),
                [] => None,
                _ => return Err(anyhow!("uso: table create <db> <t> [schema-json]")),
            };
            let mut body = json!({ "name": table });
            if let Some(schema) = schema {
                body["schema"] = schema;
            }
            client.post(&format!("/dbs/{db}/tables"), body)?;
            Some(format!("tabela '{db}.{table}' criada"))
        }
        ("table", ["rm", db, table]) => {
            client.delete(&format!("/dbs/{db}/tables/{table}"))?;
            Some(format!("tabela '{db}.{table}' removida"))
        }
        ("schema", ["get", db, table]) => {
            Some(pretty(client.get(&format!("/dbs/{db}/tables/{table}/schema"))?))
        }
        ("schema", ["set", db, table, schema, rest @ ..]) => {
            let force = rest.first().is_some_and(|flag| *flag == "--force");
            client.put(
                &format!("/dbs/{db}/tables/{table}/schema?force={force}"),
                json_arg(schema)?,
            )?;
            Some(format!("schema de '{db}.{table}' atualizado"))
        }
        ("schema", ["rm", db, table]) => {
            client.delete(&format!("/dbs/{db}/tables/{table}/schema"))?;
            Some(format!("schema de '{db}.{table}' removido"))
        }
        ("schema", ["patch", db, table, body, rest @ ..]) => {
            let force = rest.first().is_some_and(|flag| *flag == "--force");
            Some(pretty(client.patch(
                &format!("/dbs/{db}/tables/{table}/schema?force={force}"),
                json_arg(body)?,
            )?))
        }
        ("schema", ["add-field", db, table, name, field, rest @ ..]) => {
            let force = rest.first().is_some_and(|flag| *flag == "--force");
            let body = json!({ "add": { *name: json_arg(field)? } });
            Some(pretty(client.patch(
                &format!("/dbs/{db}/tables/{table}/schema?force={force}"),
                body,
            )?))
        }
        ("schema", ["rm-field", db, table, rest @ ..]) => {
            let force = rest.contains(&"--force");
            let fields = rest
                .iter()
                .filter(|arg| **arg != "--force")
                .map(|arg| Value::String((*arg).to_string()))
                .collect::<Vec<_>>();
            if fields.is_empty() {
                return Err(anyhow!("uso: schema rm-field <db> <t> <campo>... [--force]"));
            }
            Some(pretty(client.patch(
                &format!("/dbs/{db}/tables/{table}/schema?force={force}"),
                json!({ "remove": fields }),
            )?))
        }
        ("schema", ["update-field", db, table, field, changes, rest @ ..]) => {
            let force = rest.first().is_some_and(|flag| *flag == "--force");
            let body = json!({ "update": { *field: json_arg(changes)? } });
            Some(pretty(client.patch(
                &format!("/dbs/{db}/tables/{table}/schema?force={force}"),
                body,
            )?))
        }
        ("schema", ["unique", action @ ("add" | "rm"), db, table, key, rest @ ..]) => {
            let force = rest.first().is_some_and(|flag| *flag == "--force");
            let operation = if *action == "add" { "unique_add" } else { "unique_remove" };
            let body = json!({ operation: [unique_key_json(key)] });
            Some(pretty(client.patch(
                &format!("/dbs/{db}/tables/{table}/schema?force={force}"),
                body,
            )?))
        }
        ("list", [db, table, rest @ ..]) => {
            let params = parse_list_params(rest)?;
            Some(pretty(client.get(&format!(
                "/dbs/{db}/tables/{table}/records{params}"
            ))?))
        }
        ("get", [db, table, id]) => Some(pretty(client.get(&format!(
            "/dbs/{db}/tables/{table}/records/{id}"
        ))?)),
        ("add", [db, table, body]) => Some(pretty(client.post(
            &format!("/dbs/{db}/tables/{table}/records"),
            json_arg(body)?,
        )?)),
        ("put", [db, table, id, body]) => Some(pretty(client.put(
            &format!("/dbs/{db}/tables/{table}/records/{id}"),
            json_arg(body)?,
        )?)),
        ("rm", [db, table, id]) => {
            client.delete(&format!("/dbs/{db}/tables/{table}/records/{id}"))?;
            Some(format!("registro '{id}' removido"))
        }
        ("query", [db, table, query]) => Some(pretty(client.post(
            &format!("/dbs/{db}/tables/{table}/records/query"),
            json_arg(query)?,
        )?)),
        ("users", []) => Some(pretty(client.get("/users")?)),
        ("user", ["add", username, password, rest @ ..]) => {
            let (role, grants) = parse_user_args(rest)?;
            let value = client.post(
                "/users",
                json!({
                    "username": username,
                    "password": password,
                    "role": role,
                    "grants": grants,
                }),
            )?;
            Some(pretty(value))
        }
        ("user", ["rm", id]) => {
            client.delete(&format!("/users/{id}"))?;
            Some(format!("usuario '{id}' removido"))
        }
        ("grants", [id, grants]) => Some(pretty(client.put(
            &format!("/users/{id}/grants"),
            json!({ "grants": json_arg(grants)? }),
        )?)),
        ("apikey", ["create", username, rest @ ..]) => {
            let id = resolve_user(client, username)?;
            let name = match rest {
                [] => None,
                [name] => Some((*name).to_string()),
                [flag, name] if *flag == "--name" => Some((*name).to_string()),
                _ => return Err(anyhow!("uso: apikey create <usuario> [--name <nome>]")),
            };
            let value = client.post(
                &format!("/users/{id}/api-keys"),
                json!({ "name": name }),
            )?;
            Some(format!(
                "api key criada (guarde agora): {}",
                value["key"].as_str().unwrap_or("")
            ))
        }
        ("apikey", ["list", rest @ ..]) => {
            let id = match rest {
                [] => "self".to_string(),
                [username] => resolve_user(client, username)?,
                _ => return Err(anyhow!("uso: apikey list [usuario]")),
            };
            let path = if id == "self" {
                "/auth/api-keys".to_string()
            } else {
                format!("/users/{id}/api-keys")
            };
            Some(pretty(client.get(&path)?))
        }
        ("apikey", ["revoke", username, key]) => {
            let id = resolve_user(client, username)?;
            Some(pretty(client.delete(&format!(
                "/users/{id}/api-keys/{key}"
            ))?))
        }
        _ => Some(format!("comando desconhecido: {line} (use 'help')")),
    };
    Ok(output)
}

fn unique_key_json(text: &str) -> Value {
    if text.contains('+') {
        Value::Array(
            text.split('+')
                .map(|part| Value::String(part.trim().to_string()))
                .collect(),
        )
    } else {
        Value::String(text.to_string())
    }
}

fn pretty(value: Value) -> String {
    serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string())
}

fn json_arg(text: &str) -> anyhow::Result<Value> {
    serde_json::from_str(text).map_err(|err| anyhow!("json invalido: {err}"))
}

fn parse_list_params(args: &[&str]) -> anyhow::Result<String> {
    let mut params = Vec::new();
    if let Some(limit) = args.first() {
        params.push(format!("limit={}", limit.parse::<usize>()?));
    }
    if let Some(offset) = args.get(1) {
        params.push(format!("offset={}", offset.parse::<usize>()?));
    }
    if params.is_empty() {
        Ok(String::new())
    } else {
        Ok(format!("?{}", params.join("&")))
    }
}

fn parse_user_args(args: &[&str]) -> anyhow::Result<(String, Vec<Value>)> {
    let mut role = "user".to_string();
    let mut grants = Vec::new();
    for arg in args {
        if let Some((db, level)) = arg.split_once(':') {
            grants.push(json!({ "db": db, "level": level }));
        } else {
            role = arg.to_string();
        }
    }
    Ok((role, grants))
}

fn resolve_user(client: &mut Client, username: &str) -> anyhow::Result<String> {
    if username == "self" {
        let me = client.get("/me")?;
        return Ok(me["id"].as_str().unwrap_or("self").to_string());
    }
    let users = client.get("/users")?;
    users
        .as_array()
        .and_then(|users| {
            users
                .iter()
                .find(|user| user["username"] == username)
                .and_then(|user| user["id"].as_str())
        })
        .map(str::to_string)
        .ok_or_else(|| anyhow!("usuario '{username}' nao encontrado"))
}
