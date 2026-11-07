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
# The address is derived from the machine-id (same derivation the
# retired bt-fixed-mac used), so it is stable per device and needs
# no stored state.
set -e

# Wait for the controller WITHOUT mgmt calls. The hci device shows
# up in sysfs the moment the controller registers, so poll that -
# it costs nothing. The previous shape ran two btmgmt calls per
# iteration; each call takes a second or more while the stack is
# unsettled, and on a cold boot the polling plus the final write
# overran the unit's 45 s start budget: the address WAS provisioned
# but systemd killed the unit at the cap and marked it failed
# (first boot of the 10d812bf build, 2026-11-07).
i=0
while [ $i -lt 60 ]; do
	[ -e /sys/class/bluetooth/hci0 ] && break
	i=$((i + 1))
	sleep 0.5
done

# Controller never appeared: nothing to provision. Do NOT fall
# through to the public-addr call here - with no controller at
# index 0 btmgmt blocks indefinitely and stalls the whole boot
# (hit on the first boot of the 1e538414 build, 2026-11-06).
[ -e /sys/class/bluetooth/hci0 ] || exit 0

# Registered. Already configured means nothing to do; spend mgmt
# calls only from here on, and only a bounded number.
if btmgmt info 2>/dev/null | grep -q "Primary controller"; then
	exit 0
fi
found=0
i=0
while [ $i -lt 10 ]; do
	if btmgmt extinfo 2>/dev/null | grep -q "Unconfigured controller"; then
		found=1
		break
	fi
	if btmgmt info 2>/dev/null | grep -q "Primary controller"; then
		exit 0
	fi
	i=$((i + 1))
	sleep 1
done
if [ "$found" -eq 0 ]; then
	exit 0
fi

mac="$( (cat /etc/machine-id; echo bluetooth) | sha256sum )"
addr="42:$(echo "$mac" | cut -c1-2):$(echo "$mac" | cut -c5-6):$(echo "$mac" | cut -c9-10):$(echo "$mac" | cut -c13-14):$(echo "$mac" | cut -c17-18)"
exec timeout 15 btmgmt --index 0 public-addr "$addr"
