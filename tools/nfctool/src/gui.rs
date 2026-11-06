// Minimal X11 GUI for nfctool: pure-Rust x11rb window, pixels pushed
// with put_image (banded), text via ab_glyph from a system TTF. No
// libX11, no GL — the binary stays a single static musl executable.
// v1 scope: Read Card (full Classic dump via the TAG-CMD engine),
// library of known cards keyed by UID (JSON in ~/.nfctool-cards.json),
// hex + ASCII grid with scroll. Card write-back does not exist here.
use crate::{asciis, hexs, CardDump, KEY_A, KEY_B};
use ab_glyph::{Font, FontArc, PxScale, ScaleFont};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    ChangeWindowAttributesAux, ClientMessageEvent, ConfigureNotifyEvent, ConnectionExt as _,
    CreateGCAux, CreateWindowAux, EventMask, ImageFormat, KeyPressEvent, ButtonPressEvent,
    PropMode, WindowClass, CLIENT_MESSAGE_EVENT,
};
use x11rb::wrapper::ConnectionExt as _;
use x11rb::CURRENT_TIME;

const BG: u32 = 0x0010_161B;
const PANEL: u32 = 0x0019_2129;
const HEADER: u32 = 0x001B_2530;
const TEXT: u32 = 0x00DD_E3E8;
const DIM: u32 = 0x008A_94A0;
const ACCENT: u32 = 0x004C_C38A;
const WARN: u32 = 0x00E5_C07B;
/// Width of the left column (library + card info panel).
const COL_W: i32 = 300;
const BTN: u32 = 0x0026_313C;
const BTN_HOV: u32 = 0x0032_4150;
const ROW_ALT: u32 = 0x0013_1A21;
const TRAILER: u32 = 0x0024_1F16;

fn blend(fg: u32, bg: u32, c: f32) -> u32 {
    let r = (((fg >> 16) & 0xFF) as f32 * c + ((bg >> 16) & 0xFF) as f32 * (1.0 - c)) as u32;
    let g = (((fg >> 8) & 0xFF) as f32 * c + ((bg >> 8) & 0xFF) as f32 * (1.0 - c)) as u32;
    let b = ((fg & 0xFF) as f32 * c + (bg & 0xFF) as f32 * (1.0 - c)) as u32;
    (r << 16) | (g << 8) | b
}

struct Canvas {
    w: usize,
    h: usize,
    px: Vec<u32>,
    font: FontArc,
}

impl Canvas {
    fn new(w: usize, h: usize, font: FontArc) -> Canvas {
        Canvas { w, h, px: vec![BG; w * h], font }
    }
    fn resize(&mut self, w: usize, h: usize) {
        self.w = w;
        self.h = h;
        self.px = vec![BG; w * h];
    }
    fn fill(&mut self, color: u32) {
        for p in self.px.iter_mut() {
            *p = color;
        }
    }
    fn rect(&mut self, x: i32, y: i32, w: i32, h: i32, color: u32) {
        let x0 = x.max(0) as usize;
        let y0 = y.max(0) as usize;
        let x1 = (x + w).min(self.w as i32).max(0) as usize;
        let y1 = (y + h).min(self.h as i32).max(0) as usize;
        for yy in y0..y1 {
            for xx in x0..x1 {
                self.px[yy * self.w + xx] = color;
            }
        }
    }
    fn text(&mut self, x: i32, y_top: i32, size: f32, color: u32, s: &str) {
        let scale = PxScale::from(size);
        let scaled = self.font.as_scaled(scale);
        let baseline = y_top as f32 + scaled.ascent();
        let mut cx = x as f32;
        for ch in s.chars() {
            let mut glyph = scaled.glyph_id(ch).with_scale(scale);
            glyph.position = ab_glyph::point(cx, baseline);
            let adv = scaled.h_advance(glyph.id);
            if let Some(og) = self.font.outline_glyph(glyph) {
                let bb = og.px_bounds();
                og.draw(|gx, gy, cov| {
                    let px = bb.min.x as i32 + gx as i32;
                    let py = bb.min.y as i32 + gy as i32;
                    if px >= 0 && py >= 0 && (px as usize) < self.w && (py as usize) < self.h {
                        let i = py as usize * self.w + px as usize;
                        self.px[i] = blend(color, self.px[i], cov);
                    }
                });
            }
            cx += adv;
        }
    }
    fn text_w(&self, size: f32, s: &str) -> f32 {
        let scaled = self.font.as_scaled(PxScale::from(size));
        s.chars().map(|c| scaled.h_advance(scaled.glyph_id(c))).sum()
    }
}

fn load_font() -> FontArc {
    let candidates = [
        "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf",
        "/usr/share/fonts/truetype/noto/NotoSansMono-Regular.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
    ];
    for p in candidates {
        if let Ok(bytes) = std::fs::read(p) {
            if let Ok(f) = FontArc::try_from_vec(bytes) {
                return f;
            }
        }
    }
    panic!("no usable system font found");
}

#[derive(Clone, Serialize, Deserialize)]
struct LibCard {
    uid_hex: String,
    note: String,
    saved: u64,
    dump: CardDump,
    /// True when this library copy was edited in the GUI (i.e. it may
    /// differ from the physical card). Defaults false for old files.
    #[serde(default)]
    edited: bool,
}

fn lib_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    PathBuf::from(home).join(".nfctool-cards.json")
}

