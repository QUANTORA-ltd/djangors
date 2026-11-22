use minijinja::value::Value;
use minijinja::{Error as MJError, ErrorKind as MJErrorKind};
use std::fmt::Write;

/// Format a date/time using a subset of Django's format specifiers.
///
/// Supported specifiers:
/// - `Y`: 4-digit year (e.g. 2026)
/// - `m`: 2-digit month (e.g. 07)
/// - `d`: 2-digit day of the month (e.g. 17)
/// - `H`: 2-digit hour, 24-hour format (e.g. 18)
/// - `i`: 2-digit minutes (e.g. 02)
/// - `s`: 2-digit seconds (e.g. 05)
///
/// Example:
/// `{{ value|date:"Y-m-d H:i:s" }}`
pub fn date(value: Value, format_str: Option<String>) -> Result<String, MJError> {
    let format_str = format_str.unwrap_or_else(|| "Y-m-d".to_string());

    let dt_str = value.to_string();
    let dt = if let Ok(parsed) = chrono::DateTime::parse_from_rfc3339(&dt_str) {
        parsed.with_timezone(&chrono::Utc)
    } else if let Ok(parsed) = chrono::NaiveDateTime::parse_from_str(&dt_str, "%Y-%m-%d %H:%M:%S") {
        chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(parsed, chrono::Utc)
    } else if let Ok(parsed) = chrono::NaiveDate::parse_from_str(&dt_str, "%Y-%m-%d") {
        let ndt = parsed.and_hms_opt(0, 0, 0).unwrap();
        chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(ndt, chrono::Utc)
    } else {
        return Err(MJError::new(
            MJErrorKind::InvalidOperation,
            format!("date filter: could not parse date/time value: {}", dt_str),
        ));
    };

    let mut result = String::new();
    for c in format_str.chars() {
        match c {
            'Y' => write!(&mut result, "{:04}", dt.format("%Y")).unwrap(),
            'm' => write!(&mut result, "{:02}", dt.format("%m")).unwrap(),
            'd' => write!(&mut result, "{:02}", dt.format("%d")).unwrap(),
            'H' => write!(&mut result, "{:02}", dt.format("%H")).unwrap(),
            'i' => write!(&mut result, "{:02}", dt.format("%M")).unwrap(),
            's' => write!(&mut result, "{:02}", dt.format("%S")).unwrap(),
            other => result.push(other),
        }
    }

    Ok(result)
}

/// Round a float to N decimal places, or Django's default (1 decimal, trailing zeros trimmed)
/// if no argument is given.
///
/// Example:
/// `{{ value|floatformat:2 }}`
pub fn floatformat(value: Value, arg: Option<Value>) -> Result<String, MJError> {
    let num = match value.as_str() {
        Some(s) => s.parse::<f64>().map_err(|e| {
            MJError::new(
                MJErrorKind::InvalidOperation,
                format!("floatformat filter: value must be a number: {}", e),
            )
        })?,
        None => {
            if let Ok(f) = f64::try_from(value.clone()) {
                f
            } else if let Ok(i) = i64::try_from(value.clone()) {
                i as f64
            } else {
                return Err(MJError::new(
                    MJErrorKind::InvalidOperation,
                    format!("floatformat filter: value must be a number: {}", value),
                ));
            }
        }
    };

    if let Some(arg_val) = arg {
        let places = i32::try_from(arg_val).map_err(|e| {
            MJError::new(
                MJErrorKind::InvalidOperation,
                format!("floatformat filter: argument must be an integer: {}", e),
            )
        })?;

        if places < 0 {
            let abs_places = places.unsigned_abs() as usize;
            let formatted = format!("{:.*}", abs_places, num);
            if formatted.contains('.') {
                let parts: Vec<&str> = formatted.split('.').collect();
                if parts.len() == 2 && parts[1].chars().all(|c| c == '0') {
                    Ok(format!("{:.0}", num))
                } else {
                    Ok(formatted)
                }
            } else {
                Ok(formatted)
            }
        } else {
            let formatted = format!("{:.*}", places as usize, num);
            Ok(formatted)
        }
    } else {
        let formatted = format!("{:.1}", num);
        if formatted.ends_with(".0") {
            Ok(formatted[..formatted.len() - 2].to_string())
        } else {
            Ok(formatted)
        }
    }
}

