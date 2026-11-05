#!/usr/bin/env python3
"""Volume keys -> panel brightness for the OnePlus 8T (kebab).

Vol+ arrives on the 'gpio-keys' input device, Vol- on 'pm8941_resin'.
Devices are resolved by name from /proc/bus/input/devices so event
numbers may shift across boots. Pure stdlib: reads struct input_event
directly. Handles press (value=1) and autorepeat (value=2).
"""
import os
import select
import struct
import sys
import time

BACKLIGHT = "/sys/class/backlight/ae94000.dsi.0"
EV_KEY = 1
KEY_VOLUMEDOWN = 114
KEY_VOLUMEUP = 115
EVENT = struct.Struct("llHHi")


def find_event_node(name):
    try:
        with open("/proc/bus/input/devices") as f:
            blocks = f.read().split("\n\n")
    except OSError:
        return None
    for block in blocks:
        if 'Name="%s"' % name in block:
            for line in block.splitlines():
                if line.startswith("H: Handlers="):
                    for tok in line.split("=", 1)[1].split():
                        if tok.startswith("event"):
                            return "/dev/input/" + tok
    return None


def read_int(path):
    with open(path) as f:
        return int(f.read().strip())


def main():
    max_b = read_int(os.path.join(BACKLIGHT, "max_brightness"))
    step = max(1, max_b // 10)
    floor = 20  # never let the keys take the panel fully black

    nodes = []
    for name in ("gpio-keys", "pm8941_resin"):
        node = find_event_node(name)
        if node:
            nodes.append(node)
    if not nodes:
        print("volkey-brightness: no key devices found", flush=True)
        return 1

    fds = {}
    for node in nodes:
        try:
            fd = os.open(node, os.O_RDONLY)
            fds[fd] = node
        except OSError as e:
            print("volkey-brightness: cannot open %s: %s" % (node, e), flush=True)
    if not fds:
        return 1
    print("volkey-brightness: watching %s (max=%d step=%d)" %
          (sorted(fds.values()), max_b, step), flush=True)

    bri_path = os.path.join(BACKLIGHT, "brightness")
    while True:
        r, _, _ = select.select(list(fds), [], [], 60)
        for fd in r:
            data = os.read(fd, EVENT.size * 16)
            for off in range(0, len(data) - EVENT.size + 1, EVENT.size):
                _sec, _usec, etype, code, value = EVENT.unpack_from(data, off)
                if etype != EV_KEY or value not in (1, 2):
                    continue
                if code not in (KEY_VOLUMEUP, KEY_VOLUMEDOWN):
                    continue
                try:
                    cur = read_int(bri_path)
                    if code == KEY_VOLUMEUP:
                        new = min(max_b, cur + step)
                    else:
                        new = max(floor, cur - step)
                    if new != cur:
                        with open(bri_path, "w") as f:
                            f.write(str(new))
                except OSError as e:
                    print("volkey-brightness: %s" % e, flush=True)
                    time.sleep(1)


if __name__ == "__main__":
    sys.exit(main())