fn lib_load() -> Vec<LibCard> {
    std::fs::read_to_string(lib_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn lib_save(cards: &[LibCard]) {
    if let Ok(s) = serde_json::to_string_pretty(cards) {
        let p = lib_path();
        let tmp = p.with_extension("json.tmp");
        if std::fs::write(&tmp, s).is_ok() {
            let _ = std::fs::rename(&tmp, &p);
        }
    }
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Run `sudo <exe> dump-json` with a live cancel path and a hard
/// 60 s deadline. The helper runs as root, so this unprivileged
/// process cannot signal it directly; cancellation goes through a
/// second sudo: `pkill -f "nfctool dump-json"` — that pattern
/// matches the helper and its sudo parent, never this GUI, whose
/// argv is `nfctool gui`. Without this, a read that cannot find a
/// card used to grind through discovery retries for minutes with
/// no way out (reported from the field 2026-11-04).
fn run_helper(
    exe: &std::path::Path,
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<CardDump, String> {
    use std::io::Read as _;
    let mut child = match std::process::Command::new("sudo")
        .arg(exe)
        .arg("dump-json")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        Ok(c) => c,
        Err(_) => return Err("could not start the read helper".to_string()),
    };
    let pipe = child.stdout.take();
    let (otx, orx) = channel::<Vec<u8>>();
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(mut p) = pipe {
            let _ = p.read_to_end(&mut buf);
        }
        let _ = otx.send(buf);
    });
    let t0 = std::time::Instant::now();
    loop {
        if cancel.load(std::sync::atomic::Ordering::SeqCst) {
            let _ = std::process::Command::new("sudo")
                .args(["pkill", "-f", "nfctool dump-json"])
                .status();
            let _ = child.kill();
            return Err("read cancelled".to_string());
        }
        if t0.elapsed() > std::time::Duration::from_secs(60) {
            let _ = std::process::Command::new("sudo")
                .args(["pkill", "-f", "nfctool dump-json"])
                .status();
            let _ = child.kill();
            return Err("read timed out after 60 s (card not answering?)".to_string());
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                let bytes = orx
                    .recv_timeout(std::time::Duration::from_secs(2))
                    .unwrap_or_default();
                if status.success() {
                    if let Ok(d) = serde_json::from_slice::<CardDump>(&bytes) {
                        return Ok(d);
                    }
                }
                return Err("no card seen".to_string());
            }
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(100)),
            Err(_) => return Err("read failed".to_string()),
        }
    }
}

/// In-progress hex edit of one block of the in-memory dump. Edited
/// bytes go to the library copy only — this program never writes a
/// card (there is no card-write code path at all).
struct EditState {
    sector: usize,
    block: usize, // 0..3 within the sector
    bytes: [u8; 16],
    caret: usize, // nibble index 0..32
}

struct App {
    status: String,
    reading: bool,
    cancel: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    read_started: Option<std::time::Instant>,
    last_tick: std::time::Instant,
    rx: Option<Receiver<Result<CardDump, String>>>,
    current: Option<CardDump>,
    library: Vec<LibCard>,
    scroll: i32,
    mouse: (i32, i32),
    click: Option<(i32, i32)>,
    view: u8, // 0 = hex grid, 1 = decode
    last_toggle: std::time::Instant,
    edit: Option<EditState>,
    keys: std::collections::HashMap<u8, (u32, u32)>, // keycode -> (plain, shifted) keysym
}

impl App {
    fn cancel_read(&mut self) {
        if let Some(c) = &self.cancel {
            c.store(true, std::sync::atomic::Ordering::SeqCst);
            self.status = "cancelling...".into();
        }
    }

    fn start_read(&mut self) {
        if self.reading {
            return;
        }
        self.edit = None; // a fresh read replaces the working copy
        self.reading = true;
        self.status = "reading... keep the card on the reader".into();
        let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        self.cancel = Some(cancel.clone());
        self.read_started = Some(std::time::Instant::now());
        self.last_tick = std::time::Instant::now();
        let (tx, rx) = channel();
        self.rx = Some(rx);
        std::thread::spawn(move || {
            // Cross-instance lock: two GUI instances reading at once
            // interleave DEV_UP/DOWN and both fail (seen in the field
            // 2026-11-04 when a test instance raced the user's one).
            let mut lock = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open("/tmp/nfctool-read.lock");
            if lock.is_err() {
                // A previous instance closed mid-read leaves a stale
                // lock behind (seen 2026-11-04 evening). If no helper
                // is actually running, the lock is stale: take it over.
                let helper_alive = std::process::Command::new("pgrep")
                    .args(["-f", "dump-json"])
                    .output()
                    .map(|o| o.status.success())
                    .unwrap_or(false);
                if !helper_alive {
                    let _ = std::fs::remove_file("/tmp/nfctool-read.lock");
                    lock = std::fs::OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .open("/tmp/nfctool-read.lock");
                }
            }
            if lock.is_err() {
                // Say how long the other read has been running, so a
                // genuinely stuck read is distinguishable from a
                // fresh one (the lock carries its start timestamp).
                let started = std::fs::read_to_string("/tmp/nfctool-read.lock")
                    .ok()
                    .and_then(|s| s.trim().parse::<u64>().ok());
                let msg = match started {
                    Some(ts) => format!(
                        "another read is in progress (started {}s ago)",
                        now_secs().saturating_sub(ts)
                    ),
                    None => "another read is already in progress".to_string(),
                };
                let _ = tx.send(Err(msg));
                return;
            }
            if let Ok(f) = &lock {
                use std::io::Write as _;
                let mut fr: &std::fs::File = f;
                let _ = fr.write_all(now_secs().to_string().as_bytes());
            }
            // The adapter has a single owner, so a read must first
            // displace neard + nfc-watch — but restart afterwards
            // only what was actually running. Since 2026-11-06 both
            // are retired by default (the adapter sits unowned and
            // reads take it directly); resurrecting them here would
            // silently undo that retirement on every read.
            let was_active: Vec<&str> = ["neard", "nfc-watch"]
                .into_iter()
                .filter(|u| {
                    std::process::Command::new("systemctl")
                        .args(["is-active", "--quiet", u])
                        .status()
                        .map(|s| s.success())
                        .unwrap_or(false)
                })
                .collect();
            let _ = std::process::Command::new("sudo")
                .args(["systemctl", "stop", "neard", "nfc-watch"])
                .status();
            std::thread::sleep(std::time::Duration::from_millis(1000));
            // The read itself must run as root: NFC DEV_UP is a
            // GENL_ADMIN_PERM op and fails EPERM for the unprivileged
            // GUI process (the cause of the 2026-11-04 evening DEV_UP
            // failures — not a device wedge). Shell out to our own
            // privileged helper mode and parse its JSON.
            let exe = std::env::current_exe()
                .unwrap_or_else(|_| std::path::PathBuf::from("nfctool"));
            let out = run_helper(&exe, &cancel);
            if !was_active.is_empty() {
                let mut args: Vec<&str> = vec!["systemctl", "start"];
                args.extend(was_active.iter().copied());
                let _ = std::process::Command::new("sudo").args(&args).status();
            }
            let _ = std::fs::remove_file("/tmp/nfctool-read.lock");
            let _ = tx.send(out);
        });
    }

