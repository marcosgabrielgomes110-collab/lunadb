#!/usr/bin/env sh
# LunaDB - interactive configuration setup.
#
# Prompts for the main config.toml values, using the LunaDB defaults when the
# user just presses Enter. Passwords are written in plaintext on purpose: the
# server replaces them with an argon2id hash on the first boot (see bootstrap).
#
#   make luna-setup
#   scripts/luna-setup.sh [--force] [--config PATH]
#
# Exits non-zero when an existing config is present and --force was not given,
# so an accidental run cannot overwrite a working installation.

set -eu

CONFIG_PATH="${LUNADB_CONFIG:-$HOME/.lunadb/config.toml}"
FORCE=0

usage() {
    cat <<'EOF'
Usage: luna-setup.sh [OPTIONS]

Options:
  --config PATH   Where to write the config (default: ~/.lunadb/config.toml)
  --force         Overwrite an existing config without asking
  -h, --help      Show this help
EOF
}

while [ $# -gt 0 ]; do
    case "$1" in
        --config)
            [ $# -ge 2 ] || { echo "error: --config needs a path" >&2; exit 2; }
            CONFIG_PATH="$2"
            shift 2
            ;;
        --force)
            FORCE=1
            shift
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            echo "error: unknown option '$1'" >&2
            usage >&2
            exit 2
            ;;
    esac
done

# ---------------------------------------------------------------------------
# helpers
# ---------------------------------------------------------------------------

# ask_raw <prompt> <default> -> sets REPLY, rejecting single quotes.
# TOML literal strings cannot contain a single quote, so it is rejected at the
# prompt instead of failing later while the config is being written.
ask_raw() {
    while :; do
        ask "$1" "$2"
        case "$REPLY" in
            *"'"*) printf '  values cannot contain a single quote\n' >&2 ;;
            *) return ;;
        esac
    done
}

# ask <prompt> <default> -> sets REPLY.
# The prompt goes to stderr so it is never captured along with the answer, and
# REPLY is a plain variable so validators can read it without a nested subshell
# (a subshell inside a retry loop would lose the stdin of the following reads).
ask() {
    printf '%s' "$1" >&2
    if [ -n "$2" ]; then
        printf ' [%s]' "$2" >&2
    fi
    printf ': ' >&2
    IFS= read -r REPLY || REPLY=""
    if [ -z "$REPLY" ]; then
        REPLY="$2"
    fi
}

# ask_choice <prompt> <default> <opt1|opt2|...>
ask_choice() {
    while :; do
        ask "$1 ($3)" "$2"
        case "$3" in
            *"$REPLY"*) return ;;
            *) printf '  expected one of: %s\n' "$3" >&2 ;;
        esac
    done
}

# ask_bool <prompt> <default yes|no> -> true/false
ask_bool() {
    ask_choice "$1" "$2" "yes|no"
    case "$REPLY" in
        yes) REPLY=true ;;
        no) REPLY=false ;;
    esac
}

is_positive_int() {
    case "$1" in
        ''|*[!0-9]*) return 1 ;;
        *) [ "$1" -gt 0 ] 2>/dev/null || return 1 ;;
    esac
}

# ask_port -> sets REPLY
ask_port() {
    while :; do
        ask "Server port" "7700"
        if is_positive_int "$REPLY" && [ "$REPLY" -le 65535 ]; then
            return
        fi
        printf '  the port must be an integer between 1 and 65535\n' >&2
    done
}

# ask_int <prompt> <default> -> sets REPLY
ask_int() {
    while :; do
        ask "$1" "$2"
        if is_positive_int "$REPLY"; then
            return
        fi
        printf '  expected a positive integer\n' >&2
    done
}

random_password() {
    if command -v openssl >/dev/null 2>&1; then
        openssl rand -base64 18 | tr -d '/+=' | cut -c1-24
    elif [ -r /dev/urandom ]; then
        LC_ALL=C tr -dc 'A-Za-z0-9' < /dev/urandom | cut -c1-24
    else
        # last resort: derive from time and pid, still better than a fixed value
        printf 'luna%s%s' "$(date +%s)" "$$"
    fi
}

