#!/usr/bin/env sh
# Smoke test for scripts/luna-setup.sh.
#
# Verifies the generated config is valid, applies the expected values, and that
# the server actually boots from it. Runs with any POSIX shell, so CI can use
# dash to catch bash-only constructs.
#
#   sh scripts/test-luna-setup.sh [path-to-lunadb-binary]
#
# Exits non-zero on the first failure.

set -eu

BINARY="${1:-target/debug/lunadb}"
SETUP="scripts/luna-setup.sh"
PORT=7791

[ -x "$BINARY" ] || { echo "FAIL: binary not found at $BINARY (run cargo build)"; exit 1; }
[ -f "$SETUP" ] || { echo "FAIL: $SETUP not found"; exit 1; }

work="$(mktemp -d)"
pid=""
cleanup() {
    if [ -n "$pid" ]; then
        kill "$pid" 2>/dev/null || true
    fi
    rm -rf "$work"
}
trap cleanup EXIT INT TERM

fail() {
    echo "FAIL: $1"
    exit 1
}

# ---------------------------------------------------------------------------
# 1. the script is valid POSIX sh
# ---------------------------------------------------------------------------

sh -n "$SETUP" || fail "syntax error in $SETUP"

# ---------------------------------------------------------------------------
# 2. defaults: pressing Enter for every prompt
# ---------------------------------------------------------------------------

config="$work/defaults.toml"
printf '\n\n\n\n\n\n\n\n\n\n\n\n\n' | sh "$SETUP" --config "$config" --force >/dev/null 2>&1 \
    || fail "default run exited non-zero"

[ -f "$config" ] || fail "config was not written"

check_default() {
    grep -q "^$1 = $2$" "$config" || fail "expected default '$1 = $2' in config"
}
check_default host "'127.0.0.1'"
check_default port "7700"
check_default path "'~/.lunadb'"
check_default format "'msgpack'"
check_default max_body_bytes "10485760"
check_default request_timeout_secs "30"
check_default username "'root'"
check_default level "'info'"
check_default console "true"
check_default file "true"
check_default rotation "'daily'"
check_default max_files "7"

# a password must have been generated and must not be the insecure default
grep -q "^password = '[^']\+'$" "$config" || fail "no password written"
grep -q "^password = 'changeme'$" "$config" && fail "password must not be 'changeme'"

mode="$(stat -c '%a' "$config" 2>/dev/null || stat -f '%Lp' "$config")"
[ "$mode" = "600" ] || fail "expected mode 600, got $mode"

# ---------------------------------------------------------------------------
# 3. custom values are applied to the right fields
# ---------------------------------------------------------------------------

custom="$work/custom.toml"
storage="$work/storage"
printf '127.0.0.2\n%s\n%s\njson\n3\n45\nsvc\nCustomPass123\ndebug\nno\nno\nhourly\n4\n' \
    "$PORT" "$storage" | sh "$SETUP" --config "$custom" --force >/dev/null 2>&1 \
    || fail "custom run exited non-zero"

check_custom() {
    grep -q "^$1 = $2$" "$custom" || fail "expected '$1 = $2' in custom config"
}
# 127.0.0.2 is a loopback address that exists on Linux, so the value is applied
# and the server below can still bind to it
check_custom host "'127.0.0.2'"
check_custom port "$PORT"
check_custom path "'$storage'"
check_custom format "'json'"
check_custom max_body_bytes "3145728"
check_custom request_timeout_secs "45"
check_custom username "'svc'"
check_custom password "'CustomPass123'"
check_custom level "'debug'"
check_custom console "false"
check_custom file "false"
check_custom rotation "'hourly'"
check_custom max_files "4"

# ---------------------------------------------------------------------------
# 4. validation rejects bad input and retries
# ---------------------------------------------------------------------------

