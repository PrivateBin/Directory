use super::{sql_query, DirectoryDbConn, Instance, InstancesCache, Ordering, State, CRON_INTERVAL};
use diesel::prelude::*;
use diesel::query_builder::SqlQuery;
use rocket_contrib::templates::tera::{self, to_value, try_get_value, Value};
use std::collections::HashMap;

// 1F1E6 is the unicode code point for the "REGIONAL INDICATOR SYMBOL
// LETTER A" and 41 is the one for A in unicode and ASCII
const REGIONAL_INDICATOR_OFFSET: u32 = 0x1F1E6 - 0x41;

pub fn get_epoch() -> u64 {
    use std::time::SystemTime;
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

pub fn get_instances() -> SqlQuery {
    sql_query(
        "SELECT instances.id, url, version, https, https_redirect, attachments, \
            country_id, (100 * SUM(checks.up) / COUNT(checks.up)) AS uptime, \
            mozilla_observatory.rating AS rating_mozilla_observatory \
            FROM instances \
            JOIN checks ON instances.id = checks.instance_id \
            JOIN ( \
                SELECT rating, percent, instance_id \
                FROM scans WHERE scanner = \"mozilla_observatory\" \
            ) AS mozilla_observatory ON instances.id = mozilla_observatory.instance_id \
            GROUP BY instances.id \
            ORDER BY version DESC, https DESC, https_redirect DESC, \
            mozilla_observatory.percent DESC, attachments DESC, uptime DESC, url ASC \
            LIMIT 1000",
    )
}

pub fn rating_to_percent(rating: &str) -> u8 {
    // see https://en.wikipedia.org/wiki/Academic_grading_in_the_United_States#Numerical_and_letter_grades
    let percent: u8;
    match rating {
        "A+" => percent = 97,
        "A" => percent = 93,
        "A-" => percent = 90,
        "B+" => percent = 87,
        "B" => percent = 83,
        "B-" => percent = 80,
        "C+" => percent = 77,
        "C" => percent = 73,
        "C-" => percent = 70,
        "D+" => percent = 67,
        "D" => percent = 63,
        "D-" => percent = 60,
        "F" => percent = 50,
        _ => percent = 0,
    }
    percent
}

pub fn update_instance_cache(conn: DirectoryDbConn, cache: &State<InstancesCache>) {
    let now = get_epoch();
    if now >= cache.timeout.load(Ordering::Relaxed) {
        match get_instances().load::<Instance>(&*conn) {
            // flush cache
            Ok(instances_live) => {
                cache.timeout.store(now + CRON_INTERVAL, Ordering::Relaxed);
                let mut instances_cache = cache.instances.write().unwrap();
                *instances_cache = instances_live;
            }
            // database might be write-locked, try it again in a minute
            Err(_) => cache.timeout.store(now + 60, Ordering::Relaxed),
        }
    }
}

pub fn filter_country(string: Value, _: HashMap<String, Value>) -> tera::Result<Value> {
    use isocountry::CountryCode;
    let country_code = try_get_value!("country", "value", String, string);
    let mut country_chars = country_code.chars();
    let country_code_points = [
        std::char::from_u32(REGIONAL_INDICATOR_OFFSET + country_chars.next().unwrap() as u32)
            .unwrap(),
        std::char::from_u32(REGIONAL_INDICATOR_OFFSET + country_chars.next().unwrap() as u32)
            .unwrap(),
    ];
    Ok(to_value(format!(
        "<td title=\"{0}\" aria-label=\"{0}\">{1}</td>",
        CountryCode::for_alpha2(&country_code).unwrap().name(),
        country_code_points.iter().cloned().collect::<String>()
    ))
    .unwrap())
}