    fn poll_read(&mut self) -> bool {
        let mut done = false;
        if let Some(rx) = &self.rx {
            if let Ok(res) = rx.try_recv() {
                done = true;
                self.reading = false;
                self.cancel = None;
                self.read_started = None;
                match res {
                    Ok(d) => {
                        let uid_hex = hexs(&d.uid).replace(' ', "");
                        self.status = format!("read OK: UID {} (saved to library)", uid_hex);
                        if let Some(e) = self.library.iter_mut().find(|c| c.uid_hex == uid_hex) {
                            e.dump = d.clone();
                            e.saved = now_secs();
                            e.edited = false; // a fresh read is pristine again
                        } else {
                            self.library.push(LibCard {
                                uid_hex,
                                note: String::new(),
                                saved: now_secs(),
                                dump: d.clone(),
                                edited: false,
                            });
                        }
                        lib_save(&self.library);
                        self.current = Some(d);
                        self.scroll = 0;
                    }
                    Err(e) => {
                        self.status = format!("error: {}", e);
                    }
                }
            }
        }
        if done {
            self.rx = None;
        }
        if self.reading && self.last_tick.elapsed() >= std::time::Duration::from_secs(1) {
            // Live elapsed readout so a long read never looks frozen.
            self.last_tick = std::time::Instant::now();
            if let Some(t0) = self.read_started {
                self.status = format!(
                    "reading... {}s (Cancel Read stops it)",
                    t0.elapsed().as_secs()
                );
            }
            return true;
        }
        done
    }
}

/// Short permission summary for a data block, indexed by (C1,C2,C3).
fn data_perm(c: u8) -> &'static str {
    match c {
        0 => "r/w/inc/dec A|B",
        1 => "r A|B, dec A|B (value)",
        2 => "r A|B only",
        3 => "r/w B only",
        4 => "r A|B, w B",
        5 => "r B only",
        6 => "r A|B, w/inc B, dec A|B",
        _ => "no access",
    }
}

/// Permission summary for a sector trailer, indexed by (C1,C2,C3).
fn trailer_perm(c: u8) -> &'static str {
    match c {
        0 => "KA wr:A | AC rd:A | KB wr:A",
        1 => "KA wr:A | AC rd/wr:A | KB wr:A",
        2 => "KA - | AC rd:A | KB -",
        3 => "KA wr:B | AC rd:A|B wr:B | KB wr:B",
        4 => "KA wr:B | AC rd:A|B | KB wr:B",
        5 => "KA - | AC rd:A|B wr:B | KB -",
        6 => "KA - | AC rd:A|B | KB -",
        _ => "KA - | AC rd:A|B | KB -",
    }
}