/// Pluralize a suffix based on a count.
///
/// Returns "" if count == 1, "s" otherwise (or a custom suffix if one is given as an argument).
/// If custom suffix contains a comma, the part before the comma is for singular, and the part
/// after is for plural (e.g. "y,ies" for dynamic -> dynamics, or cherry -> cherries).
///
/// Example:
/// `{{ count }} item{{ count|pluralize }}`
/// `{{ count }} cherr{{ count|pluralize:"y,ies" }}`
pub fn pluralize(value: Value, arg: Option<String>) -> Result<String, MJError> {
    let count = if let Ok(i) = i64::try_from(value.clone()) {
        i as f64
    } else if let Ok(f) = f64::try_from(value.clone()) {
        f
    } else {
        return Err(MJError::new(
            MJErrorKind::InvalidOperation,
            format!("pluralize filter: value must be a number: {}", value),
        ));
    };

    let is_singular = count == 1.0;

    let arg = arg.unwrap_or_else(|| "s".to_string());
    if let Some((singular_suffix, plural_suffix)) = arg.split_once(',') {
        if is_singular {
            Ok(singular_suffix.to_string())
        } else {
            Ok(plural_suffix.to_string())
        }
    } else {
        if is_singular {
            Ok("".to_string())
        } else {
            Ok(arg)
        }
    }
}

/// Truncate a string to N words, appending "…" if truncation actually happened.
///
/// Example:
/// `{{ value|truncatewords:30 }}`
pub fn truncatewords(value: Value, arg: Value) -> Result<String, MJError> {
    let s = value.to_string();
    let num_words = usize::try_from(arg).map_err(|e| {
        MJError::new(
            MJErrorKind::InvalidOperation,
            format!("truncatewords filter: argument must be an integer: {}", e),
        )
    })?;

    let words: Vec<&str> = s.split_whitespace().collect();
    if words.len() <= num_words {
        Ok(s)
    } else {
        let truncated = words[..num_words].join(" ");
        Ok(format!("{}…", truncated))
    }
}

/// Format an integer or numeric value with thousands separators (commas).
///
/// Example: `{{ 1234567|intcomma }}` -> `"1,234,567"`
pub fn intcomma(value: Value) -> Result<String, MJError> {
    let s = value.to_string();

    let (sign, rest) = if let Some(stripped) = s.strip_prefix('-') {
        ("-", stripped)
    } else if let Some(stripped) = s.strip_prefix('+') {
        ("+", stripped)
    } else {
        ("", s.as_str())
    };

    let (int_part, dec_part) = match rest.split_once('.') {
        Some((int_p, dec_p)) => (int_p, Some(dec_p)),
        None => (rest, None),
    };

    if int_part.is_empty() || !int_part.chars().all(|c| c.is_ascii_digit()) {
        return Ok(s);
    }

    let mut formatted_int = String::new();
    let len = int_part.len();
    for (i, c) in int_part.chars().enumerate() {
        if i > 0 && (len - i) % 3 == 0 {
            formatted_int.push(',');
        }
        formatted_int.push(c);
    }

    let mut result = format!("{}{}", sign, formatted_int);
    if let Some(dec) = dec_part {
        result.push('.');
        result.push_str(dec);
    }
    Ok(result)
}

/// Format a byte count into a human-readable size string.
///
/// Example: `{{ 1536|filesizeformat }}` -> `"1.5 KB"`
pub fn filesizeformat(value: Value) -> Result<String, MJError> {
    let bytes = match value.as_str() {
        Some(s) => s.parse::<f64>().map_err(|e| {
            MJError::new(
                MJErrorKind::InvalidOperation,
                format!("filesizeformat filter: value must be a number: {}", e),
            )
        })?,
        None => {
            if let Ok(f) = f64::try_from(value.clone()) {
                f
            } else if let Ok(i) = i64::try_from(value.clone()) {
                i as f64
            } else {
                return Err(MJError::new(
                    MJErrorKind::InvalidOperation,
                    format!("filesizeformat filter: value must be a number: {}", value),
                ));
            }
        }
    };

    if bytes < 0.0 {
        return Ok("0 bytes".to_string());
    }

    if bytes < 1024.0 {
        let b = bytes as i64;
        if b == 1 {
            Ok("1 byte".to_string())
        } else {
            Ok(format!("{} bytes", b))
        }
    } else {
        let units = ["KB", "MB", "GB", "TB", "PB"];
        let mut scaled = bytes / 1024.0;
        let mut unit_idx = 0;
        while scaled >= 1024.0 && unit_idx < units.len() - 1 {
            scaled /= 1024.0;
            unit_idx += 1;
        }
        Ok(format!("{:.1} {}", scaled, units[unit_idx]))
    }
}

