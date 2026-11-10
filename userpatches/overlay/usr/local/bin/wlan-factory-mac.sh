#!/bin/sh
# Publish the WLAN factory MAC, read from the persist partition,
# as a udev .link file so the interface is born with it.
#
# Why: mainline ath11k never reads persist, so the interface is
# born with a board-data template address that changes every
# boot (00:03:7f:12:XX:XX family). The stock qca driver reads
# /persist/qca6390/wlan_mac.bin (Intf0MacAddress). An earlier
# version of this script applied that address with ip link set;
# on real boots something always has the interface up by the
# time a service can run and ath11k then refuses the change.
# A .link file is applied by udev at device creation, before
# any service starts, so nothing can race it. First run writes
# the file; from the next boot on the interface is born with
# the factory address. Fail-open: any problem leaves the
# address untouched and the boot proceeds as before.
#
# The generated file must also carry Name=wlp1s0: for .link
# files the first matching file wins wholesale, so this file
# (10-...) shadows 99-default.link and its NamePolicy - without
# an explicit name the interface keeps its kernel name wlan0
# and every wlp1s0-keyed config (netplan, networkd, supplicant)
# misses it (hit on the first .link boot, 2026-11-10: factory
# MAC applied, interface named wlan0, no WiFi).
set -u
log() { echo "wlan-factory-mac: $*"; }
PART=/dev/disk/by-partlabel/persist
FILE=qca6390/wlan_mac.bin
MP=/run/wlan-mac-persist
LINK=/etc/systemd/network/10-wlp1s0.link
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
	*) log "no valid Intf0MacAddress found, skipping" ; exit 0 ;;
esac
mac=$(echo "$hex" | sed 's/\(..\)\(..\)\(..\)\(..\)\(..\)\(..\)/\1:\2:\3:\4:\5:\6/' | tr 'A-F' 'a-f')
if [ -f "$LINK" ] && grep -q "^MACAddress=$mac\$" "$LINK" && grep -q "^Name=wlp1s0\$" "$LINK"; then
	log "link file already carries $mac"
	exit 0
fi
mkdir -p "$(dirname "$LINK")"
tmp="$LINK.tmp"
{
	echo "# Written by wlan-factory-mac from the persist factory address."
	echo "[Match]"
	echo "Driver=ath11k_pci"
	echo ""
	echo "[Link]"
	echo "Name=wlp1s0"
	echo "MACAddress=$mac"
} > "$tmp" || { log "could not write link file, skipping"; exit 0; }
chmod 644 "$tmp"
mv "$tmp" "$LINK"
log "wrote $LINK with factory address $mac"
exit 0
