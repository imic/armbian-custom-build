#!/bin/bash
# Battery charge hysteresis for the OnePlus 8T server: stop charging at 80%, resume at 50%.
# Requires the SMB5-enabled boot image (pm8150b-charger present). No-ops gracefully without it.
CHG=/sys/class/power_supply/pm8150b-charger/charging_enabled
GAUGE=/sys/class/power_supply/bq27541-0/capacity
[ -w "$CHG" ] && [ -r "$GAUGE" ] || exit 0
cap=$(cat "$GAUGE"); en=$(cat "$CHG")
if [ "$cap" -ge 80 ] && [ "$en" = "1" ]; then echo 0 > "$CHG"; fi
if [ "$cap" -le 50 ] && [ "$en" = "0" ]; then echo 1 > "$CHG"; fi
exit 0