/// Human-readable interpretation of a dump: block-0 checks, per-sector
/// access-bit decoding (NXP AN10927 layout), value blocks, printable
/// strings, and a light NDEF scan. Pure presentation over the in-memory
/// dump — never touches the card.
/// Compact card identification + protocol summary for the side
/// panel: card type by SAK, anti-collision data, the RF interface
/// this tool reads through, key/access census and a text scan. The
/// Decode view remains the full-detail rendering; this is the
/// at-a-glance version that stays visible next to the hex grid.
fn card_info_lines(d: &CardDump) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let ty = match d.sak {
        0x08 => "MIFARE Classic 1K",
        0x18 => "MIFARE Classic 4K",
        0x09 => "MIFARE Mini",
        0x88 => "Classic-like (SAK 88)",
        0x00 => "Ultralight class",
        0x20 => "ISO 14443-4 card",
        _ => "unknown (by SAK)",
    };
    out.push(format!("Type  {}", ty));
    out.push(format!("UID   {} ({}B)", hexs(&d.uid), d.uid.len()));
    out.push(format!("ATQA  0x{:04X}  SAK 0x{:02X}", d.atqa, d.sak));
    out.push("RF    ISO/IEC 14443-A".to_string());
    out.push("Read  TAG-CMD iface 0x80".to_string());
    out.push(format!(
        "Size  {} sec x 4 blk = {} B",
        d.sectors.len(),
        d.sectors.len() * 64
    ));
    // Block 0: BCC is the XOR of the 4 UID bytes.
    if let Some(b0) = d.sectors.first().and_then(|s| s.blocks.first()).and_then(|b| b.as_ref()) {
        if b0.len() >= 5 && d.uid.len() >= 4 {
            let calc = d.uid[0] ^ d.uid[1] ^ d.uid[2] ^ d.uid[3];
            out.push(format!(
                "BCC   stored {:02X} calc {:02X} {}",
                b0[4],
                calc,
                if b0[4] == calc { "OK" } else { "MISMATCH" }
            ));
        }
    }
    // Key census per selector.
    for (sel, name) in [(KEY_A, "KeyA"), (KEY_B, "KeyB")] {
        let hits: Vec<&crate::SectorDump> = d
            .sectors
            .iter()
            .filter(|s| s.key_sel == Some(sel))
            .collect();
        if !hits.is_empty() {
            let key = hits[0].key.map(|k| hexs(&k)).unwrap_or_default();
            out.push(format!("{}  {} x{}", name, key, hits.len()));
        }
    }
    if d.sectors.iter().all(|s| s.key_sel.is_none()) {
        out.push("Keys  none opened".to_string());
    }
    // Access-bytes census from readable trailers.
    let mut pats: Vec<([u8; 3], usize)> = Vec::new();
    for sd in &d.sectors {
        if let Some(Some(t)) = sd.blocks.get(3) {
            let p = [t[6], t[7], t[8]];
            if let Some(e) = pats.iter_mut().find(|e| e.0 == p) {
                e.1 += 1;
            } else {
                pats.push((p, 1));
            }
        }
    }
    if let Some((p, n)) = pats.iter().max_by_key(|e| e.1) {
        out.push(format!("Acc   {} x{}", hexs(p), n));
    }
    // Non-zero data blocks (excluding block 0 and trailers).
    let mut nz = 0usize;
    let mut tot = 0usize;
    for (i, sd) in d.sectors.iter().enumerate() {
        for (b, blk) in sd.blocks.iter().enumerate() {
            if b == 3 || (i == 0 && b == 0) {
                continue;
            }
            if let Some(dd) = blk {
                tot += 1;
                if dd.iter().any(|&x| x != 0) {
                    nz += 1;
                }
            }
        }
    }
    out.push(format!("Data  {}/{} blocks non-zero", nz, tot));
    // First printable strings (>= 4 chars) in data blocks.
    let mut shown = 0;
    'scan: for (i, sd) in d.sectors.iter().enumerate() {
        for (b, blk) in sd.blocks.iter().enumerate() {
            if b == 3 || (i == 0 && b == 0) {
                continue; // trailers, and block 0 (manufacturer bytes are not text)
            }
            if let Some(dd) = blk {
                let mut run: Vec<u8> = Vec::new();
                for &c in dd.iter().chain(std::iter::once(&0u8)) {
                    if (0x20..0x7f).contains(&c) {
                        run.push(c);
                    } else {
                        if run.len() >= 4 {
                            let s: String = run.iter().map(|&c| c as char).collect();
                            let s: String = s.chars().take(18).collect();
                            out.push(format!("Text  \"{}\" @B{}", s, i * 4 + b));
                            shown += 1;
                            if shown >= 2 {
                                break 'scan;
                            }
                        }
                        run.clear();
                    }
                }
            }
        }
    }
    out
}

fn decode_lines(d: &CardDump) -> Vec<(String, u8)> {
    let mut out: Vec<(String, u8)> = Vec::new();
    let sak_name = match d.sak {
        0x08 => "MIFARE Classic 1K",
        0x88 => "MIFARE Classic 1K (Infineon)",
        0x18 => "MIFARE Classic 4K",
        0x09 => "MIFARE Classic Mini",
        _ => "unknown (not a standard Classic SAK)",
    };
    out.push(("== Card ==".to_string(), 0));
    out.push((format!("Type by SAK 0x{:02X}: {}", d.sak, sak_name), 1));
    out.push((
        format!(
            "Sectors read: {} ({} bytes incl. trailers)",
            d.sectors.len(),
            d.sectors.len() * 64
        ),
        1,
    ));

    out.push(("== Block 0 (manufacturer) ==".to_string(), 0));
    if let Some(b0) = d
        .sectors
        .first()
        .and_then(|s| s.blocks.first())
        .and_then(|b| b.as_ref())
    {
        if b0.len() == 16 && d.uid.len() == 4 {
            let bcc = d.uid[0] ^ d.uid[1] ^ d.uid[2] ^ d.uid[3];
            out.push((
                format!(
                    "BCC stored 0x{:02X}, computed 0x{:02X} -> {}",
                    b0[4],
                    bcc,
                    if b0[4] == bcc { "OK" } else { "MISMATCH" }
                ),
                1,
            ));
            out.push((
                format!(
                    "SAK copy 0x{:02X}, ATQA copy 0x{:02X} 0x{:02X}, manufacturer: {}",
                    b0[5],
                    b0[6],
                    b0[7],
                    hexs(&b0[8..16])
                ),
                1,
            ));
        }
    }

    out.push(("== Sector access bits ==".to_string(), 0));
    for (i, sd) in d.sectors.iter().enumerate() {
        let t = match sd.blocks.get(3).and_then(|b| b.as_ref()) {
            Some(t) if t.len() == 16 => t,
            _ => {
                out.push((format!("Sector {:>2}: trailer unreadable", i), 1));
                continue;
            }
        };
        let (b6, b7, b8) = (t[6], t[7], t[8]);
        let mut cs = [0u8; 4];
        let mut valid = true;
        for blk in 0..4usize {
            let c1 = (b7 >> (4 + blk)) & 1;
            let c2 = (b8 >> blk) & 1;
            let c3 = (b8 >> (4 + blk)) & 1;
            if (b6 >> blk) & 1 != 1 - c1 || (b6 >> (4 + blk)) & 1 != 1 - c2 || (b7 >> blk) & 1 != 1 - c3 {
                valid = false;
            }
            cs[blk] = (c1 << 2) | (c2 << 1) | c3;
        }
        out.push((
            format!(
                "Sector {:>2}: bits {:02X} {:02X} {:02X} (complements {}), GPB 0x{:02X}",
                i,
                b6,
                b7,
                b8,
                if valid { "OK" } else { "BROKEN" },
                t[9]
            ),
            1,
        ));
        out.push((
            format!(
                "   data [0]{} [1]{} [2]{}",
                data_perm(cs[0]),
                data_perm(cs[1]),
                data_perm(cs[2])
            ),
            1,
        ));
        out.push((format!("   trailer: {}", trailer_perm(cs[3])), 1));
        let kb = &t[10..16];
        if kb.iter().all(|&x| x == 0) {
            out.push(("   Key B: not readable with the opening key".to_string(), 1));
        } else {
            out.push((format!("   Key B (as read): {}", hexs(kb)), 1));
        }
    }

    out.push(("== Value blocks ==".to_string(), 0));
    let mut nval = 0;
    for (i, sd) in d.sectors.iter().enumerate() {
        for (b, blk) in sd.blocks.iter().enumerate() {
            if b == 3 {
                continue;
            }
            if let Some(v) = blk {
                if v.len() == 16
                    && v[0..4] == v[8..12]
                    && (0..4).all(|k| v[4 + k] == !v[k])
                    && v[12] == v[14]
                    && v[13] == !v[12]
                    && v[15] == !v[12]
                {
                    let val = i32::from_le_bytes([v[0], v[1], v[2], v[3]]);
                    out.push((format!("Block {:>2}: VALUE {} (addr {})", i * 4 + b, val, v[12]), 1));
                    nval += 1;
                }
            }
        }
    }
    if nval == 0 {
        out.push(("none found".to_string(), 1));
    }

    // Flattened data area (data blocks only) for string/NDEF scans.
    let mut area: Vec<u8> = Vec::new();
    for sd in d.sectors.iter() {
        for (b, blk) in sd.blocks.iter().enumerate() {
            if b == 3 {
                continue;
            }
            if let Some(v) = blk {
                area.extend_from_slice(v);
            }
        }
    }

    out.push(("== Printable strings (>= 6) ==".to_string(), 0));
    let mut nstr = 0;
    let mut run: Vec<u8> = Vec::new();
    let mut flush = |run: &Vec<u8>, out: &mut Vec<(String, u8)>, nstr: &mut i32| {
        if run.len() >= 6 && *nstr < 10 {
            out.push((format!("\"{}\"", String::from_utf8_lossy(run)), 1));
            *nstr += 1;
        }
    };
    for &c in area.iter() {
        if (0x20..0x7F).contains(&c) {
            run.push(c);
        } else {
            flush(&run, &mut out, &mut nstr);
            run.clear();
        }
    }
    flush(&run, &mut out, &mut nstr);
    if nstr == 0 {
        out.push(("none found".to_string(), 1));
    }

    out.push(("== NDEF ==".to_string(), 0));
    match scan_ndef(&area) {
        Some(lines) => out.extend(lines.into_iter().map(|s| (s, 1))),
        None => out.push(("no NDEF message found (raw or blank sectors)".to_string(), 1)),
    }
    out
}

