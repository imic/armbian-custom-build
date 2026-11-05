#!/bin/bash
# WiFi join (hardened): rescan -> wait iface -> pin MAC -> up -> wpa -> dhcpcd(wait) -> verify inet -> power_save off.
# Exits non-zero unless an IPv4 address is actually obtained.
set -u
echo 1 > /sys/bus/pci/rescan 2>/dev/null
IF=""
for i in $(seq 1 45); do
  IF=$(basename "$(ls -d /sys/class/net/wl* 2>/dev/null | head -1)" 2>/dev/null)
  [ -n "$IF" ] && break
  sleep 1
done
[ -n "$IF" ] || { echo "wifi-join: FAIL no wireless interface after 45s"; exit 1; }
# Optional per-device MAC pin: set WIFI_MAC in /etc/wifi-join.conf (not in this repo).
[ -r /etc/wifi-join.conf ] && . /etc/wifi-join.conf
if [ -n "${WIFI_MAC:-}" ] && [ "$(cat /sys/class/net/$IF/address)" != "$WIFI_MAC" ]; then
  ip link set dev "$IF" down 2>/dev/null
  ip link set dev "$IF" address "$WIFI_MAC" || { echo "wifi-join: FAIL cannot set MAC"; exit 1; }
fi
ip link set dev "$IF" up || exit 1
wpa_supplicant -B -i "$IF" -c /root/wpa.conf || { echo "wifi-join: FAIL wpa_supplicant"; exit 1; }
dhcpcd -w -t 45 "$IF" || { echo "wifi-join: FAIL dhcpcd no lease"; exit 1; }
ip -4 addr show "$IF" | grep -q "inet " || { echo "wifi-join: FAIL no IPv4"; exit 1; }
iw dev "$IF" set power_save off 2>/dev/null
echo "wifi-join: OK $IF $(ip -4 -o addr show "$IF" | awk '{print $4}' | head -1)"
exit 0