/// Format a datetime as relative time ("3 minutes ago", "just now", "in 5 minutes").
///
/// Example: `{{ created_at|naturaltime }}`
pub fn naturaltime(value: Value) -> Result<String, MJError> {
    let dt_str = value.to_string();
    let dt = if let Ok(parsed) = chrono::DateTime::parse_from_rfc3339(&dt_str) {
        parsed.with_timezone(&chrono::Utc)
    } else if let Ok(parsed) = chrono::NaiveDateTime::parse_from_str(&dt_str, "%Y-%m-%d %H:%M:%S") {
        chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(parsed, chrono::Utc)
    } else if let Ok(parsed) = chrono::NaiveDate::parse_from_str(&dt_str, "%Y-%m-%d") {
        let ndt = parsed.and_hms_opt(0, 0, 0).unwrap();
        chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(ndt, chrono::Utc)
    } else {
        return Err(MJError::new(
            MJErrorKind::InvalidOperation,
            format!(
                "naturaltime filter: could not parse date/time value: {}",
                dt_str
            ),
        ));
    };

    let now = chrono::Utc::now();
    let seconds = (dt - now).num_seconds();

    if (-1..=1).contains(&seconds) {
        Ok("just now".to_string())
    } else if seconds < -1 {
        let ago = -seconds;
        if ago < 60 {
            Ok(format!("{} seconds ago", ago))
        } else if ago < 3600 {
            let mins = (ago + 30) / 60;
            if mins == 1 {
                Ok("1 minute ago".to_string())
            } else {
                Ok(format!("{} minutes ago", mins))
            }
        } else if ago < 86400 {
            let hours = (ago + 1800) / 3600;
            if hours == 1 {
                Ok("1 hour ago".to_string())
            } else {
                Ok(format!("{} hours ago", hours))
            }
        } else if ago < 2592000 {
            let days = (ago + 43200) / 86400;
            if days == 1 {
                Ok("1 day ago".to_string())
            } else {
                Ok(format!("{} days ago", days))
            }
        } else if ago < 31536000 {
            let months = (ago + 1296000) / 2592000;
            if months == 1 {
                Ok("1 month ago".to_string())
            } else {
                Ok(format!("{} months ago", months))
            }
        } else {
            let years = (ago + 15768000) / 31536000;
            if years == 1 {
                Ok("1 year ago".to_string())
            } else {
                Ok(format!("{} years ago", years))
            }
        }
    } else {
        let in_secs = seconds;
        if in_secs < 60 {
            Ok(format!("in {} seconds", in_secs))
        } else if in_secs < 3600 {
            let mins = (in_secs + 30) / 60;
            if mins == 1 {
                Ok("in 1 minute".to_string())
            } else {
                Ok(format!("in {} minutes", mins))
            }
        } else if in_secs < 86400 {
            let hours = (in_secs + 1800) / 3600;
            if hours == 1 {
                Ok("in 1 hour".to_string())
            } else {
                Ok(format!("in {} hours", hours))
            }
        } else if in_secs < 2592000 {
            let days = (in_secs + 43200) / 86400;
            if days == 1 {
                Ok("in 1 day".to_string())
            } else {
                Ok(format!("in {} days", days))
            }
        } else if in_secs < 31536000 {
            let months = (in_secs + 1296000) / 2592000;
            if months == 1 {
                Ok("in 1 month".to_string())
            } else {
                Ok(format!("in {} months", months))
            }
        } else {
            let years = (in_secs + 15768000) / 31536000;
            if years == 1 {
                Ok("in 1 year".to_string())
            } else {
                Ok(format!("in {} years", years))
            }
        }
    }
}

