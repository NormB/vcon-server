#!/usr/bin/env bash
# Starts the PostgreSQL servers the PostgreSQL catalog's tests run against
# (crates/vcon-server/src/backend/postgres/tests.rs), in containers, and
# removes them.
#
#   scripts/pg-test-server.sh start <image> <dir>
#   scripts/pg-test-server.sh stop <dir>
#
# start makes <dir> (it must not exist), a CA and a second CA, a server
# certificate for localhost, 127.0.0.1, ::1 and the name `primary`, client
# certificates, and a random password, then starts from <image>, on a
# network of its own:
#
#   - a primary with TLS, whose pg_hba.conf gives each test role one way to
#     log in (scram-sha-256, md5, password, trust, cert, or scram-sha-256
#     with a client certificate) and refuses every connection without TLS;
#   - a hot standby of it, copied by pg_basebackup, with the same TLS;
#   - a server with ssl off.
#
# Each port is published on the loopback address only, the primary's on
# ::1 too. <dir>/env holds the VCON_TEST_PG_* lines the tests read, and
# <dir>/containers and <dir>/network what stop removes: stop removes only
# what start recorded, then <dir>.
set -euo pipefail

die() { echo "pg-test-server: $*" >&2; exit 1; }

# Waits up to 60 seconds for the server in container $1 to take TCP
# connections, then prints its log and fails.
ready() {
  for _ in $(seq 1 120); do
    if docker exec "$1" pg_isready -q -h 127.0.0.1 -p 5432 2>/dev/null; then
      return 0
    fi
    sleep 0.5
  done
  docker logs "$1" >&2 || true
  die "the server in $1 never became ready"
}

# The host port container $1 publishes 5432 on, for the address $2.
port_of() {
  local line addr
  while read -r line; do
    addr="${line%:*}"
    addr="${addr#[}"
    addr="${addr%]}"
    if [ "$addr" = "$2" ]; then
      echo "${line##*:}"
      return 0
    fi
  done < <(docker port "$1" 5432/tcp)
}

# A key and a certificate signed by the CA $1, for subject $2, with the
# extensions $3, as $4.key and $4.crt.
signed() {
  local ca="$1" subject="$2" ext="$3" out="$4"
  openssl req -new -newkey ec -pkeyopt ec_paramgen_curve:P-256 -nodes \
    -keyout "$out.key" -out "$out.csr" -subj "$subject" 2>/dev/null
  printf '%s\n' "$ext" > "$out.ext"
  openssl x509 -req -in "$out.csr" -CA "$ca.pem" -CAkey "$ca.key" -CAcreateserial \
    -days 2 -sha256 -extfile "$out.ext" -out "$out.crt" 2>/dev/null
  rm -f "$out.csr" "$out.ext"
  chmod 0600 "$out.key"
}

# A self-signed CA as $1.key and $1.pem.
authority() {
  openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:P-256 -nodes -days 2 \
    -keyout "$1.key" -out "$1.pem" -subj "/CN=$2" \
    -addext "basicConstraints=critical,CA:TRUE" \
    -addext "keyUsage=critical,keyCertSign,cRLSign" 2>/dev/null
}

