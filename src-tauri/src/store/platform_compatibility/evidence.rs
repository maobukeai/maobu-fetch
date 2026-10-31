use crate::models::PlatformCompatibility;

pub(super) fn verified_download(item: &PlatformCompatibility) -> bool {
    let Some(evidence) = &item.verification else {
        return false;
    };
    evidence.app_version == env!("CARGO_PKG_VERSION")
        && evidence.scope == "download"
        && evidence.result == "passed"
        && !evidence.tool_version.trim().is_empty()
        && !evidence.sample.trim().is_empty()
        && valid_date(&item.last_tested_at)
}

fn valid_date(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() < 10 || bytes.get(4) != Some(&b'-') || bytes.get(7) != Some(&b'-') {
        return false;
    }
    let (Ok(year), Ok(month), Ok(day)) = (
        value.get(..4).unwrap_or_default().parse::<u32>(),
        value.get(5..7).unwrap_or_default().parse::<u32>(),
        value.get(8..10).unwrap_or_default().parse::<u32>(),
    ) else {
        return false;
    };
    if !(1970..=9999).contains(&year) || !(1..=12).contains(&month) {
        return false;
    }
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let max_day = match month {
        2 if leap => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    };
    if day == 0 || day > max_day {
        return false;
    }
    if value.len() != 10 && !valid_utc_time(value.get(10..).unwrap_or_default()) {
        return false;
    }
    // 不能把未来日期当成已完成的测试。
    let current = crate::format_unix_seconds(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
    );
    value.get(..10).unwrap_or_default() <= current.get(..10).unwrap_or_default()
}

fn valid_utc_time(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() != 10
        || bytes[0] != b'T'
        || bytes[3] != b':'
        || bytes[6] != b':'
        || bytes[9] != b'Z'
    {
        return false;
    }
    let (Ok(hour), Ok(minute), Ok(second)) = (
        value[1..3].parse::<u8>(),
        value[4..6].parse::<u8>(),
        value[7..9].parse::<u8>(),
    ) else {
        return false;
    };
    hour < 24 && minute < 60 && second < 60
}