fn parse_datetime_value(
    value: &Value,
    filter_name: &str,
) -> Result<chrono::DateTime<chrono::Utc>, MJError> {
    let dt_str = value.to_string();
    if let Ok(parsed) = chrono::DateTime::parse_from_rfc3339(&dt_str) {
        Ok(parsed.with_timezone(&chrono::Utc))
    } else if let Ok(parsed) = chrono::NaiveDateTime::parse_from_str(&dt_str, "%Y-%m-%d %H:%M:%S") {
        Ok(chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(
            parsed,
            chrono::Utc,
        ))
    } else if let Ok(parsed) = chrono::NaiveDate::parse_from_str(&dt_str, "%Y-%m-%d") {
        let ndt = parsed.and_hms_opt(0, 0, 0).unwrap();
        Ok(chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(
            ndt,
            chrono::Utc,
        ))
    } else {
        Err(MJError::new(
            MJErrorKind::InvalidOperation,
            format!("{filter_name} filter: could not parse date/time value: {dt_str}"),
        ))
    }
}

fn format_timespan(seconds: i64) -> String {
    if seconds < 60 {
        return "0 minutes".to_string();
    }
    const CHUNKS: &[(i64, &str, &str)] = &[
        (365 * 86400, "year", "years"),
        (30 * 86400, "month", "months"),
        (7 * 86400, "week", "weeks"),
        (86400, "day", "days"),
        (3600, "hour", "hours"),
        (60, "minute", "minutes"),
    ];

    let mut remaining = seconds;
    let mut parts = Vec::new();
    for &(chunk_secs, singular, plural) in CHUNKS {
        let count = remaining / chunk_secs;
        if count > 0 {
            let label = if count == 1 { singular } else { plural };
            parts.push(format!("{count} {label}"));
            remaining -= count * chunk_secs;
            if parts.len() == 2 {
                break;
            }
        } else if !parts.is_empty() {
            // Only emit adjacent units (Django's timesince convention).
            break;
        }
    }

    if parts.is_empty() {
        "0 minutes".to_string()
    } else {
        parts.join(", ")
    }
}

fn escape_html_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#x27;"),
            _ => out.push(c),
        }
    }
    out
}

