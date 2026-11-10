#!/usr/bin/env bash
# S-07/LIN-34: Abgesicherter Prod-Deploy. Statt `git pull` wird nur ein GÜLTIG SIGNIERTER, geprüfter
# Commit ausgecheckt und gebaut. Der zur Prüfung nötige öffentliche Signierschlüssel liegt NUR auf
# dem Host (git-Config `gpg.ssh.allowedSignersFile`), nicht auf GitHub. Ein übernommenes GitHub-Konto
# kann zwar auf `main` pushen, aber keinen Commit erzeugen, den dieser Deploy als gültig signiert
# akzeptiert – er bricht dann ab, bevor etwas gebaut oder gestartet wird.
#
# Härtung (automatischer Security-Review):
#   - `gpg.minTrustLevel=fully`: eine SSH-Signatur zählt nur, wenn ihr Schlüssel in
#     .allowed_signers steht (sonst TRUST_UNDEFINED → Abbruch). stderr wird NICHT verschluckt.
#   - DEPLOY_SIGNER (optional): erwarteter Principal; die Signatur muss von ihm stammen.
#   - Fast-Forward-Zwang: der Ziel-Commit muss Nachfolger des aktuell deployten sein (kein
#     Rollback auf einen alten, aber gültig signierten Commit). Override: DEPLOY_ALLOW_NONFF=1.
#
# Voraussetzung auf dem Host (einmalig, siehe LIN-34-Hand-off):
#   git config gpg.format ssh
#   git config gpg.ssh.allowedSignersFile /opt/lineal/.allowed_signers
#   # .allowed_signers:  <principal> namespaces="git" <ssh-pubkey>
#
# Nutzung (auf dem Host, in /opt/lineal):  scripts/deploy.sh [<ref>]
#   <ref> ist Branch, Tag oder SHA; Vorgabe origin/main.
set -euo pipefail
cd "$(dirname "$0")/.."

REF="${1:-origin/main}"
SIGNER="${DEPLOY_SIGNER:-}"

echo "==> git fetch"
git fetch --prune --tags origin

# Ziel-Commit auflösen (bei einem Tag der Commit, auf den er zeigt).
commit="$(git rev-parse --verify "${REF}^{commit}")"

echo "==> Signatur prüfen: $REF ($commit)"
# minTrustLevel=fully: ohne Eintrag in allowed_signers ist der Trust-Level nicht "fully" → Abbruch.
# stderr wird mitgelesen und bei Erfolg wie Fehler ausgegeben (enthält "Good ... signature for <p>").
if ! out="$(git -c gpg.minTrustLevel=fully verify-commit "$commit" 2>&1)"; then
  echo "ABBRUCH: $REF ($commit) ist nicht gültig signiert. Nichts deployt." >&2
  echo "$out" >&2
  exit 1
fi
echo "$out"
if [ -n "$SIGNER" ] && ! grep -qF "$SIGNER" <<<"$out"; then
  echo "ABBRUCH: Signatur nicht vom erwarteten Principal ($SIGNER). Nichts deployt." >&2
  exit 1
fi

# Rollback-Schutz: nur vorwärts (Ziel-Commit muss Nachfolger des aktuell ausgecheckten sein).
current="$(git rev-parse --verify HEAD)"
if [ "$current" != "$commit" ] && ! git merge-base --is-ancestor "$current" "$commit"; then
  if [ "${DEPLOY_ALLOW_NONFF:-}" = "1" ]; then
    echo "WARNUNG: $commit ist kein Nachfolger von $current – per DEPLOY_ALLOW_NONFF=1 erlaubt." >&2
  else
    echo "ABBRUCH: $commit ist kein Nachfolger des aktuell deployten $current (Rollback?)." >&2
    echo "Absichtlich? Erneut mit DEPLOY_ALLOW_NONFF=1 ausführen." >&2
    exit 1
  fi
fi

echo "==> Auschecken (detached) $commit"
git checkout --quiet --detach "$commit"

echo "==> Build & Start"
docker compose --profile app up -d --build

echo "==> Deploy fertig: $commit"
