#!/usr/bin/env bash
# End-to-end test against a real gpg-agent.
#
# Uses a throwaway GNUPGHOME so the caller's own GPG configuration, keyring
# and running agent are left completely alone.
#
# Usage:
#   scripts/test-gpg-integration.sh [tty|dms] [keygen]
#
# With `dms` the prompt appears in the shell and you must type the passphrase
# shown below. With `tty` the prompt is drawn on this terminal.
#
# With `keygen` the test key's passphrase is also chosen through pinentry,
# exercising gpg-agent's new-passphrase flow: re-entry, mismatches and the
# weak-passphrase confirmation. You then type your chosen passphrase to sign.

set -uo pipefail

UI="${1:-dms}"
MODE="${2:-sign}"
PASSPHRASE="integration-test-passphrase"
BINARY="$(cd "$(dirname "$0")/.." && pwd)/target/release/dank-pinentry"
GNUPGHOME="$(mktemp -d /tmp/dank-pinentry-gpg.XXXXXX)"
export GNUPGHOME

# shellcheck disable=SC2317,SC2329  # invoked indirectly, via the EXIT trap below
cleanup() {
	# Stop the throwaway agent before removing its home, or it lingers
	# holding a socket in a deleted directory.
	gpgconf --kill gpg-agent >/dev/null 2>&1 || true
	rm -rf "$GNUPGHOME"
}
trap cleanup EXIT

if [[ ! -x $BINARY ]]; then
	echo "error: $BINARY not built (cargo build --release)" >&2
	exit 1
fi

chmod 700 "$GNUPGHOME"
cat >"$GNUPGHOME/gpg-agent.conf" <<EOF
pinentry-program $BINARY
# Zero caching, so every operation actually reaches pinentry instead of
# being served from the agent's cache.
default-cache-ttl 0
max-cache-ttl 0
EOF

export DANK_PINENTRY_UI="$UI"

echo "GNUPGHOME=$GNUPGHOME"
echo "pinentry-program=$BINARY"
echo "ui=$UI"
echo

if [[ $MODE == keygen ]]; then
	echo "==> generating a test key; choose its passphrase in the prompt"
	echo "    a short one (under 8 characters) triggers the weak-passphrase question"
	if ! timeout 300 gpg --batch --yes \
		--quick-generate-key "Pinentry Dank Test <test@example.invalid>" \
		default default never >"$GNUPGHOME/genkey.log" 2>&1; then
		echo "FAIL: key generation failed" >&2
		tail -20 "$GNUPGHOME/genkey.log" >&2
		exit 1
	fi
	PASSPHRASE="the passphrase you just chose"
else
	echo "==> generating a test key (passphrase supplied in batch mode)"
	if ! gpg --batch --yes --pinentry-mode loopback \
		--passphrase "$PASSPHRASE" \
		--quick-generate-key "Pinentry Dank Test <test@example.invalid>" \
		default default never >"$GNUPGHOME/genkey.log" 2>&1; then
		echo "FAIL: key generation failed" >&2
		tail -20 "$GNUPGHOME/genkey.log" >&2
		exit 1
	fi
fi
echo "    key created"

# Drop anything the generation step may have cached.
gpgconf --reload gpg-agent >/dev/null 2>&1

echo
echo "==> signing, which must go through dank-pinentry"
echo "    PASSPHRASE TO TYPE: $PASSPHRASE"
echo

if echo "integration test payload" |
	timeout 90 gpg --batch --yes --sign --output "$GNUPGHOME/out.sig" 2>"$GNUPGHOME/sign.log"; then
	echo
	echo "PASS: signature produced at \$GNUPGHOME/out.sig"
	echo
	echo "==> verifying"
	if gpg --verify "$GNUPGHOME/out.sig" >/dev/null 2>&1; then
		echo "PASS: signature verifies"
		exit 0
	fi
	echo "FAIL: signature does not verify" >&2
	exit 1
fi

echo >&2
echo "FAIL: signing failed" >&2
echo "--- gpg log ---" >&2
tail -20 "$GNUPGHOME/sign.log" >&2
exit 1