/// Safely output a value as JSON inside a `<script type="application/json">` tag.
///
/// Escapes `<`, `>`, and `&` as Unicode escape sequences (`\u003C`, `\u003E`,
/// `\u0026`) so the serialized JSON cannot break out of the `<script>` tag.
///
/// Example: `{{ data|json_script("my-data") }}`
pub fn json_script(value: Value, element_id: String) -> Result<Value, MJError> {
    let raw_json = serde_json::to_string(&value).map_err(|e| {
        MJError::new(
            MJErrorKind::InvalidOperation,
            format!("json_script filter: failed to serialize JSON: {e}"),
        )
    })?;
    let safe_json = raw_json
        .replace('<', "\\u003C")
        .replace('>', "\\u003E")
        .replace('&', "\\u0026");
    let escaped_id = crate::functions::escape_html_attr(&element_id);
    let html = format!(r#"<script id="{escaped_id}" type="application/json">{safe_json}</script>"#);
    Ok(Value::from_safe_string(html))
}

/// Format the time elapsed since a given date/time (e.g. `"2 days, 3 hours"`).
///
/// Optionally takes a reference date/time argument; defaults to `Utc::now()`.
/// Returns `"0 minutes"` if the value is in the future relative to the reference.
pub fn timesince(value: Value, arg: Option<Value>) -> Result<String, MJError> {
    let dt = parse_datetime_value(&value, "timesince")?;
    let reference = match arg {
        Some(ref v) if !v.is_none() && !v.is_undefined() => parse_datetime_value(v, "timesince")?,
        _ => chrono::Utc::now(),
    };
    let diff = (reference - dt).num_seconds();
    if diff <= 0 {
        Ok("0 minutes".to_string())
    } else {
        Ok(format_timespan(diff))
    }
}

/// Format the time remaining until a given date/time (e.g. `"2 days, 3 hours"`).
///
/// Optionally takes a reference date/time argument; defaults to `Utc::now()`.
/// Returns `"0 minutes"` if the value is in the past relative to the reference.
pub fn timeuntil(value: Value, arg: Option<Value>) -> Result<String, MJError> {
    let dt = parse_datetime_value(&value, "timeuntil")?;
    let reference = match arg {
        Some(ref v) if !v.is_none() && !v.is_undefined() => parse_datetime_value(v, "timeuntil")?,
        _ => chrono::Utc::now(),
    };
    let diff = (dt - reference).num_seconds();
    if diff <= 0 {
        Ok("0 minutes".to_string())
    } else {
        Ok(format_timespan(diff))
    }
}

/// Replace line breaks in plain text with HTML `<p>` and `<br>` tags.
///
/// A single newline becomes `<br>`; two or more consecutive newlines start a
/// new `<p>...</p>` paragraph. HTML characters in the input are escaped.
pub fn linebreaks(value: Value) -> Result<Value, MJError> {
    let normalized = value.to_string().replace("\r\n", "\n").replace('\r', "\n");
    let escaped = escape_html_text(&normalized);
    let mut paragraphs = Vec::new();
    let mut current = String::new();
    let mut prev_blank = false;

    for line in escaped.split('\n') {
        if line.trim().is_empty() {
            if !current.is_empty() {
                paragraphs.push(format!("<p>{current}</p>"));
                current.clear();
            }
            prev_blank = true;
        } else {
            if !current.is_empty() && !prev_blank {
                current.push_str("<br>");
            }
            current.push_str(line);
            prev_blank = false;
        }
    }
    if !current.is_empty() {
        paragraphs.push(format!("<p>{current}</p>"));
    }

    Ok(Value::from_safe_string(paragraphs.join("\n\n")))
}

/// Replace all newlines in plain text with HTML `<br>` tags.
///
/// HTML characters in the input are escaped before replacing newlines.
pub fn linebreaksbr(value: Value) -> Result<Value, MJError> {
    let normalized = value.to_string().replace("\r\n", "\n").replace('\r', "\n");
    let escaped = escape_html_text(&normalized);
    Ok(Value::from_safe_string(escaped.replace('\n', "<br>")))
}

/// Convert a string into a URL-friendly slug.
///
/// Lowercases ASCII letters, replaces spaces and underscores with hyphens,
/// strips non-alphanumeric characters, collapses repeated hyphens, and trims
/// leading/trailing hyphens.
pub fn slugify(value: Value) -> Result<String, MJError> {
    let s = value.to_string().to_lowercase();
    let mut out = String::with_capacity(s.len());
    let mut last_was_hyphen = true;
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
            last_was_hyphen = false;
        } else if (c.is_whitespace() || c == '-' || c == '_') && !last_was_hyphen {
            out.push('-');
            last_was_hyphen = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    Ok(out)
}

/// Convert URLs (`http://`, `https://`, `www.`) in plain text into clickable HTML links.
///
/// HTML characters in surrounding text are escaped and links include `rel="nofollow"`.
pub fn urlize(value: Value) -> Result<Value, MJError> {
    let text = value.to_string();
    let mut out = String::with_capacity(text.len());
    let mut rest = text.as_str();

    while !rest.is_empty() {
        if let Some(ws_len) = rest
            .chars()
            .take_while(|c| c.is_whitespace())
            .map(char::len_utf8)
            .sum::<usize>()
            .checked_sub(0)
            .filter(|&n| n > 0)
        {
            out.push_str(&rest[..ws_len]);
            rest = &rest[ws_len..];
            continue;
        }

        let token_len = rest.find(char::is_whitespace).unwrap_or(rest.len());
        let token = &rest[..token_len];
        rest = &rest[token_len..];

        let trimmed_end = token.trim_end_matches(['.', ',', ')', '!', '?', ';', ':']);
        let trailing = &token[trimmed_end.len()..];

        if trimmed_end.starts_with("http://")
            || trimmed_end.starts_with("https://")
            || trimmed_end.starts_with("www.")
        {
            let href = if trimmed_end.starts_with("www.") {
                format!("https://{trimmed_end}")
            } else {
                trimmed_end.to_string()
            };
            let escaped_href = crate::functions::escape_html_attr(&href);
            let escaped_label = escape_html_text(trimmed_end);
            write!(
                &mut out,
                r#"<a href="{escaped_href}" rel="nofollow">{escaped_label}</a>"#
            )
            .unwrap();
            out.push_str(&escape_html_text(trailing));
        } else {
            out.push_str(&escape_html_text(token));
        }
    }

    Ok(Value::from_safe_string(out))
}

/// Strip all HTML tags (`<...>`) from a string.
pub fn striptags(value: Value) -> Result<String, MJError> {
    let s = value.to_string();
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    Ok(out)
}

/// Map `true`, `false`, and `None`/undefined values to custom strings.
///
/// Expects a comma-separated argument like `"yeah,no,maybe"` (default: `"yes,no,maybe"`).
/// If only two values are provided (`"yes,no"`), `None` maps to the `no` value.
pub fn yesno(value: Value, arg: Option<String>) -> Result<String, MJError> {
    let spec = arg.unwrap_or_else(|| "yes,no,maybe".to_string());
    let parts: Vec<&str> = spec.split(',').collect();
    if parts.len() < 2 {
        return Err(MJError::new(
            MJErrorKind::InvalidOperation,
            "yesno filter: argument must contain at least two comma-separated values",
        ));
    }
    let yes_str = parts[0];
    let no_str = parts[1];
    let maybe_str = parts.get(2).copied().unwrap_or(no_str);

    if value.is_none() || value.is_undefined() {
        Ok(maybe_str.to_string())
    } else if value.is_true() {
        Ok(yes_str.to_string())
    } else {
        Ok(no_str.to_string())
    }
}

/// Convert an integer to its English ordinal string (`1st`, `2nd`, `3rd`, `4th`, `11th`, `21st`).
pub fn ordinal(value: Value) -> Result<String, MJError> {
    let n = if let Ok(i) = i64::try_from(value.clone()) {
        i
    } else if let Some(s) = value.as_str() {
        s.trim().parse::<i64>().map_err(|e| {
            MJError::new(
                MJErrorKind::InvalidOperation,
                format!("ordinal filter: value must be an integer: {e}"),
            )
        })?
    } else {
        return Err(MJError::new(
            MJErrorKind::InvalidOperation,
            format!("ordinal filter: value must be an integer: {value}"),
        ));
    };

    let abs = n.unsigned_abs();
    let suffix = if matches!(abs % 100, 11..=13) {
        "th"
    } else {
        match abs % 10 {
            1 => "st",
            2 => "nd",
            3 => "rd",
            _ => "th",
        }
    };
    Ok(format!("{n}{suffix}"))
}

/// Format a date as `"today"`, `"yesterday"`, `"tomorrow"`, or fall back to formatted date.
pub fn naturalday(value: Value, format_str: Option<String>) -> Result<String, MJError> {
    let dt = parse_datetime_value(&value, "naturalday")?;
    let today = chrono::Utc::now().date_naive();
    let delta = (dt.date_naive() - today).num_days();
    match delta {
        0 => Ok("today".to_string()),
        -1 => Ok("yesterday".to_string()),
        1 => Ok("tomorrow".to_string()),
        _ => date(value, format_str),
    }
}

/// For numbers 1 through 9, return the Associated Press style word (`"one"`..`"nine"`);
/// otherwise return the number unchanged.
pub fn apnumber(value: Value) -> Result<String, MJError> {
    let n = if let Ok(i) = i64::try_from(value.clone()) {
        i
    } else if let Some(s) = value.as_str() {
        match s.trim().parse::<i64>() {
            Ok(i) => i,
            Err(_) => return Ok(s.to_string()),
        }
    } else {
        return Ok(value.to_string());
    };

    let word = match n {
        1 => "one",
        2 => "two",
        3 => "three",
        4 => "four",
        5 => "five",
        6 => "six",
        7 => "seven",
        8 => "eight",
        9 => "nine",
        _ => return Ok(n.to_string()),
    };
    Ok(word.to_string())
}

/// Sort a list of dictionaries/objects by the given key.
///
/// Example: `{{ items|dictsort("age") }}`
pub fn dictsort(value: Value, key: String) -> Result<Value, MJError> {
    let iter = value.try_iter().map_err(|e| {
        MJError::new(
            MJErrorKind::InvalidOperation,
            format!("dictsort filter: value must be iterable: {e}"),
        )
    })?;
    let mut items: Vec<Value> = iter.collect();
    items.sort_by(|a, b| {
        let va = a.get_attr(&key).unwrap_or(Value::UNDEFINED);
        let vb = b.get_attr(&key).unwrap_or(Value::UNDEFINED);
        match (f64::try_from(va.clone()), f64::try_from(vb.clone())) {
            (Ok(fa), Ok(fb)) => fa.partial_cmp(&fb).unwrap_or(std::cmp::Ordering::Equal),
            _ => va.to_string().cmp(&vb.to_string()),
        }
    });
    Ok(Value::from(items))
}