/// Very light NDEF record scan over the data area: finds the first
/// plausible short record with a well-known type and decodes Text/URI.
fn scan_ndef(area: &[u8]) -> Option<Vec<String>> {
    let prefixes = [
        "", "http://www.", "https://www.", "http://", "https://", "tel:", "mailto:",
    ];
    for i in 0..area.len().saturating_sub(4) {
        let hdr = area[i];
        if hdr & 0x80 == 0 || hdr & 0x10 == 0 || hdr & 0x20 != 0 {
            continue; // need MB=1, SR=1, CF=0
        }
        let tnf = hdr & 0x07;
        if tnf != 1 {
            continue; // well-known type only
        }
        let tl = area[i + 1] as usize;
        if tl == 0 || tl > 4 || i + 3 + tl >= area.len() {
            continue;
        }
        let ty = &area[i + 3..i + 3 + tl];
        if !ty.iter().all(|&c| (0x20..0x7F).contains(&c)) {
            continue;
        }
        let plen = area[i + 2] as usize;
        let p0 = i + 3 + tl;
        if plen == 0 || p0 + plen > area.len() {
            continue;
        }
        let payload = &area[p0..p0 + plen];
        let tystr = String::from_utf8_lossy(ty).to_string();
        let mut lines = vec![format!("record at area offset {}: type \"{}\"", i, tystr)];
        match tystr.as_str() {
            "T" => {
                let ll = (payload[0] & 0x3F) as usize;
                if payload.len() > 1 + ll {
                    lines.push(format!(
                        "Text: \"{}\"",
                        String::from_utf8_lossy(&payload[1 + ll..])
                    ));
                }
            }
            "U" => {
                let pre = prefixes.get(payload[0] as usize).unwrap_or(&"");
                lines.push(format!(
                    "URI: {}{}",
                    pre,
                    String::from_utf8_lossy(&payload[1..])
                ));
            }
            _ => {
                lines.push(format!("payload: {}", hexs(&payload[..plen.min(24)])));
            }
        }
        return Some(lines);
    }
    None
}

/// Commit the in-progress block edit into the current dump and the
/// matching library entry (flagged `edited`). The card is untouched —
/// this program has no card-write path.
fn commit_edit(app: &mut App) {
    let Some(ed) = app.edit.take() else { return };
    let uid_hex = match &mut app.current {
        Some(cur) => {
            if let Some(sd) = cur.sectors.get_mut(ed.sector) {
                if ed.block < sd.blocks.len() {
                    sd.blocks[ed.block] = Some(ed.bytes);
                }
            }
            hexs(&cur.uid).replace(' ', "")
        }
        None => return,
    };
    if let Some(cur) = &app.current {
        let cur = cur.clone();
        if let Some(e) = app.library.iter_mut().find(|c| c.uid_hex == uid_hex) {
            e.dump = cur;
            e.edited = true;
            e.saved = now_secs();
            lib_save(&app.library);
        }
    }
    app.status = format!(
        "block {} updated in the library copy (card untouched)",
        ed.sector * 4 + ed.block
    );
}

