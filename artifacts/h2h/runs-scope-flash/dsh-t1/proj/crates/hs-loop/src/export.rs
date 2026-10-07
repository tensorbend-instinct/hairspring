//! Export a session directory as a ZIP (dsh "Export"): stored entries, standard format.
use std::io::Write;
use std::path::Path;

fn crc32(data: &[u8]) -> u32 {
    let mut c = 0xFFFF_FFFFu32;
    for &b in data {
        c ^= u32::from(b);
        for _ in 0..8 {
            c = if c & 1 == 1 { (c >> 1) ^ 0xEDB8_8320 } else { c >> 1 };
        }
    }
    !c
}

fn walk(base: &Path, dir: &Path, out: &mut Vec<(String, std::path::PathBuf)>) -> std::io::Result<()> {
    for e in std::fs::read_dir(dir)? {
        let p = e?.path();
        if p.is_dir() {
            walk(base, &p, out)?;
        } else if let Ok(rel) = p.strip_prefix(base) {
            out.push((rel.to_string_lossy().replace('\\', "/"), p));
        }
    }
    Ok(())
}

/// Write every file under `src` into `out`; returns the file count.
pub fn export_zip(src: &Path, out: &Path) -> std::io::Result<usize> {
    let mut files = vec![];
    walk(src, src, &mut files)?;
    files.sort();
    let mut buf: Vec<u8> = vec![];
    let mut central: Vec<u8> = vec![];
    for (name, path) in &files {
        let data = std::fs::read(path)?;
        let crc = crc32(&data);
        let off = buf.len() as u32;
        let nb = name.as_bytes();
        let len = data.len() as u32;
        // local header, method 0 (stored)
        buf.extend_from_slice(&[0x50, 0x4b, 3, 4, 20, 0, 0, 8, 0, 0, 0, 0, 0x21, 0]);
        buf.extend_from_slice(&crc.to_le_bytes());
        buf.extend_from_slice(&len.to_le_bytes());
        buf.extend_from_slice(&len.to_le_bytes());
        buf.extend_from_slice(&(nb.len() as u16).to_le_bytes());
        buf.extend_from_slice(&[0, 0]);
        buf.extend_from_slice(nb);
        buf.extend_from_slice(&data);
        central.extend_from_slice(&[0x50, 0x4b, 1, 2, 20, 0, 20, 0, 0, 8, 0, 0, 0, 0, 0x21, 0]);
        central.extend_from_slice(&crc.to_le_bytes());
        central.extend_from_slice(&len.to_le_bytes());
        central.extend_from_slice(&len.to_le_bytes());
        central.extend_from_slice(&(nb.len() as u16).to_le_bytes());
        central.extend_from_slice(&[0; 12]);
        central.extend_from_slice(&off.to_le_bytes());
        central.extend_from_slice(nb);
    }
    let cd_off = buf.len() as u32;
    let cd_len = central.len() as u32;
    buf.extend_from_slice(&central);
    buf.extend_from_slice(&[0x50, 0x4b, 5, 6, 0, 0, 0, 0]);
    buf.extend_from_slice(&(files.len() as u16).to_le_bytes());
    buf.extend_from_slice(&(files.len() as u16).to_le_bytes());
    buf.extend_from_slice(&cd_len.to_le_bytes());
    buf.extend_from_slice(&cd_off.to_le_bytes());
    buf.extend_from_slice(&[0, 0]);
    std::fs::File::create(out)?.write_all(&buf)?;
    Ok(files.len())
}
