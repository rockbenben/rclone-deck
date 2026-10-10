use chrono::{DateTime, Local, NaiveDate};

/// 界面上那一句"几点查的 / 几点算的"的唯一格式化处。
///
/// 口径来自 `DESIGN.md` 的时间那一档：**今天 16:05 / 昨天 21:40 / 再往前直接写 10-01 09:12**（没有「跨过一周才换样式」那一档），
/// 秒与纳秒只留在日志里。面板可能整夜开着，所以只带时分不带日期的读数跨过午夜就说假话——
/// 这儿的"今天/昨天"就是那条风险的解法，而不是装饰。
///
/// **格式化归后台，不归界面**：`measured_at` 交出去的就是成品字符串。要在 TS 里
/// 把"几月几日"再解释成"今天"，就是拿浏览器的时钟去解释后台那一刻（而那一刻两侧
/// 虽然同机同源，判定的手却分了两处），也正是本仓一直守着的那条：判定与解释
/// 作为数据走出去，界面只渲染。
pub fn label(moment: DateTime<Local>, now: DateTime<Local>) -> String {
    let hhmm = moment.format("%H:%M").to_string();
    match day_relation(moment.date_naive(), now.date_naive()) {
        DayRelation::Today => format!("今天 {hhmm}"),
        DayRelation::Yesterday => format!("昨天 {hhmm}"),
        DayRelation::Older => format!("{} {hhmm}", moment.format("%m-%d")),
    }
}

/// "就是现在的那一刻"的那一句：两个报告（对账 / 盘点）取时刻都走这一句。
pub fn now_label() -> String {
    let now = Local::now();
    label(now, now)
}

enum DayRelation { Today, Yesterday, Older }

fn day_relation(day: NaiveDate, today: NaiveDate) -> DayRelation {
    if day == today {
        return DayRelation::Today;
    }
    // `checked_sub_days` 而不是裸 `pred()`：后者在 `NaiveDate::MIN` 上会 panic，
    // 而这条函数是给界面用的，不该带一条能panic的路。
    let is_yesterday = today.checked_sub_days(chrono::Days::new(1))
        .map(|y| day == y)
        .unwrap_or(false);
    if is_yesterday { DayRelation::Yesterday } else { DayRelation::Older }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(y: i32, m: u32, d: u32, hh: u32, mm: u32) -> DateTime<Local> {
        Local.with_ymd_and_hms(y, m, d, hh, mm, 0).single().expect("这个本地时刻唯一")
    }

    /// 三条臂各钉一条，并且都从**同一对**日期推：今天/昨天不是字符串里写死的，
    /// 而是拿 `now` 减出来的——把 `Yesterday` 那臂接错到 `Older`，下面第二条就红。
    #[test]
    fn same_day_reads_as_today_with_the_clock() {
        let now = at(2026, 10, 3, 16, 5);
        assert_eq!(label(at(2026, 10, 3, 14, 22), now), "今天 14:22");
    }

    #[test]
    fn previous_day_reads_as_yesterday_not_as_a_date() {
        let now = at(2026, 10, 3, 0, 30);
        assert_eq!(label(at(2026, 10, 2, 21, 40), now), "昨天 21:40");
    }

    #[test]
    fn older_days_keep_the_date_and_drop_the_year() {
        let now = at(2026, 10, 3, 16, 5);
        assert_eq!(label(at(2026, 10, 1, 9, 12), now), "10-01 09:12");
        // 跨年也一样带着月-日：面板可能连着几周开着，只写时分就说不清是哪一次。
        assert_eq!(label(at(2025, 12, 31, 23, 59), now), "12-31 23:59");
    }

    /// 契约那条"秒只留在日志里"：那一句的时间段永远只有时分（五位、一个冒号）。
    #[test]
    fn the_surface_moment_never_carries_seconds() {
        let now = at(2026, 10, 3, 16, 5);
        for moment in [at(2026, 10, 3, 14, 22), at(2026, 10, 2, 21, 40), at(2026, 9, 1, 9, 12)] {
            let s = label(moment, now);
            let clock = s.split(' ').last().expect("那一句里有时间");
            assert_eq!(clock.matches(':').count(), 1, "那一句里有秒：{s}");
            assert_eq!(clock.len(), 5, "那不是一档时分：{s}");
        }
    }
}
