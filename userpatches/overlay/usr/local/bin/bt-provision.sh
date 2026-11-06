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
i=0
while [ $i -lt 30 ]; do
	if btmgmt info 2>/dev/null | grep -q "Primary controller"; then
		exit 0
	fi
	if btmgmt extinfo 2>/dev/null | grep -q "Unconfigured controller"; then
		break
	fi
	i=$((i + 1))
	sleep 1
done

mac="$( (cat /etc/machine-id; echo bluetooth) | sha256sum )"
addr="42:$(echo "$mac" | cut -c1-2):$(echo "$mac" | cut -c5-6):$(echo "$mac" | cut -c9-10):$(echo "$mac" | cut -c13-14):$(echo "$mac" | cut -c17-18)"
exec btmgmt --index 0 public-addr "$addr"
