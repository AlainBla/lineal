# SSH-Härtung

## Zweck

`deploy/sshd-hardening.conf` ist ein Drop-in für `sshd`, das die SSH-Anmeldung auf Public-Key
beschränkt und nicht benötigte Funktionen abschaltet. Es gehört auf den **Host**, nicht in den
Container.

## Voraussetzungen

- Root-/Admin-Zugang auf dem Host.
- **Ein funktionierender Public-Key** in `~/.ssh/authorized_keys` des Zielkontos, bevor neu geladen
  wird (sonst Aussperrung).
- `sshd` liest Drop-ins aus `/etc/ssh/sshd_config.d/` (per `Include` prüfen).

## Inhalt / Variablen

Die Datei setzt feste Optionen (keine Laufzeitvariablen):

| Option | Wert | Grund |
|---|---|---|
| `PasswordAuthentication` | `no` | keine Passwort-Anmeldung |
| `KbdInteractiveAuthentication` | `no` | sonst bliebe keyboard-interactive (= Passwort) offen |
| `PermitRootLogin` | `prohibit-password` | root nur per Schlüssel |
| `X11Forwarding` | `no` | nicht gebraucht (Audit-Fund) |
| `AuthenticationMethods` | `publickey` | schließt GSSAPI/keyboard-interactive als Weg aus |

`MaxAuthTries` wird bewusst **nicht** gesenkt (mehrere angebotene Agent-Schlüssel sperren sonst
intermittierend aus). **Niemals echte Admin-IPs** in diese öffentlich versionierte Datei schreiben —
Quell-IP-Beschränkungen gehören in die Host-Firewall (nftables).

Der Dateiname beginnt mit `00-`, damit diese Werte **vor** einer evtl. vorhandenen
`50-cloud-init.conf` greifen (sshd nimmt bei mehrfach gesetzten Optionen den ersten Treffer).

## Beispielaufruf

```bash
cd /opt/<app>

# 1. Bestandsaufnahme: liest sshd Drop-ins? Was setzt cloud-init?
grep -n '^Include' /etc/ssh/sshd_config; ls -1 /etc/ssh/sshd_config.d/ 2>/dev/null
grep -rniE 'passwordauth|kbdinteractive|permitrootlogin' /etc/ssh/sshd_config /etc/ssh/sshd_config.d/

# 2. Drop-in ablegen (00- steht VOR 50-cloud-init), Syntax prüfen
install -m 0644 deploy/sshd-hardening.conf /etc/ssh/sshd_config.d/00-lineal-hardening.conf
sshd -t

# 3. Neu laden – AKTUELLE Sitzung offen lassen!
systemctl reload ssh
```

## Verifikation

```bash
# In einem ZWEITEN Terminal eine neue Schlüssel-Anmeldung testen, BEVOR die erste geschlossen wird.
# Effektive Werte:
sshd -T | grep -Ei 'passwordauth|kbdinteractive|x11forward|permitrootlogin|maxauthtries|authenticationmethods'
```

Optional als Brute-Force-Schutz `fail2ban` mit nftables-Backend (falls der Host nft nutzt).

## Rollback

```bash
rm /etc/ssh/sshd_config.d/00-lineal-hardening.conf
systemctl reload ssh
```

Da die aktive Sitzung beim Reload offen bleibt, lässt sich jede Fehlkonfiguration aus dem noch
offenen Terminal zurücknehmen. Deshalb Schritt für Schritt vorgehen und die erste Sitzung erst
schließen, wenn eine **neue** Anmeldung nachweislich funktioniert.
