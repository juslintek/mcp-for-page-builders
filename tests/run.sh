#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
STATE_FILE="$PROJECT_DIR/.test-env"
COMPOSE_FILE="$PROJECT_DIR/docker-compose.test.yml"

free_port() { python3 -c "import socket; s=socket.socket(); s.bind(('',0)); print(s.getsockname()[1]); s.close()"; }
log() { echo "=== $1 ===" >&2; }
err() { echo "ERROR: $1" >&2; exit 1; }

load_state() { [ -f "$STATE_FILE" ] && source "$STATE_FILE"; }

compose() { docker compose -p "$PROJECT_NAME" -f "$COMPOSE_FILE" "$@"; }

cmd_up() {
  if load_state 2>/dev/null && curl -fsS "http://localhost:$WP_PORT/wp-json/" -o /dev/null 2>/dev/null; then
    log "Already running on :$WP_PORT"
    return 0
  fi

  WP_PORT=$(free_port)
  PROJECT_NAME="emcp-test-${WP_PORT}"
  export WP_PORT WP_ADMIN_USER=admin WP_ADMIN_PASS=admin

  cat > "$STATE_FILE" << EOF
WP_PORT=$WP_PORT
PROJECT_NAME=$PROJECT_NAME
WP_TEST_URL=http://localhost:$WP_PORT
WP_TEST_USER=admin
WP_TEST_PASS=pending
EOF

  log "Starting WordPress on :$WP_PORT"
  compose up -d

  log "Waiting for WordPress (timeout 180s)"
  local elapsed=0
  while ! curl -fsS "http://localhost:$WP_PORT/wp-json/" -o /dev/null 2>/dev/null; do
    sleep 3; elapsed=$((elapsed + 3))
    [ $elapsed -ge 180 ] && { compose logs >&2; err "Not ready after 180s"; }
  done

  local pass
  pass=$(compose exec -T wordpress /bin/sh -c "cat /wp/app-password.txt 2>/dev/null" || echo "")
  [ -n "$pass" ] && sed -i.bak "s/WP_TEST_PASS=.*/WP_TEST_PASS=$pass/" "$STATE_FILE" && rm -f "${STATE_FILE}.bak"

  log "Ready at http://localhost:$WP_PORT"
}

run_tests() {
  local test_args="$@"
  load_state || err "No environment"
  ( cd "$PROJECT_DIR" && source "$STATE_FILE" && export WP_TEST_URL WP_TEST_USER WP_TEST_PASS && cargo test $test_args )
}

cmd_unit()   { log "Unit tests";        ( cd "$PROJECT_DIR" && cargo test --test unit ); }
cmd_test()   { cmd_up; log "Integration tests"; run_tests --test integration -- --nocapture; }
cmd_e2e()    { cmd_up; log "E2E tests (sequential)"; run_tests --test e2e -- --test-threads=1; }
cmd_demo()   { cmd_up; log "Persistent E2E (creates visible content)"; run_tests --test e2e_persistent -- --test-threads=1 --nocapture; echo "Browse: http://localhost:$WP_PORT"; }
cmd_all()    { cmd_up; log "All tests"; run_tests --test unit --test integration --test e2e -- --test-threads=1; }

cmd_down() {
  load_state 2>/dev/null || { rm -f "$STATE_FILE"; return 0; }
  export WP_PORT 2>/dev/null || true
  log "Tearing down ${PROJECT_NAME:-}"
  compose down -v --remove-orphans 2>/dev/null || true
  rm -f "$STATE_FILE"
  log "Cleaned up"
}

cmd_status() {
  load_state 2>/dev/null || { echo "No environment running."; return 0; }
  export WP_PORT
  echo "WordPress: http://localhost:$WP_PORT"
  compose ps 2>/dev/null || echo "  (stopped)"
  echo "Rerun: $0 --retest | --e2e | --all"
  echo "Down:  $0 --down"
}

case "${1:-}" in
  --keep)    cmd_up; cmd_all; echo ""; cmd_status ;;
  --retest)  cmd_test ;;
  --e2e)     cmd_e2e ;;
  --demo)    cmd_demo ;;
  --all)     cmd_all ;;
  --down)    cmd_down ;;
  --status)  cmd_status ;;
  --unit)    cmd_unit ;;
  "")        trap cmd_down EXIT; cmd_unit; cmd_all; log "All tests passed" ;;
  *)         echo "Usage: $0 [--keep|--retest|--e2e|--demo|--all|--down|--status|--unit]"; exit 1 ;;
esac
