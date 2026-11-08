#!/bin/sh
# Set the WLAN interface's factory MAC, read from the persist
# partition, before the network stack configures it.
#
# Why: mainline ath11k never reads persist, so the interface is
# born with a board-data template address that changes every
# boot (00:03:7f:12:XX:XX family). The stock qca driver reads
# /persist/qca6390/wlan_mac.bin (Intf0MacAddress). This script
# does the same read and applies the address while the link is
# still down, so networkd/wpa_supplicant see the unit's real
# factory address from the start. Fail-open: any problem leaves
# the address untouched and the boot proceeds as before.
set -u
log() { echo "wlan-factory-mac: $*"; }
PART=/dev/disk/by-partlabel/persist
FILE=qca6390/wlan_mac.bin
MP=/run/wlan-mac-persist
[ -e "$PART" ] || { log "no persist partition, skipping"; exit 0; }
mkdir -p "$MP"
if ! mount -o ro,noload "$PART" "$MP" 2>/dev/null; then
	log "persist mount failed, skipping"
	exit 0
fi
hex=""
if [ -f "$MP/$FILE" ]; then
	hex=$(grep '^Intf0MacAddress=' "$MP/$FILE" | head -1 | cut -d= -f2 | tr -d ' \r\n')
fi
umount "$MP" 2>/dev/null
rmdir "$MP" 2>/dev/null
case "$hex" in
	[0-9A-Fa-f][0-9A-Fa-f][0-9A-Fa-f][0-9A-Fa-f][0-9A-Fa-f][0-9A-Fa-f][0-9A-Fa-f][0-9A-Fa-f][0-9A-Fa-f][0-9A-Fa-f][0-9A-Fa-f][0-9A-Fa-f]) ;;
	*) log "no valid Intf0MacAddress found, skipping"; exit 0 ;;
esac
mac=$(echo "$hex" | sed 's/\(..\)\(..\)\(..\)\(..\)\(..\)\(..\)/\1:\2:\3:\4:\5:\6/' | tr 'A-F' 'a-f')
iface=""
i=0
while [ $i -lt 60 ]; do
	for n in wlp1s0 wlan0; do
		if [ -e "/sys/class/net/$n" ]; then iface=$n; break 2; fi
	done
	i=$((i + 1))
	sleep 0.25
done
[ -n "$iface" ] || { log "no wlan interface appeared, skipping"; exit 0; }
if ip link set dev "$iface" address "$mac" 2>/dev/null; then
	log "set $iface address $mac from persist"
else
	log "could not set address on $iface (already up?), leaving as-is"
fi
exit 0