/// Route one key press into the block editor (hex nibbles, caret
/// movement, Enter = commit, Esc = cancel).
fn handle_edit_key(app: &mut App, keycode: u8, state: u16) {
    let (plain, shifted) = app.keys.get(&keycode).copied().unwrap_or((0, 0));
    let sym = if state & 0x1 != 0 && shifted != 0 { shifted } else { plain };
    match sym {
        0xFF1B => {
            app.edit = None;
            app.status = "edit cancelled".into();
        }
        0xFF0D | 0xFF8D => commit_edit(app),
        0xFF08 | 0xFF51 => {
            if let Some(ed) = &mut app.edit {
                ed.caret = ed.caret.saturating_sub(1);
            }
        }
        0xFF53 => {
            if let Some(ed) = &mut app.edit {
                ed.caret = (ed.caret + 1).min(31);
            }
        }
        0xFF50 => {
            if let Some(ed) = &mut app.edit {
                ed.caret = 0;
            }
        }
        0xFF57 => {
            if let Some(ed) = &mut app.edit {
                ed.caret = 31;
            }
        }
        _ => {
            let nib = match sym {
                0x30..=0x39 => Some(sym - 0x30),
                0x61..=0x66 => Some(sym - 0x61 + 10),
                0x41..=0x46 => Some(sym - 0x41 + 10),
                _ => None,
            };
            if let (Some(n), Some(ed)) = (nib, &mut app.edit) {
                let byte = &mut ed.bytes[ed.caret / 2];
                if ed.caret % 2 == 0 {
                    *byte = ((n << 4) as u8) | (*byte & 0x0F);
                } else {
                    *byte = (*byte & 0xF0) | (n as u8);
                }
                ed.caret = (ed.caret + 1).min(31);
            }
        }
    }
}