# The output is captured to a file rather than piped into grep -q, because
# grep -q closes the pipe early and would kill the prompt with SIGPIPE.
retry="$work/retry.toml"
retry_log="$work/retry.log"
printf '\nnot-a-port\n70000\n8080\n\n\n\n\n\n\n\n\n\n' \
    | sh "$SETUP" --config "$retry" --force >"$retry_log" 2>&1
grep -q "port must be an integer" "$retry_log" \
    || fail "invalid ports should be rejected with a message"
grep -q "^port = 8080$" "$retry" || fail "the valid port after retries was not applied"

# single quotes are invalid in TOML literal strings and must be refused
quotes="$work/quotes.toml"
quotes_log="$work/quotes.log"
printf "bad'host\nokhost\n\n\n\n\n\nbob's\nbob\n\n\n\n\n\n\n" \
    | sh "$SETUP" --config "$quotes" --force >"$quotes_log" 2>&1
grep -q "single quote" "$quotes_log" || fail "single quotes should be refused"
grep -q "^host = 'okhost'$" "$quotes" || fail "host after the quote retry is wrong"
grep -q "^username = 'bob'$" "$quotes" || fail "username after the quote retry is wrong"

# ---------------------------------------------------------------------------
# 5. an existing config is not overwritten without consent
# ---------------------------------------------------------------------------

if printf 'no\n' | sh "$SETUP" --config "$custom" >/dev/null 2>&1; then
    fail "declining to overwrite must exit non-zero"
fi
grep -q "^host = '127.0.0.2'$" "$custom" || fail "config changed after declining"

# ---------------------------------------------------------------------------
# 6. usage errors
# ---------------------------------------------------------------------------

if sh "$SETUP" --bogus >/dev/null 2>&1; then
    fail "an unknown option must exit non-zero"
fi
if sh "$SETUP" --config >/dev/null 2>&1; then
    fail "--config without a value must exit non-zero"
fi
sh "$SETUP" --help >/dev/null 2>&1 || fail "--help must exit zero"

# ---------------------------------------------------------------------------
# 7. the generated config boots a real server
# ---------------------------------------------------------------------------

"$BINARY" --config "$custom" init >/dev/null 2>&1 || fail "init failed on the generated config"

"$BINARY" --config "$custom" serve >/dev/null 2>&1 &
pid=$!

ready=0
attempts=0
while [ "$attempts" -lt 50 ]; do
    if curl -sf "http://127.0.0.2:$PORT/health" >/dev/null 2>&1; then
        ready=1
        break
    fi
    attempts=$((attempts + 1))
    sleep 0.2
done
[ "$ready" -eq 1 ] || fail "server did not become healthy on the configured port"

key="$(curl -sf -X POST "http://127.0.0.2:$PORT/auth/login" \
    -H 'content-type: application/json' \
    -d '{"username":"svc","password":"CustomPass123"}' \
    | sed -n 's/.*"api_key":"\([^"]*\)".*/\1/p')"
[ -n "$key" ] || fail "login with the configured credentials failed"

curl -sf -X POST "http://127.0.0.2:$PORT/dbs" \
    -H "authorization: Bearer $key" -H 'content-type: application/json' \
    -d '{"name":"smoke"}' >/dev/null || fail "create db failed"
curl -sf -X POST "http://127.0.0.2:$PORT/dbs/smoke/tables" \
    -H "authorization: Bearer $key" -H 'content-type: application/json' \
    -d '{"name":"t"}' >/dev/null || fail "create table failed"
curl -sf -X POST "http://127.0.0.2:$PORT/dbs/smoke/tables/t/records" \
    -H "authorization: Bearer $key" -H 'content-type: application/json' \
    -d '{"ok":true}' >/dev/null || fail "create record failed"

# the storage format chosen in the wizard must be the one on disk
[ -f "$storage/dbs/smoke/t.json" ] || fail "expected t.json (format = json)"

kill "$pid" 2>/dev/null || true
wait "$pid" 2>/dev/null || true
pid=""

echo "OK: luna-setup smoke test passed (shell: ${0##*/}, binary: $BINARY)"
