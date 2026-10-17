# Operação

> Veja também: [HTTP API](api.md) · [Shell e CLI](shell.md) · [Python SDK](sdk.md)

Este documento cobre o uso do LunaDB na prática: onde ficam os dados, como fazer backup, como
ler os logs e o que fazer quando algo dá errado. Ele descreve o comportamento do código como
está hoje (0.0.1); o formato em disco ainda pode mudar antes do 1.0.

## Layout em disco

Tudo fica sob `storage.path` da config (`~/.lunadb` por padrão):

```
~/.lunadb/
├── config.toml              configuração de servidor, storage, root e log (modo 600)
├── meta/
│   ├── users.json           usuários, hashes de senha e de API key
│   ├── roles.json           roles e suas permissões
│   ├── dbs.json             registro de bancos
│   └── master.key           chave mestra de criptografia (modo 600)
├── dbs/
│   └── <db>/
│       └── <tabela>.msgpack um arquivo por tabela (ou .json se format = "json")
└── infra/
    ├── status.json          pid/porta/versão do servidor em execução
    ├── shell_history        histórico do REPL
    └── logs/
        └── lunadb.log.*     logs rotacionados (ver logging.rotation)
```

Dois arquivos importam mais que o resto:

- **`meta/master.key`** deriva as chaves de criptografia por banco. **Se ela for perdida, todo
  campo `encrypted` fica ilegível.** Não é derivada de nada e não há como recuperá-la.
- **`meta/users.json`** guarda os hashes argon2id das senhas e das API keys. Perdê-lo significa
  perder todas as credenciais, mas não os dados.

## Backup

O servidor não tem backup embutido e o `init` não cria nenhum. O backup seguro é uma cópia do
diretório de storage feita **com o servidor parado**, para que nenhuma tabela esteja no meio de
uma escrita:

```bash
# pare o servidor antes (Ctrl+C ou SIGTERM), depois:
tar czf lunadb-backup-$(date +%F).tar.gz -C ~ .lunadb
```

Restaurar é o inverso: pare o servidor, restaure o diretório, suba de novo. Como as escritas são
atômicas (arquivo temporário + rename), uma cópia com o servidor rodando dificilmente fica
corrompida, mas pode pegar uma tabela entre duas mudanças; por isso a cópia confiável é a com o
servidor parado.

A chave mestra faz parte do backup e é necessária para ler campos criptografados. Guarde-a
também em local separado e seguro; manter a única cópia junto dos dados anula o propósito dela.

## Capacidade e limitações conhecidas

O LunaDB guarda **um arquivo por tabela** e mantém a tabela inteira em memória durante a
requisição. Toda mutação lê o arquivo, altera e grava de volta atomicamente, então o custo de
uma escrita cresce com o tamanho da tabela:

- **Uma escrita de registro regrava a tabela inteira.** Inserir um registro numa tabela de 100
  mil registros codifica e faz fsync de todos os 100 mil. As escritas continuam corretas e
  seguras, mas não escalam linearmente.
- **Escritas na mesma tabela são serializadas** (lock de escrita por tabela). Escritas em
  tabelas diferentes rodam em paralelo.
- **Índices `unique` são reconstruídos a cada escrita.** Com campos `unique` no schema, cada
  insert/update percorre todos os registros de novo, somando outra passada completa.
- **Leituras decodificam a tabela inteira** e depois aplicam filtro, ordenação e paginação em
  memória. `limit`/`cursor` limitam o tamanho da resposta, não o trabalho feito.

Orientação prática, na 0.0.1:

| Tamanho da tabela | Comportamento |
|---|---|
| até ~10 mil registros | confortável para uso interativo |
| ~10 mil a 100 mil | cada escrita recodifica o arquivo; agrupe o trabalho em menos requisições maiores |
| acima de ~100 mil | não recomendado; divida os dados em várias tabelas |

**Não há transações multi-tabela** nem índices secundários além de `unique` — uma operação toca
exatamente um arquivo de tabela, então consistência entre tabelas é responsabilidade do cliente.
Como o formato é pré-1.0, trate esses números como orientação operacional e não como contrato;
meça com o tamanho real dos seus registros antes de confiar neles.

## Saúde, status e métricas

| Endpoint | Auth | Para que serve |
|---|---|---|
| `GET /health` | público | só liveness: `{"status":"ok","name":"lunadb","version":"..."}` |
| `GET /status` | admin | `pid`, `version`, `started_at`, `uptime_seconds`, `host`, `port`, `storage`, `config`, `log_file`, `users`, `dbs` |
| `GET /metrics` | público | contadores Prometheus (abaixo) |

O `/health` não toca no storage, então ele responder bem não prova que os dados estão legíveis —
prova que o processo está de pé e atendendo.

O `/metrics` expõe `lunadb_build_info`, `lunadb_uptime_seconds`, `lunadb_requests_total` e
`lunadb_responses_total{status_class="2xx|3xx|4xx|5xx"}`. Uma razão crescente de `4xx`/`5xx` é a
primeira coisa a olhar; os contadores são do processo e zeram quando o servidor reinicia.

