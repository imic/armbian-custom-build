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

# Wait for the controller to show up as unconfigured (or bail out
# quietly if it is already configured).
found=0
i=0
while [ $i -lt 30 ]; do
	if btmgmt info 2>/dev/null | grep -q "Primary controller"; then
		exit 0
	fi
	if btmgmt extinfo 2>/dev/null | grep -q "Unconfigured controller"; then
		found=1
		break
	fi
	i=$((i + 1))
	sleep 1
done

# Controller never appeared: nothing to provision. Do NOT fall
# through to the public-addr call here - with no controller at
# index 0 btmgmt blocks indefinitely and stalls the whole boot
# (hit on the first boot of the 1e538414 build, 2026-11-06).
if [ "$found" -eq 0 ]; then
	exit 0
fi

mac="$( (cat /etc/machine-id; echo bluetooth) | sha256sum )"
addr="42:$(echo "$mac" | cut -c1-2):$(echo "$mac" | cut -c5-6):$(echo "$mac" | cut -c9-10):$(echo "$mac" | cut -c13-14):$(echo "$mac" | cut -c17-18)"
exec btmgmt --index 0 public-addr "$addr"
