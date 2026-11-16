#!/usr/bin/env bash
# LIN-50 (S-15): Platten-Monitoring mit Alarm, ergänzend zur Byte-Quota je Workspace.
#
# Prüft die Belegung des Dateisystems, auf dem die Lineal-Blobs und Backups liegen,
# und meldet Alarm, sobald ein Schwellwert überschritten ist. Gedacht als Host-Cron
# (nicht im Container), da nur der Host die reale Plattenbelegung kennt.
#
# Konfiguration über Umgebung:
#   LINEAL_DISK_PATHS   Leerzeichenliste zu prüfender Pfade (Default: "/opt/lineal /var/lib/docker")
#   LINEAL_DISK_WARN    Warnschwelle in Prozent (Default: 85)
#   LINEAL_DISK_CRIT    Kritischschwelle in Prozent (Default: 92)
#   LINEAL_DISK_MAILTO  Optionale Mailadresse für den Alarm (nutzt `mail`, sonst nur stderr/Exit-Code)
#
# Exit-Code: 0 = alles ok, 1 = Warnung, 2 = kritisch. So kann auch systemd/cron-Mailrouting greifen.
set -euo pipefail
export LC_ALL=C

paths=${LINEAL_DISK_PATHS:-"/opt/lineal /var/lib/docker"}
warn=${LINEAL_DISK_WARN:-85}
crit=${LINEAL_DISK_CRIT:-92}
mailto=${LINEAL_DISK_MAILTO:-}

worst=0
report=""
for p in $paths; do
  [ -e "$p" ] || continue
  # Belegung des Dateisystems, das $p enthält (Prozent ohne %-Zeichen).
  used=$(df -P "$p" | awk 'NR==2 {gsub("%","",$5); print $5}')
  avail=$(df -Ph "$p" | awk 'NR==2 {print $4}')
  line=$(printf '%-24s %3s%% belegt, %s frei' "$p" "$used" "$avail")
  report+="$line"$'\n'
  [ "$used" -gt "$worst" ] && worst=$used
done

level=OK
code=0
if [ "$worst" -ge "$crit" ]; then
  level=CRITICAL
  code=2
elif [ "$worst" -ge "$warn" ]; then
  level=WARNING
  code=1
fi

msg="Lineal Disk $level (Spitze ${worst}%, Warn ${warn}% / Krit ${crit}%) auf $(hostname)"$'\n'"$report"

if [ "$code" -ne 0 ]; then
  echo "$msg" >&2
  if [ -n "$mailto" ] && command -v mail >/dev/null 2>&1; then
    printf '%s\n' "$msg" | mail -s "$level: Lineal Plattenbelegung ${worst}%" "$mailto"
  fi
else
  echo "$msg"
fi
exit "$code"