# ---------------------------------------------------------------------------
# prompts
# ---------------------------------------------------------------------------

if [ -f "$CONFIG_PATH" ] && [ "$FORCE" -eq 0 ]; then
    printf 'config already exists at %s\n' "$CONFIG_PATH"
    printf 'overwrite? (yes/no) [no]: '
    IFS= read -r confirm || confirm=""
    case "$confirm" in
        yes|y|Y|YES) ;;
        *)
            echo "aborted; nothing was changed"
            exit 1
            ;;
    esac
fi

echo "LunaDB setup - press Enter to accept the value in brackets"
echo

ask_raw 'Server host' '127.0.0.1'
host="$REPLY"
ask_port
port="$REPLY"
ask_raw 'Storage path' '~/.lunadb'
storage="$REPLY"
ask_choice 'Storage format' 'msgpack' 'json|msgpack'
format="$REPLY"
ask_int 'Max request body (MB)' '10'
max_body_mb="$REPLY"
ask_int 'Request timeout (seconds)' '30'
timeout="$REPLY"
echo
ask_raw 'Root username' 'root'
username="$REPLY"
printf 'Root password (Enter generates a random one): ' >&2
IFS= read -r password || password=""
while :; do
    case "$password" in
        *"'"*) printf '  values cannot contain a single quote\n' >&2 ;;
        *) break ;;
    esac
    printf 'Root password (Enter generates a random one): ' >&2
    IFS= read -r password || password=""
done
if [ -z "$password" ]; then
    password="$(random_password)"
    generated=1
else
    generated=0
fi
echo
ask_choice 'Log level' 'info' 'trace|debug|info|warn|error'
level="$REPLY"
ask_bool 'Log to console' 'yes'
console="$REPLY"
ask_bool 'Log to file' 'yes'
logfile="$REPLY"
ask_choice 'Log rotation' 'daily' 'never|hourly|daily'
rotation="$REPLY"
ask_int 'Max log files' '7'
max_files="$REPLY"

# TOML uses single-quoted literal strings, where backslashes and quotes are
# taken verbatim, so no escaping is needed for paths. Single quotes are already
# rejected while prompting (ask_raw), which keeps every value writable here.
toml_string() {
    printf "'%s'" "$1"
}

# ---------------------------------------------------------------------------
# write
# ---------------------------------------------------------------------------

config_dir="$(dirname "$CONFIG_PATH")"
mkdir -p "$config_dir"

# write to a temp file first, so a failure never leaves a half-written config
tmp="$CONFIG_PATH.tmp.$$"
trap 'rm -f "$tmp"' EXIT INT TERM

cat > "$tmp" <<EOF
# LunaDB - configuration file
# Generated by scripts/luna-setup.sh
#
# The root password is stored in plaintext here and replaced by an argon2id
# hash on the first boot of the server.

[server]
host = $(toml_string "$host")
port = $port
max_body_bytes = $((max_body_mb * 1024 * 1024))
request_timeout_secs = $timeout

[storage]
path = $(toml_string "$storage")
format = $(toml_string "$format")

[root]
username = $(toml_string "$username")
password = $(toml_string "$password")

[encryption]

[logging]
level = $(toml_string "$level")
console = $console
file = $logfile
rotation = $(toml_string "$rotation")
max_files = $max_files
EOF

# the config holds the root password until the first boot hashes it
chmod 600 "$tmp"
mv "$tmp" "$CONFIG_PATH"
trap - EXIT INT TERM

echo
echo "config written to $CONFIG_PATH"
echo "  server:  http://$host:$port"
echo "  storage: $storage ($format)"
if [ "$generated" -eq 1 ]; then
    echo
    echo "  generated root password: $password"
    echo "  store it now; it is shown only once"
fi
echo
echo "next steps:"
echo "  make init    # creates the storage layout and hashes the password"
echo "  make serve   # starts the API"
