#!/usr/bin/env python3
"""nfc-watch: resident watcher for neard on the OnePlus 8T.

Keeps the adapter powered and polling (neard ends its poll loop after
each discovery cycle, so this re-arms it), and appends one JSON line
per tag event to ~/nfc-log/events.jsonl:
  {"ts", "event": found|removed|watcher_start, "type", "protocol",
   "uid", "readonly", "records": [...], "status": ok|partial}

Must run as root (org.neard adapter control is root-only on this box).
Depends on the SN100 being powered (venpoke module / round-17 DTB).
"""
import json
import os
from datetime import datetime

import gi
gi.require_version('Gio', '2.0')
from gi.repository import Gio, GLib

LOGDIR = os.environ.get('NFC_LOG_DIR', os.path.expanduser('~/nfc-log'))
LOGFILE = os.path.join(LOGDIR, 'events.jsonl')
ADAPTER = '/org/neard/nfc0'
TAG_PREFIX = '/org/neard/nfc0/tag'

os.makedirs(LOGDIR, exist_ok=True)


def now_iso():
    return datetime.now().astimezone().isoformat(timespec='seconds')


def clean(v):
    if isinstance(v, (bytes, bytearray)):
        return v.hex()
    if isinstance(v, dict):
        return {k: clean(x) for k, x in v.items()}
    if isinstance(v, (list, tuple)):
        return [clean(x) for x in v]
    return v


def log_event(ev):
    ev = clean(ev)
    ev['ts'] = now_iso()
    with open(LOGFILE, 'a') as f:
        f.write(json.dumps(ev, ensure_ascii=False) + '\n')
    print(ev, flush=True)


bus = Gio.bus_get_sync(Gio.BusType.SYSTEM, None)


def call_sync(path, iface, method, params=None, rtype=None):
    return bus.call_sync('org.neard', path, iface, method, params,
                         rtype, Gio.DBusCallFlags.NONE, 5000, None)


def get_all(path, iface):
    v = call_sync(path, 'org.freedesktop.DBus.Properties', 'GetAll',
                  GLib.Variant('(s)', (iface,)),
                  GLib.VariantType.new('(a{sv})'))
    return v.unpack()[0]


def get_prop(path, iface, name):
    v = call_sync(path, 'org.freedesktop.DBus.Properties', 'Get',
                  GLib.Variant('(ss)', (iface, name)),
                  GLib.VariantType.new('(v)'))
    return v.unpack()[0]


def managed_objects():
    for root in ('/', '/org/neard'):
        try:
            v = call_sync(root, 'org.freedesktop.DBus.ObjectManager',
                          'GetManagedObjects', None,
                          GLib.VariantType.new('(a{oa{sa{sv}}})'))
            return v.unpack()[0]
        except Exception:
            continue
    return {}


seen = {}  # tag object path -> uid hex


def dump_tag(path):
    if path in seen:
        return
    try:
        props = get_all(path, 'org.neard.Tag')
    except Exception as e:
        log_event({'event': 'found', 'tag': path,
                   'status': 'partial', 'error': str(e)})
        seen[path] = ''
        return
    uid = ''.join(f'{b:02X}' for b in props.get('Uid', []))
    records = []
    for p, ifaces in managed_objects().items():
        if p.startswith(path + '/') and 'org.neard.Record' in ifaces:
            records.append(dict(ifaces['org.neard.Record']))
    seen[path] = uid
    log_event({'event': 'found', 'tag': path,
               'type': props.get('Type'),
               'protocol': props.get('Protocol'),
               'uid': uid,
               'readonly': props.get('ReadOnly'),
               'records': records,
               'status': 'ok'})


def on_interfaces_added(conn, sender, obj_path, iface, member, params):
    added_path, ifaces = params.unpack()
    if 'org.neard.Tag' in ifaces and added_path.startswith(TAG_PREFIX):
        GLib.timeout_add(200, lambda: (dump_tag(added_path), False)[1])


def on_interfaces_removed(conn, sender, obj_path, iface, member, params):
    removed_path, ifaces = params.unpack()
    if 'org.neard.Tag' in ifaces and removed_path in seen:
        log_event({'event': 'removed', 'tag': removed_path,
                   'uid': seen.pop(removed_path), 'status': 'ok'})


def rearm():
    try:
        if not get_prop(ADAPTER, 'org.neard.Adapter', 'Powered'):
            call_sync(ADAPTER, 'org.freedesktop.DBus.Properties', 'Set',
                      GLib.Variant('(ssv)', ('org.neard.Adapter', 'Powered',
                                             GLib.Variant('b', True))))
        if not seen and not get_prop(ADAPTER, 'org.neard.Adapter', 'Polling'):
            call_sync(ADAPTER, 'org.neard.Adapter', 'StartPollLoop',
                      GLib.Variant('(s)', ('Initiator',)))
    except Exception as e:
        print('rearm:', e, flush=True)
    return True


def main():
    bus.signal_subscribe('org.neard', 'org.freedesktop.DBus.ObjectManager',
                         'InterfacesAdded', None, None,
                         Gio.DBusSignalFlags.NONE, on_interfaces_added)
    bus.signal_subscribe('org.neard', 'org.freedesktop.DBus.ObjectManager',
                         'InterfacesRemoved', None, None,
                         Gio.DBusSignalFlags.NONE, on_interfaces_removed)
    log_event({'event': 'watcher_start', 'status': 'ok'})
    rearm()
    GLib.timeout_add_seconds(3, rearm)
    GLib.MainLoop().run()


if __name__ == '__main__':
    main()