## Logs

O log é configurado em `[logging]` no `config.toml`:

```toml
[logging]
level = "info"      # trace | debug | info | warn | error
console = true      # também escreve no stdout
file = true         # escreve em infra/logs/
rotation = "daily"  # never | hourly | daily
max_files = 7       # quantos arquivos rotacionados manter
```

`RUST_LOG` sobrepõe o nível quando definido, por exemplo `RUST_LOG=lunadb=debug lunadb serve`.

Dois tipos de linha importam ao depurar:

- **`lunadb::ops`** tem uma entrada por operação, com `op`, `user`, `db`, `table` e um
  `count`/`id`. Busque pelo nome da operação: `grep 'op="create_record"' lunadb.log`.
- **`http_request`** traz `method`, `uri`, `status` e `latency_ms`. Erros de cliente logam em
  `warn`, erros de servidor em `error`.

Valores sensíveis nunca são logados: nem senhas, nem API keys, e campos criptografados
permanecem cifrados no arquivo da tabela.

## Problemas comuns

### `error: api key missing or invalid` (401)

A chave está ausente, foi revogada ou pertence a um login rotacionado. O `POST /auth/login`
rotaciona apenas a chave chamada `login`; uma chave criada com `lunadb apikey create` continua
válida até ser revogada. Se você trocou a senha, **todas** as chaves desse usuário foram
revogadas e é preciso logar de novo.

### `too many attempts; try again in Ns` (429)

Após 5 falhas consecutivas para o mesmo usuário, o login fica bloqueado por 60 segundos. Um
login bem-sucedido zera o contador. O bloqueio é por usuário, então outra conta não é afetada.

### `no access to db '<nome>'` (403)

O usuário não tem grant que cubra esse banco. O nível efetivo é o **máximo** entre os grants do
próprio usuário e as permissões da role dele, com `*` valendo para qualquer banco:

```bash
curl -s $BASE/me -H "authorization: Bearer $KEY"        # grants e role do próprio usuário
curl -s $BASE/roles -H "authorization: Bearer $KEY"     # permissões das roles (admin)
```

### `option 'admin' required on db '<nome>'` ao excluir um banco

Só um admin (ou um grant de `admin` naquele banco) pode excluí-lo. `admin` é estritamente mais
forte que `write`, então um grant de `write` não basta.

### `schema rejected by existing records` (409)

Uma mudança de schema invalidaria registros; a resposta lista os ids e campos problemáticos.
Corrija os registros antes, ou repita a requisição com `force=true` para manter os registros
inválidos como estão. Por exemplo pelo shell:

```bash
schema set app usuarios '<json>' --force
```

### `failed to decrypt field '<nome>'` (500)

O valor armazenado não pode ser decifrado com a chave mestra atual. Causas, por ordem de
probabilidade: há um `LUNADB_MASTER_KEY` diferente do usado na escrita, o arquivo da chave foi
substituído, ou a chave foi movida entre bancos. **Os dados não são recuperáveis sem a chave
original.** Restaure `meta/master.key` do backup; não apague o campo para "resolver", pois isso
perde dados que uma chave restaurada ainda leria.

### `server appears to be running` no `lunadb migrate`

O `migrate` se recusa a rodar enquanto o servidor segura o storage, porque reescreve todos os
arquivos de tabela. Pare o servidor e tente de novo, ou passe `--force` se tiver certeza. A
checagem lê `infra/status.json` e confirma que o pid registrado ainda está vivo, então um
arquivo deixado por um servidor que caiu não bloqueia você.

### O servidor não sobe

- **`Cannot assign requested address (os error 99)`** — `server.host` é um endereço que não
  existe nesta máquina. Use `127.0.0.1` ou `0.0.0.0`.
- **`Address already in use`** — outro processo ocupa a porta; verifique
  `ss -ltnp | grep 7700`.
- **`failed to read master key at ...`** — o diretório de storage está errado ou incompleto.
  Rode `lunadb init`, ou aponte `storage.path` para o diretório correto.

### Um arquivo de tabela parece corrompido

As escritas são atômicas (arquivo temporário e depois rename), então um arquivo pela metade é
improvável. Se uma tabela não parseia, o arquivo pode ser inspecionado direto: arquivos
`msgpack` são MessagePack, arquivos `json` são JSON identado. Restaure só aquele arquivo de um
backup com o servidor parado. O `lunadb migrate` também converte uma tabela legível entre
formatos offline.

## Atualização de versão

O projeto é pré-1.0 e o formato em disco pode mudar. Antes de atualizar:

1. Pare o servidor e faça backup (acima), incluindo `meta/master.key`.
2. Compile ou instale o binário novo.
3. Apague o `infra/status.json` antigo se estiver obsoleto — ele é recriado na subida.
4. Suba o servidor e confira o `GET /health`, depois faça login e leia uma tabela.

Não há migração automática do layout de storage além do `lunadb migrate`, que apenas converte
uma tabela entre JSON e MessagePack.
