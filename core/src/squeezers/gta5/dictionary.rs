use super::DrawableKind;
use super::raw::{TEXTURE_RECORD_SIZE, graphics_offset, system_offset, u16_at, u32_at, u64_at};

pub(super) struct TexRecord {
    pub(super) record: usize,
    pub(super) name: String,
    pub(super) width: u32,
    pub(super) height: u32,
    pub(super) format: u32,
    pub(super) levels: u32,
    pub(super) data: usize,
}

pub(super) fn parse_dictionary(
    sys: &[u8],
    gfx: &[u8],
    dict_off: usize,
) -> Result<Vec<TexRecord>, String> {
    match try_layout(sys, gfx, dict_off + 0x30) {
        Ok(records) => Ok(records),
        Err(_) => {
            try_layout(sys, gfx, dict_off + 0x28).map_err(|e| format!("texture list at 0x28: {e}"))
        }
    }
}

fn try_layout(sys: &[u8], gfx: &[u8], list_off: usize) -> Result<Vec<TexRecord>, String> {
    let list_ptr = u64_at(sys, list_off).ok_or("truncated dictionary header")?;
    let count = u16_at(sys, list_off + 8).ok_or("truncated dictionary header")? as usize;
    if count == 0 {
        return Err("empty texture list".into());
    }
    let list = system_offset(list_ptr, sys.len())
        .ok_or_else(|| format!("texture list pointer {list_ptr:#x} not in system segment"))?;

    let mut records = Vec::with_capacity(count);
    for i in 0..count {
        let entry_ptr =
            u64_at(sys, list + i * 8).ok_or_else(|| format!("texture pointer {i} truncated"))?;
        let record = system_offset(entry_ptr, sys.len())
            .filter(|&o| o + TEXTURE_RECORD_SIZE <= sys.len())
            .ok_or_else(|| format!("texture record {i} pointer {entry_ptr:#x} invalid"))?;

        let name_ptr = u64_at(sys, record + 0x28).unwrap_or(0);
        let name_off = system_offset(name_ptr, sys.len())
            .ok_or_else(|| format!("texture {i} name pointer {name_ptr:#x} invalid"))?;
        let name_end = sys[name_off..]
            .iter()
            .position(|&b| b == 0)
            .map(|p| name_off + p)
            .ok_or_else(|| format!("texture {i} name unterminated"))?;
        let name = String::from_utf8_lossy(&sys[name_off..name_end]).into_owned();

        let width = u16_at(sys, record + 0x50).unwrap_or(0) as u32;
        let height = u16_at(sys, record + 0x52).unwrap_or(0) as u32;
        let format = u32_at(sys, record + 0x58).unwrap_or(0);
        let levels = sys.get(record + 0x5D).copied().unwrap_or(1) as u32;
        let data_ptr = u64_at(sys, record + 0x70).unwrap_or(0);
        let data = graphics_offset(data_ptr, gfx.len())
            .ok_or_else(|| format!("texture {i} data pointer {data_ptr:#x} invalid"))?;

        if !(1..=16384).contains(&width) || !(1..=16384).contains(&height) {
            return Err(format!(
                "texture {i} (`{name}`) has implausible size {width}x{height}"
            ));
        }

        records.push(TexRecord {
            record,
            name,
            width,
            height,
            format,
            levels,
            data,
        });
    }
    Ok(records)
}

pub(super) fn drawable_offsets(sys: &[u8], kind: DrawableKind) -> Vec<usize> {
    match kind {
        DrawableKind::Drawable => vec![0],
        DrawableKind::Dictionary => dictionary_entries(sys).collect(),
        DrawableKind::Fragment => u64_at(sys, 0x30)
            .and_then(|p| system_offset(p, sys.len()))
            .into_iter()
            .collect(),
    }
}

pub(super) fn find_embedded_dictionaries(sys: &[u8], kind: DrawableKind) -> Vec<usize> {
    let dict_of_drawable = |draw_off: usize| -> Option<usize> {
        let shader_group = system_offset(u64_at(sys, draw_off + 0x10)?, sys.len())?;
        system_offset(u64_at(sys, shader_group + 0x08)?, sys.len())
    };

    match kind {
        DrawableKind::Drawable => dict_of_drawable(0).into_iter().collect(),
        DrawableKind::Dictionary => dictionary_entries(sys)
            .filter_map(dict_of_drawable)
            .collect(),
        DrawableKind::Fragment => u64_at(sys, 0x30)
            .and_then(|p| system_offset(p, sys.len()))
            .and_then(dict_of_drawable)
            .into_iter()
            .collect(),
    }
}

fn dictionary_entries(sys: &[u8]) -> impl Iterator<Item = usize> + '_ {
    let list = u64_at(sys, 0x30).and_then(|p| system_offset(p, sys.len()));
    let count = match list {
        Some(_) => u16_at(sys, 0x38).unwrap_or(0) as usize,
        None => 0,
    };
    (0..count.min(1024)).filter_map(move |i| system_offset(u64_at(sys, list? + i * 8)?, sys.len()))
}

pub(super) fn looks_like_vehicle_dict(records: &[TexRecord]) -> bool {
    records
        .iter()
        .any(|r| r.name.len() >= 15 && r.name[..15].eq_ignore_ascii_case("vehicle_generic"))
}
