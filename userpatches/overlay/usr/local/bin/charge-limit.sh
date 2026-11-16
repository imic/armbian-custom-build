#!/bin/bash
# Charge hysteresis via the MPS charger (bus5 0x5c). The Python limiter
# parks at vRest 8.12 V and resumes at vRest 7.85 V. This shim replaces
# the old pm8150b charging_enabled version. That path does not deliver
# on this board.
exec /usr/bin/python3 /usr/local/bin/mp2650-charge-limit.py
