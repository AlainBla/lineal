# TLS / CAA

## Zweck

Generalisierte Notizen zum TLS-Abschluss und zu CAA-DNS-Einträgen für eine selbst gehostete
Installation. TLS terminiert ein Reverse-Proxy auf dem Host (z. B. Caddy mit automatischem
Let's-Encrypt-Zertifikat) **vor** dem `web`-Container; die App setzt HSTS
(`max-age=31536000; includeSubDomains`, siehe `deploy/security-headers.conf`). CAA-Einträge
begrenzen, welche Zertifizierungsstellen (CAs) für die Domain ausstellen dürfen.

> Alle Hosts hier sind Platzhalter (`example.org`, `lineal.example.org`,
> `lineal-files.example.org`). Durch die eigenen Domains ersetzen. Konkrete Issuer/Adressen gehören
> ins private Infra-Repo, nicht hierher.

## Voraussetzungen

- DNS-Verwaltung für die Domain (`example.org`).
- Reverse-Proxy mit automatischem Zertifikat (Caddy o. Ä.).
- Wissen, welche CA der Proxy tatsächlich nutzt (Caddy 2.x: Let's Encrypt mit ZeroSSL-Fallback,
  ZeroSSL stellt unter `sectigo.com` aus).

## Variablen / Platzhalter

| Platzhalter | Bedeutung |
|---|---|
| `example.org` | Apex-Domain (CAA auf dem Apex gilt auch für Subdomains) |
| `lineal.example.org` | App-Origin |
| `lineal-files.example.org` | Datei-Origin (erbt CAA vom Apex) |
| `<security@example.org>` | `iodef`-Kontakt für CAA-Verstöße |

## Beispiel: tatsächliche CA prüfen, dann CAA setzen

```bash
# Welche CA stellt aktuell aus? (erst prüfen, dann CAA setzen – sonst blockiert man die Erneuerung)
echo | openssl s_client -connect lineal.example.org:443 2>/dev/null | openssl x509 -noout -issuer
```

CAA auf dem Apex `example.org` gilt für alle Subdomains. Nutzen andere Hosts andere CAs, entweder
alle CAs auflisten oder CAA nur auf `lineal.example.org` scoped setzen. Beispiel (Issuer an die
eigene CA anpassen):

```
example.org.         CAA 0 issue "letsencrypt.org"
example.org.         CAA 0 issue "sectigo.com"        # nur falls ZeroSSL-Fallback aktiv bleibt
example.org.         CAA 0 iodef "mailto:<security@example.org>"
```

## Verifikation

```bash
# CAA vorhanden?
dig +short CAA example.org

# Zertifikat/Issuer nach einer Erneuerung erneut prüfen
echo | openssl s_client -connect lineal.example.org:443 2>/dev/null | openssl x509 -noout -issuer -dates
```

Nach dem Setzen eine Testerneuerung (bzw. den nächsten automatischen Lauf) beobachten: Ein zu enges
CAA (nur eine CA, obwohl der Proxy eine andere nutzt) verhindert die Ausstellung.

## Rollback

CAA ist ein reiner DNS-Eintrag: Zum Zurücknehmen den Record löschen oder um die benötigte CA
erweitern. Da HSTS `includeSubDomains` aktiv ist, muss jede genutzte Subdomain (auch die
Datei-Origin) gültiges HTTPS liefern — beim Entfernen von Subdomains das im Blick behalten.