fn draw(cv: &mut Canvas, app: &mut App) {
    let (w, h) = (cv.w as i32, cv.h as i32);
    cv.fill(BG);

    // Build grid rows from the current dump.
    let mut rows: Vec<(String, u8, Option<(usize, usize)>)> = Vec::new(); // kind: 0=head 1=block 2=trailer
    if app.view == 1 {
        if let Some(d) = &app.current {
            rows = decode_lines(d).into_iter().map(|(s, k)| (s, k, None)).collect();
        }
    } else if let Some(d) = &app.current {
        for (i, sd) in d.sectors.iter().enumerate() {
            let head = match (sd.key_sel, sd.key) {
                (Some(sel), Some(k)) => format!(
                    "Sector {:>2} - Key {} {}",
                    i,
                    if sel == KEY_A { "A" } else { "B" },
                    hexs(&k)
                ),
                _ => format!("Sector {:>2} - LOCKED", i),
            };
            rows.push((head, 0, None));
            for (b, blk) in sd.blocks.iter().enumerate() {
                let line = match blk {
                    Some(dd) => format!("  B{:>2}  {}  |{}|", i * 4 + b, hexs(dd), asciis(dd)),
                    None => format!("  B{:>2}  -- unreadable --", i * 4 + b),
                };
                rows.push((line, if b == 3 { 2 } else { 1 }, Some((i, b))));
            }
        }
    }
    let max_scroll = (rows.len() as i32 - 10).max(0);
    if app.scroll > max_scroll {
        app.scroll = max_scroll;
    }
    if app.scroll < 0 {
        app.scroll = 0;
    }

    // Grid.
    let gx = COL_W + 12;
    let gy = 92;
    let row_h = 20;
    let vis = ((h - 30 - gy) / row_h).max(0);
    let mut edit_click: Option<(usize, usize)> = None;
    for j in 0..vis {
        let ri = app.scroll + j;
        if ri as usize >= rows.len() {
            break;
        }
        let (line, kind, tgt) = &rows[ri as usize];
        let y = gy + j * row_h;
        if matches!(&app.edit, Some(ed) if tgt == &Some((ed.sector, ed.block))) {
            cv.rect(gx - 6, y - 2, w - gx - 4, row_h, TRAILER);
        }
        if let Some(cl) = app.click {
            if app.view == 0
                && !app.reading
                && cl.0 >= gx - 6
                && cl.1 >= y - 2
                && cl.1 < y - 2 + row_h
            {
                if let Some((s, b)) = tgt {
                    edit_click = Some((*s, *b));
                }
            }
        }
        match kind {
            0 => {
                cv.rect(gx - 6, y - 2, w - gx - 4, row_h, PANEL);
                cv.text(gx, y, 14.0, ACCENT, line);
            }
            2 => {
                cv.rect(gx - 6, y - 2, w - gx - 4, row_h, TRAILER);
                cv.text(gx, y, 14.0, TEXT, line);
            }
            _ => {
                if ri % 2 == 1 {
                    cv.rect(gx - 6, y - 2, w - gx - 4, row_h, ROW_ALT);
                }
                cv.text(gx, y, 14.0, TEXT, line);
            }
        }
    }

    // Apply a block-row click: start editing that block's copy.
    if let Some((s, b)) = edit_click {
        if s == 0 && b == 0 {
            app.status = "block 0 is the manufacturer block - not editable".into();
        } else {
            let data = app
                .current
                .as_ref()
                .and_then(|d| d.sectors.get(s))
                .and_then(|sd| sd.blocks.get(b))
                .and_then(|x| x.clone());
            if let Some(blk) = data {
                let mut bytes = [0u8; 16];
                let n = blk.len().min(16);
                bytes[..n].copy_from_slice(&blk[..n]);
                app.edit = Some(EditState { sector: s, block: b, bytes, caret: 0 });
                app.status = format!(
                    "editing block {} - type hex digits, Enter saves to the library copy, Esc cancels",
                    s * 4 + b
                );
            }
        }
    }

    // Info line.
    match &app.current {
        Some(d) => {
            let s = format!("UID: {}   ATQA: 0x{:04X}   SAK: 0x{:02X}", hexs(&d.uid), d.atqa, d.sak);
            cv.text(COL_W + 12, 60, 15.0, TEXT, &s);
        }
        None => {
            cv.text(COL_W + 12, 60, 15.0, DIM, "no card loaded - press Read Card, or pick one from the library");
        }
    }

    // Library panel.
    cv.rect(0, 52, COL_W, h - 52 - 26, PANEL);
    cv.text(12, 60, 15.0, TEXT, "Library");
    let mut lib_click: Option<usize> = None;
    for (i, c) in app.library.iter().enumerate() {
        let y = 88 + i as i32 * 26;
        if y > h - 60 {
            break;
        }
        let hov = app.mouse.0 < COL_W && app.mouse.1 >= y && app.mouse.1 < y + 24;
        if hov {
            cv.rect(6, y - 2, COL_W - 12, 24, BTN_HOV);
        }
        let label = if c.edited {
            format!("{} *", c.uid_hex)
        } else {
            c.uid_hex.clone()
        };
        cv.text(12, y, 13.0, TEXT, &label);
        if let Some(cl) = app.click {
            if cl.0 < COL_W && cl.1 >= y && cl.1 < y + 24 {
                lib_click = Some(i);
            }
        }
    }
    if let Some(i) = lib_click {
        let c = app.library[i].clone();
        app.status = format!("loaded {} from library", c.uid_hex);
        app.current = Some(c.dump);
        app.scroll = 0;
        app.edit = None;
    }

    // Card info / protocol details under the library list, for the
    // currently loaded card (read or picked from the library).
    if let Some(d) = &app.current {
        let iy = 88 + app.library.len() as i32 * 26 + 16;
        if iy + 30 < h - 26 {
            cv.text(12, iy, 15.0, TEXT, "Card Info");
            for (j, line) in card_info_lines(d).iter().enumerate() {
                let y = iy + 28 + j as i32 * 19;
                if y > h - 26 - 14 {
                    break;
                }
                cv.text(12, y, 13.0, DIM, line);
            }
        }
    }

    // Header.
    cv.rect(0, 0, w, 52, HEADER);
    cv.text(14, 8, 19.0, TEXT, "NFC Card Tool");
    cv.text(14, 32, 12.0, DIM, "MIFARE Classic - TAG-CMD reader (read-only)");

    // View toggle (Hex grid / Decode).
    let (vx, vy, vw2, vh2) = (w - 320, 10, 146, 32);
    let vhov = app.mouse.0 >= vx && app.mouse.0 < vx + vw2 && app.mouse.1 >= vy && app.mouse.1 < vy + vh2;
    cv.rect(vx, vy, vw2, vh2, if vhov { BTN_HOV } else { BTN });
    let vlabel = if app.view == 0 { "Decode" } else { "Hex View" };
    let vtw = cv.text_w(15.0, vlabel);
    cv.text(vx + (vw2 - vtw as i32) / 2, vy + 8, 15.0, TEXT, vlabel);
    if let Some(cl) = app.click {
        if cl.0 >= vx && cl.0 < vx + vw2 && cl.1 >= vy && cl.1 < vy + vh2 {
            // Debounce: xfwm4 re-sends a focus click, so one physical
            // click can arrive as two presses ~one frame apart and
            // would toggle twice (seen in remote testing 2026-11-04).
            if app.last_toggle.elapsed() > std::time::Duration::from_millis(400) {
                app.view = 1 - app.view;
                app.scroll = 0;
                app.last_toggle = std::time::Instant::now();
            }
        }
    }

    // Read button — while a read runs it becomes Cancel Read, so a
    // read that cannot find a card never traps the user.
    let (bx, by, bw2, bh2) = (w - 160, 10, 146, 32);
    let hov = app.mouse.0 >= bx && app.mouse.0 < bx + bw2 && app.mouse.1 >= by && app.mouse.1 < by + bh2;
    cv.rect(bx, by, bw2, bh2, if hov { BTN_HOV } else { BTN });
    let label = if app.reading { "Cancel Read" } else { "Read Card" };
    let tw = cv.text_w(15.0, label);
    cv.text(
        bx + (bw2 - tw as i32) / 2,
        by + 8,
        15.0,
        if app.reading { WARN } else { TEXT },
        label,
    );
    if let Some(cl) = app.click {
        if cl.0 >= bx && cl.0 < bx + bw2 && cl.1 >= by && cl.1 < by + bh2 {
            if app.reading {
                app.cancel_read();
            } else {
                app.start_read();
            }
        }
    }

    // Editor bar (visible while a block edit is in progress).
    if let Some(ed) = &app.edit {
        let by2 = h - 26 - 38;
        cv.rect(0, by2, w, 38, TRAILER);
        let label = format!(
            "EDIT B{} - library copy only, card is never written:  ",
            ed.sector * 4 + ed.block
        );
        let hexstr = ed
            .bytes
            .iter()
            .map(|b| format!("{:02X}", b))
            .collect::<Vec<_>>()
            .join(" ");
        cv.text(
            12,
            by2 + 10,
            14.0,
            TEXT,
            &format!("{}{}  |{}|", label, hexstr, asciis(&ed.bytes)),
        );
        let nib_char = (ed.caret / 2) * 3 + (ed.caret % 2);
        let pre: String = hexstr.chars().take(nib_char).collect();
        let cx = 12 + cv.text_w(14.0, &label) as i32 + cv.text_w(14.0, &pre) as i32;
        let cw = cv.text_w(14.0, "0") as i32;
        cv.rect(cx, by2 + 31, cw.max(4), 2, ACCENT);
    }

    // Footer status.
    cv.rect(0, h - 26, w, 26, HEADER);
    let st = app.status.clone();
    cv.text(10, h - 22, 13.0, if app.reading { WARN } else { DIM }, &st);
}