start() {
  local image="$1" dir="$2"
  [ -e "$dir" ] && die "$dir exists; start makes it"
  mkdir -m 0700 "$dir"
  dir="$(cd "$dir" && pwd)"
  : > "$dir/containers"
  local major
  major="$(docker image inspect --format '{{range .Config.Env}}{{println .}}{{end}}' "$image" | sed -n 's/^PG_MAJOR=//p')"
  [ -n "$major" ] || die "cannot read the server version of $image"

  authority "$dir/ca" "vcon-server test CA"
  authority "$dir/wrong-ca" "another test CA"
  mkdir -m 0700 "$dir/server"
  signed "$dir/ca" "/CN=localhost" \
    "subjectAltName=DNS:localhost,DNS:primary,IP:127.0.0.1,IP:::1
basicConstraints=critical,CA:FALSE
extendedKeyUsage=serverAuth" "$dir/server/server"
  for user in vcon_cert vcon_both; do
    signed "$dir/ca" "/CN=$user" "basicConstraints=critical,CA:FALSE
extendedKeyUsage=clientAuth" "$dir/$user"
  done
  signed "$dir/wrong-ca" "/CN=vcon_cert" "basicConstraints=critical,CA:FALSE
extendedKeyUsage=clientAuth" "$dir/stranger"
  cp "$dir/ca.pem" "$dir/server/ca.pem"
  rm -f "$dir/ca.key" "$dir/wrong-ca.key" "$dir/ca.srl" "$dir/wrong-ca.srl"
  openssl rand -hex 24 > "$dir/password"
  chmod 0600 "$dir/password"

  cat > "$dir/server/pg_hba.conf" <<'HBA'
local all all trust
hostssl replication replicator all scram-sha-256
hostssl all vcon_md5 all md5
hostssl all vcon_plain all password
hostssl all vcon_trust all trust
hostssl all vcon_cert all cert
hostssl all vcon_both all scram-sha-256 clientcert=verify-full
hostssl all all all scram-sha-256
host all all all reject
HBA
  cat > "$dir/server/init.sh" <<'INIT'
#!/bin/sh
set -eu
psql -v ON_ERROR_STOP=1 -v pw="$VCON_TEST_PASSWORD" --username postgres <<'SQL'
CREATE ROLE vcon_app LOGIN PASSWORD :'pw';
CREATE ROLE vcon_owner NOLOGIN;
CREATE DATABASE vcon OWNER vcon_owner;
CREATE ROLE vcon_migrate LOGIN PASSWORD :'pw';
GRANT vcon_owner TO vcon_migrate;
CREATE ROLE vcon_tester LOGIN CREATEDB PASSWORD :'pw';
GRANT vcon_owner TO vcon_tester;
CREATE ROLE vcon_stranger LOGIN PASSWORD :'pw';
CREATE ROLE vcon_cert LOGIN;
CREATE ROLE vcon_both LOGIN PASSWORD :'pw';
CREATE ROLE vcon_trust LOGIN;
CREATE ROLE vcon_super LOGIN SUPERUSER PASSWORD :'pw';
CREATE ROLE vcon_bypass LOGIN BYPASSRLS PASSWORD :'pw';
CREATE ROLE vcon_ro LOGIN PASSWORD :'pw';
ALTER ROLE vcon_ro SET default_transaction_read_only = on;
CREATE ROLE replicator LOGIN REPLICATION PASSWORD :'pw';
CREATE ROLE "vcon_ünï" LOGIN PASSWORD :'pw';
CREATE DATABASE "vcon_数据" OWNER "vcon_ünï";
SET password_encryption = 'md5';
CREATE ROLE vcon_md5 LOGIN PASSWORD :'pw';
CREATE ROLE vcon_plain LOGIN PASSWORD :'pw';
SQL
INIT
  chmod 0644 "$dir/server/init.sh" "$dir/server/pg_hba.conf" "$dir/server/ca.pem" "$dir/server/server.crt"

  local net="vcon-pg-test-$(openssl rand -hex 4)"
  docker network create "$net" > /dev/null
  echo "$net" > "$dir/network"
  local password
  password="$(cat "$dir/password")"
  # The certificates are copied into the container, owned by postgres,
  # since the server refuses a key file others may read.
  local setup='set -e
run_as="$(command -v su-exec || command -v gosu)"
install -d -o postgres -m 0700 /pgconf
install -o postgres -m 0600 /certs/server.key /pgconf/server.key
install -o postgres -m 0644 /certs/server.crt /certs/ca.pem /certs/pg_hba.conf /pgconf/'
  local tls='-c ssl=on -c ssl_cert_file=/pgconf/server.crt -c ssl_key_file=/pgconf/server.key -c ssl_ca_file=/pgconf/ca.pem -c hba_file=/pgconf/pg_hba.conf'

  local primary
  primary="$(docker run -d --network "$net" --network-alias primary \
    -p 127.0.0.1:0:5432 -p '[::1]:0:5432' \
    -e POSTGRES_PASSWORD="$password" -e VCON_TEST_PASSWORD="$password" \
    -v "$dir/server:/certs:ro" --entrypoint sh "$image" -c "$setup
install -d /docker-entrypoint-initdb.d
install -m 0755 /certs/init.sh /docker-entrypoint-initdb.d/init.sh
exec docker-entrypoint.sh postgres $tls")"
  echo "$primary" >> "$dir/containers"
  ready "$primary"

  local standby
  standby="$(docker run -d --network "$net" -p 127.0.0.1:0:5432 \
    -e PGPASSWORD="$password" -v "$dir/server:/certs:ro" --entrypoint sh "$image" -c "$setup
install -d -o postgres -m 0700 /standby
\"\$run_as\" postgres pg_basebackup -D /standby -R -X stream \
  -d 'host=primary user=replicator sslmode=verify-full sslrootcert=/pgconf/ca.pem'
exec \"\$run_as\" postgres postgres -D /standby $tls")"
  echo "$standby" >> "$dir/containers"
  ready "$standby"

  local plain
  plain="$(docker run -d --network "$net" -p 127.0.0.1:0:5432 \
    -e POSTGRES_PASSWORD="$password" "$image")"
  echo "$plain" >> "$dir/containers"
  ready "$plain"

  {
    echo "VCON_TEST_PG_DIR=$dir"
    echo "VCON_TEST_PG_MAJOR=$major"
    echo "VCON_TEST_PG_PORT=$(port_of "$primary" 127.0.0.1)"
    echo "VCON_TEST_PG_PORT6=$(port_of "$primary" ::1)"
    echo "VCON_TEST_PG_STANDBY_PORT=$(port_of "$standby" 127.0.0.1)"
    echo "VCON_TEST_PG_PLAIN_PORT=$(port_of "$plain" 127.0.0.1)"
  } > "$dir/env"
  grep -q '=$' "$dir/env" && die "a port was not published: $(cat "$dir/env")"
  cat "$dir/env"
}

stop() {
  local dir="$1"
  [ -d "$dir" ] || die "$dir is not a directory start made"
  [ -f "$dir/containers" ] || die "$dir has no containers file; it was not made by start"
  while read -r id; do
    [ -n "$id" ] && docker rm -f "$id" > /dev/null
  done < "$dir/containers"
  if [ -f "$dir/network" ]; then
    docker network rm "$(cat "$dir/network")" > /dev/null
  fi
  rm -rf "$dir"
}

case "${1:-}" in
  start) [ $# -eq 3 ] || die "usage: $0 start <image> <dir>"; start "$2" "$3" ;;
  stop) [ $# -eq 2 ] || die "usage: $0 stop <dir>"; stop "$2" ;;
  *) die "usage: $0 start <image> <dir> | stop <dir>" ;;
esac
