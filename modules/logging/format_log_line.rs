use crate::*;

pub(crate) struct LineFormat;

pub fn timestamp() -> String {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO);
    let days = (elapsed.as_secs() / 86_400) as i64;
    let seconds = elapsed.as_secs() % 86_400;
    let (year, month, day) = civil_date(days);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02},{:03} UTC",
        seconds / 3600,
        seconds / 60 % 60,
        seconds % 60,
        elapsed.subsec_millis()
    )
}

fn civil_date(days_since_epoch: i64) -> (i64, i64, i64) {
    let shifted = days_since_epoch + 719_468;
    let era = if shifted >= 0 {
        shifted
    } else {
        shifted - 146_096
    } / 146_097;
    let day_of_era = shifted - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    (year, month, day)
}

fn single_line(value: &str) -> String {
    value.replace('\r', "\\r").replace('\n', "\\n")
}

pub fn format_line(level: char, source: &str, message: &str) -> String {
    let source = single_line(source).replace(']', "_");
    format!(
        "[{level} {}] [{source}] {}",
        timestamp(),
        single_line(message)
    )
}

#[derive(Default)]
struct Fields {
    message: String,
    mod_id: Option<String>,
    extra: Vec<String>,
}

impl Visit for Fields {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn fmt::Debug) {
        let value = format!("{value:?}");
        match field.name() {
            "message" => self.message = value,
            "mod_id" => self.mod_id = Some(value.trim_matches('"').to_owned()),
            _ => self.extra.push(format!("{}={value}", field.name())),
        }
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        match field.name() {
            "message" => self.message = value.to_owned(),
            "mod_id" => self.mod_id = Some(value.to_owned()),
            _ => self.extra.push(format!("{}={value}", field.name())),
        }
    }
}

impl<S, N> FormatEvent<S, N> for LineFormat
where
    S: Subscriber + for<'lookup> LookupSpan<'lookup>,
    N: for<'writer> FormatFields<'writer> + 'static,
{
    fn format_event(
        &self,
        _context: &FmtContext<'_, S, N>,
        mut writer: Writer<'_>,
        event: &Event<'_>,
    ) -> fmt::Result {
        let level = match *event.metadata().level() {
            tracing::Level::TRACE => 'T',
            tracing::Level::DEBUG => 'D',
            tracing::Level::INFO => 'I',
            tracing::Level::WARN => 'W',
            tracing::Level::ERROR => 'E',
        };
        let mut fields = Fields::default();
        event.record(&mut fields);
        let target = event.metadata().target();
        let source = fields.mod_id.as_deref().unwrap_or_else(|| match target {
            target if target.starts_with("shroudforge_api") => "api",
            target if target.starts_with("shroudforge_package") => "package",
            target if target.starts_with("shroudforge_modloader") => "runtime",
            _ => target.strip_prefix("shroudforge::").unwrap_or(target),
        });
        write!(writer, "{}", format_line(level, source, &fields.message))?;
        if !fields.extra.is_empty() {
            write!(writer, " {}", single_line(&fields.extra.join(" ")))?;
        }
        writeln!(writer)
    }
}