fn present(conn: &x11rb::rust_connection::RustConnection, win: u32, gc: u32, cv: &Canvas, depth: u8) {
    let band = 32usize;
    let mut y = 0usize;
    while y < cv.h {
        let rows = band.min(cv.h - y);
        let data: &[u8] = unsafe {
            std::slice::from_raw_parts(cv.px[y * cv.w..].as_ptr() as *const u8, rows * cv.w * 4)
        };
        let _ = conn.put_image(
            ImageFormat::Z_PIXMAP,
            win,
            gc,
            cv.w as u16,
            rows as u16,
            0,
            y as i16,
            0,
            depth,
            data,
        );
        y += rows;
    }
}

pub fn run_gui() {
    let font = load_font();
    let (conn, screen_num) = x11rb::connect(None).expect("X connect failed (DISPLAY set?)");
    let screen = &conn.setup().roots[screen_num];
    let win = conn.generate_id().expect("xid");
    let (w0, h0) = (1180u16, 780u16);
    conn.create_window(
        screen.root_depth,
        win,
        screen.root,
        0,
        0,
        w0,
        h0,
        0,
        WindowClass::INPUT_OUTPUT,
        screen.root_visual,
        &CreateWindowAux::new()
            .background_pixel(BG)
            .event_mask(
                EventMask::EXPOSURE
                    | EventMask::BUTTON_PRESS
                    | EventMask::KEY_PRESS
                    | EventMask::POINTER_MOTION
                    | EventMask::STRUCTURE_NOTIFY,
            ),
    )
    .expect("create_window");
    let wm_protocols = conn
        .intern_atom(false, b"WM_PROTOCOLS")
        .expect("intern")
        .reply()
        .expect("intern reply")
        .atom;
    let wm_delete = conn
        .intern_atom(false, b"WM_DELETE_WINDOW")
        .expect("intern")
        .reply()
        .expect("intern reply")
        .atom;
    conn.change_property(
        PropMode::REPLACE,
        win,
        wm_protocols,
        x11rb::protocol::xproto::AtomEnum::ATOM,
        32,
        1,
        &wm_delete.to_ne_bytes(),
    )
    .expect("wm protocols");
    conn.change_property8(
        PropMode::REPLACE,
        win,
        x11rb::protocol::xproto::AtomEnum::WM_NAME,
        x11rb::protocol::xproto::AtomEnum::STRING,
        "NFC Card Tool".as_bytes(),
    )
    .expect("wm name");
    conn.map_window(win).expect("map");
    let gc = conn.generate_id().expect("gc xid");
    conn.create_gc(gc, win, &CreateGCAux::new()).expect("gc");
    conn.flush().expect("flush");
    let depth = screen.root_depth;

    let mut canvas = Canvas::new(w0 as usize, h0 as usize, font);
    // Keyboard map for the block editor: keycode -> (plain, shifted).
    let mut keys: std::collections::HashMap<u8, (u32, u32)> = std::collections::HashMap::new();
    {
        let setup = conn.setup();
        let first = setup.min_keycode;
        let count = setup.max_keycode - setup.min_keycode + 1;
        if let Ok(Ok(k)) = conn.get_keyboard_mapping(first, count).map(|c| c.reply()) {
            let per = k.keysyms_per_keycode as usize;
            if per > 0 {
                for i in 0..count as usize {
                    let base = i * per;
                    let plain = k.keysyms.get(base).copied().unwrap_or(0);
                    let shifted = k.keysyms.get(base + 1).copied().unwrap_or(plain);
                    keys.insert(first + i as u8, (plain, shifted));
                }
            }
        }
    }

    let mut app = App {
        status: "ready - put a card on the reader, press Read Card".into(),
        reading: false,
        cancel: None,
        read_started: None,
        last_tick: std::time::Instant::now(),
        rx: None,
        current: None,
        library: lib_load(),
        scroll: 0,
        mouse: (0, 0),
        click: None,
        view: 0,
        last_toggle: std::time::Instant::now() - std::time::Duration::from_secs(1),
        edit: None,
        keys,
    };
    let mut dirty = true;
    loop {
        while let Ok(Some(ev)) = conn.poll_for_event() {
            match ev {
                x11rb::protocol::Event::ButtonPress(e) => {
                    app.mouse = (e.event_x as i32, e.event_y as i32);
                    match e.detail {
                        1 => app.click = Some(app.mouse),
                        4 => app.scroll -= 3,
                        5 => app.scroll += 3,
                        _ => {}
                    }
                    dirty = true;
                }
                x11rb::protocol::Event::MotionNotify(e) => {
                    app.mouse = (e.event_x as i32, e.event_y as i32);
                    dirty = true;
                }
                x11rb::protocol::Event::KeyPress(e) => {
                    if app.edit.is_some() {
                        handle_edit_key(&mut app, e.detail, u16::from(e.state));
                        dirty = true;
                    } else if e.detail == 9 {
                        return; // Esc
                    }
                }
                x11rb::protocol::Event::ConfigureNotify(e) => {
                    if e.width as usize != canvas.w || e.height as usize != canvas.h {
                        canvas.resize(e.width as usize, e.height as usize);
                        dirty = true;
                    }
                }
                x11rb::protocol::Event::ClientMessage(e) => {
                    if e.data.as_data32()[0] == wm_delete {
                        return;
                    }
                }
                x11rb::protocol::Event::Expose(_) => {
                    dirty = true;
                }
                _ => {}
            }
        }
        if app.poll_read() {
            dirty = true;
        }
        if dirty {
            // Clicks are consumed late in draw() (library load, edit
            // start), after the areas they affect were already painted;
            // schedule one follow-up frame so their effect is visible
            // immediately instead of waiting for the next input event.
            let had_click = app.click.is_some();
            draw(&mut canvas, &mut app);
            present(&conn, win, gc, &canvas, depth);
            let _ = conn.flush();
            app.click = None;
            dirty = had_click;
        }
        std::thread::sleep(std::time::Duration::from_millis(30));
    }
}
