use super::{boxes::*, metadata::Metadata};

pub fn patch(
    data: &mut [u8],
    header: usize,
    old_offset: u64,
    new_offset: u64,
    mdats: &[BoxHeader],
    meta: &Metadata,
    new_id: u32,
    sequence: u32,
) -> Result<(), String> {
    let moof = &data[header..];
    let mfhd = only(moof, b"mfhd")?;
    let traf = only(moof, b"traf")?;
    if children(moof)?
        .iter()
        .any(|item| !matches!(&item.kind, b"mfhd" | b"traf"))
    {
        return Err("moof 包含不支持的关联 box".into());
    }
    let mfhd_abs = header + mfhd.offset + mfhd.header;
    if payload(moof, mfhd).len() != 8 || u32_at(data, mfhd_abs)? != 0 {
        return Err("mfhd 无效".into());
    }
    let traf_start = header + traf.offset + traf.header;
    let traf_end = header + traf.offset + traf.size;
    let children = children(&data[traf_start..traf_end])?;
    if children
        .iter()
        .any(|item| !matches!(&item.kind, b"tfhd" | b"tfdt" | b"trun"))
    {
        return Err("不支持含加密或额外采样引用的片段".into());
    }
    let tfhd = only(&data[traf_start..traf_end], b"tfhd")?;
    let decode_times: Vec<_> = children
        .iter()
        .filter(|item| item.kind == *b"tfdt")
        .collect();
    if decode_times.len() > 1 {
        return Err("traf 包含重复 tfdt".into());
    }
    for item in decode_times {
        let value =
            &data[traf_start + item.offset + item.header..traf_start + item.offset + item.size];
        if !matches!((u32_at(value, 0)?, value.len()), (0, 8) | (0x01000000, 12)) {
            return Err("tfdt 版本或长度无效".into());
        }
    }
    let tfhd_start = traf_start + tfhd.offset + tfhd.header;
    let flags = u32_at(data, tfhd_start)?;
    if flags & !0x02003b != 0 || u32_at(data, tfhd_start + 4)? != meta.id {
        return Err("tfhd 标志或 track_id 无效".into());
    }
    let mut cursor = tfhd_start + 8;
    let base = if flags & 1 != 0 {
        let value = u64_at(data, cursor)?;
        cursor += 8;
        value
    } else {
        old_offset
    };
    if flags & 2 != 0 {
        if u32_at(data, cursor)? != 1 {
            return Err("tfhd 采样描述索引无效".into());
        }
        cursor += 4;
    }
    if flags & 8 != 0 {
        cursor += 4;
    }
    let sample_size = if flags & 16 != 0 {
        let size = u32_at(data, cursor)?;
        cursor += 4;
        size
    } else {
        meta.default_size
    };
    if flags & 32 != 0 {
        cursor += 4;
    }
    if cursor != traf_start + tfhd.offset + tfhd.size {
        return Err("tfhd 字段长度与标志不符".into());
    }
    let mut previous = None;
    let mut runs = 0;
    for item in children.iter().filter(|item| item.kind == *b"trun") {
        let start = traf_start + item.offset + item.header;
        let end = traf_start + item.offset + item.size;
        let run = &data[start..end];
        let full = u32_at(run, 0)?;
        let version = full >> 24;
        let flags = full & 0x00ffffff;
        if version > 1 || flags & !0xf05 != 0 || (flags & 4 != 0 && flags & 0x400 != 0) {
            return Err("trun 版本或标志无效".into());
        }
        let count = u32_at(run, 4)?;
        if count == 0 {
            return Err("空 trun 无法生成有效媒体".into());
        }
        let mut at = 8;
        let position = if flags & 1 != 0 {
            let delta = u32_at(run, at)? as i32;
            at += 4;
            let value = i128::from(base) + i128::from(delta);
            u64::try_from(value).map_err(|_| "trun 数据偏移溢出")?
        } else {
            previous.unwrap_or(base)
        };
        if flags & 4 != 0 {
            at += 4;
        }
        let stride = ((flags & 0xf00).count_ones() as usize) * 4;
        let expected = at
            .checked_add(
                (count as usize)
                    .checked_mul(stride)
                    .ok_or("trun 采样数量溢出")?,
            )
            .ok_or("trun 长度溢出")?;
        if expected != run.len() {
            return Err("trun 采样字段长度不完整".into());
        }
        let mut size = 0u64;
        if flags & 0x200 == 0 {
            size = u64::from(sample_size)
                .checked_mul(u64::from(count))
                .ok_or("trun 数据长度溢出")?;
        } else {
            for _ in 0..count {
                if flags & 0x100 != 0 {
                    at += 4;
                }
                size = size
                    .checked_add(u64::from(u32_at(run, at)?))
                    .ok_or("trun 数据长度溢出")?;
                at += 4;
                if flags & 0x400 != 0 {
                    at += 4;
                }
                if flags & 0x800 != 0 {
                    at += 4;
                }
            }
        }
        let end = position.checked_add(size).ok_or("trun 数据范围溢出")?;
        if size == 0
            || !mdats.iter().any(|item| {
                position >= item.offset + item.header as u64 && end <= item.offset + item.size
            })
        {
            return Err("trun 引用超出已验证 mdat 媒体范围".into());
        }
        previous = Some(end);
        runs += 1;
    }
    if runs == 0 {
        return Err("moof 缺少有效 trun 采样".into());
    }
    if flags & 1 != 0 {
        let moved = i128::from(base) + i128::from(new_offset) - i128::from(old_offset);
        put_u64(
            data,
            tfhd_start + 8,
            u64::try_from(moved).map_err(|_| "tfhd 绝对偏移溢出")?,
        )?;
    }
    put_u32(data, tfhd_start + 4, new_id)?;
    put_u32(data, mfhd_abs + 4, sequence)
}
