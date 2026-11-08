#!/bin/sh
# Provision the QCA6390 Bluetooth controller's public address.
#
# The controller registers as an UNCONFIGURED controller and only
# graduates to the configured index list once a public address has
# been set over mgmt - bluetoothd alone never does this, so without
# this step there is no usable Bluetooth at all. Setting the address
# makes the driver re-run its full setup; that is safe now that the
# qca6390-pmu power sequencer owns the BT enable line (tlmm 21) and
# the controller can really power-cycle (with the old GPIO hog the
# same re-setup wedged the controller - the run #7 failure).
#
# Address: the factory WLAN address (Intf0MacAddress in
# qca6390/wlan_mac.bin on the persist partition) minus one - the
# usual factory block allocation on these combo chips (on the
# reference unit the file assigns ...C6 to P2P and ...C8 to STA,
# leaving ...C7, the slot BT occupies). Persist survives
# reflashing, so the BT address is stable per unit and pairings
# survive image changes. Fallback when persist has no usable
# entry: the previous machine-id derivation (locally administered
# 42:...), kept so odd units still get a stable-per-install
# address.
#
# Delivery: mgmt calls BLOCK for tens of seconds while bluetoothd
# is still initializing its adapter (measured 2026-11-08: the
# first btmgmt info of a cold boot did not return inside the old
# 45 s unit budget; the same call takes ~0.1 s once the stack has
# settled). Every call is therefore wrapped in timeout 5 and the
# whole sequence retries patiently inside a 180 s unit budget -
# a timed-out call just means "stack not ready yet".
set -u
log() { echo "bt-provision: $*"; }

# --- address ---------------------------------------------------
addr=""
PART=/dev/disk/by-partlabel/persist
MP=/run/bt-provision-persist
hex=""
if [ -e "$PART" ]; then
	mkdir -p "$MP"
	if mount -o ro,noload "$PART" "$MP" 2>/dev/null; then
		if [ -f "$MP/qca6390/wlan_mac.bin" ]; then
			hex=$(grep '^Intf0MacAddress=' "$MP/qca6390/wlan_mac.bin" | head -1 | cut -d= -f2 | tr -d ' \r\n')
		fi
		umount "$MP" 2>/dev/null
	fi
	rmdir "$MP" 2>/dev/null
fi
case "$hex" in
	[0-9A-Fa-f][0-9A-Fa-f][0-9A-Fa-f][0-9A-Fa-f][0-9A-Fa-f][0-9A-Fa-f][0-9A-Fa-f][0-9A-Fa-f][0-9A-Fa-f][0-9A-Fa-f][0-9A-Fa-f][0-9A-Fa-f])
		num=$(( (0x$hex - 1) & 0xFFFFFFFFFFFF ))
		addr=$(printf '%012X' "$num" | sed 's/\(..\)\(..\)\(..\)\(..\)\(..\)\(..\)/\1:\2:\3:\4:\5:\6/')
		log "address from persist: $addr (Intf0 minus 1)"
		;;
	*)
		mac="$( (cat /etc/machine-id; echo bluetooth) | sha256sum )"
		addr="42:$(echo "$mac" | cut -c1-2):$(echo "$mac" | cut -c5-6):$(echo "$mac" | cut -c9-10):$(echo "$mac" | cut -c13-14):$(echo "$mac" | cut -c17-18)"
		log "persist unusable, fallback address: $addr"
		;;
esac

# --- wait for the controller (sysfs poll, no mgmt cost) --------
i=0
while [ $i -lt 60 ]; do
	[ -e /sys/class/bluetooth/hci0 ] && break
	i=$((i + 1))
	sleep 0.5
done
# Controller never appeared: nothing to provision. Do NOT fall
# through to mgmt calls here - with no controller at index 0
# btmgmt blocks indefinitely and stalls the whole boot (hit on
# the first boot of the 1e538414 build, 2026-11-06).
[ -e /sys/class/bluetooth/hci0 ] || { log "no controller, nothing to do"; exit 0; }

# --- provision, patiently --------------------------------------
found=0
round=0
while [ $round -lt 40 ]; do
	if timeout 5 btmgmt info 2>/dev/null | grep -q "Primary controller"; then
		log "controller already configured"
		exit 0
	fi
	if timeout 5 btmgmt extinfo 2>/dev/null | grep -q "Unconfigured controller"; then
		found=1
		break
	fi
	round=$((round + 1))
	sleep 2
done
if [ "$found" -eq 1 ]; then
	try=0
	while [ $try -lt 4 ]; do
		timeout 15 btmgmt --index 0 public-addr "$addr" 2>/dev/null
		sleep 3
		if timeout 5 btmgmt info 2>/dev/null | grep -q "Primary controller"; then
			log "provisioned $addr"
			exit 0
		fi
		try=$((try + 1))
	done
	log "write attempted but controller did not graduate"
	exit 0
fi
log "controller never appeared in the unconfigured list"
exit 0
