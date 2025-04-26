use super::{
    about, add, api, check, favicon, forward_me, index, report, save, Build, DirectoryDbConn,
    Instance, InstancesCache, Relaxed, Rocket, State, Template, CRON_INTERVAL,
};
use diesel::prelude::*;
use diesel::query_builder::SqlQuery;
use diesel_migrations::EmbeddedMigrations;
use diesel_migrations::MigrationHarness;
use regex::Regex;
use rocket::fs::FileServer;
use rocket_dyn_templates::tera::{to_value, try_get_value, Result, Value};
use std::collections::HashMap;
use std::sync::atomic::AtomicU64;
use std::sync::OnceLock;
use std::sync::RwLock;

// 1F1E6 is the unicode code point for the "REGIONAL INDICATOR SYMBOL
// LETTER A" and 41 is the one for A in unicode and ASCII
const REGIONAL_INDICATOR_OFFSET: u32 = 0x1F1E6 - 0x41;
#[cfg(not(test))]
const CACHE_TIMEOUT: u64 = 300; // 5 minutes
#[cfg(test)]
const CACHE_TIMEOUT: u64 = 1; // 1 second, for unit tests
static SLASHES_EXP: OnceLock<Regex> = OnceLock::new();

pub fn get_epoch() -> u64 {
    use std::time::SystemTime;
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .expect(
            "Negative seconds since UNIX epoch returned - your running software from the future.",
        )
        .as_secs()
}

pub fn get_instances() -> SqlQuery {
    diesel::dsl::sql_query(
        "SELECT instances.id, url, version, https, https_redirect, country_id, \
            attachments, csp_header, (100 * SUM(checks.up) / COUNT(checks.up)) AS uptime, \
            mozilla_observatory.rating AS rating_mozilla_observatory \
            FROM instances \
            JOIN checks ON instances.id = checks.instance_id \
            JOIN ( \
                SELECT rating, percent, instance_id \
                FROM scans WHERE scanner = \"mozilla_observatory\" \
            ) AS mozilla_observatory ON instances.id = mozilla_observatory.instance_id \
            GROUP BY instances.id \
            ORDER BY version DESC, https DESC, https_redirect DESC, csp_header DESC, \
            mozilla_observatory.percent DESC, attachments DESC, uptime DESC, url ASC \
            LIMIT 1000",
    )
}

pub fn is_cached(cache: &RwLock<HashMap<String, u64>>, key: &str) -> bool {
    if let Ok(read_cache) = cache.read() {
        if let Some(timestamp) = read_cache.get(key) {
            if *timestamp < get_epoch() - CACHE_TIMEOUT {
                drop(read_cache); // drop read lock, before requesting a write one
                if let Ok(mut write_cache) = cache.write() {
                    write_cache.remove(key);
                }
            } else {
                return true;
            }
        }
    }
    false
}

pub fn rating_to_percent(rating: &str) -> u8 {
    // see https://en.wikipedia.org/wiki/Academic_grading_in_the_United_States#Numerical_and_letter_grades
    match rating {
        "A+" => 97,
        "A" => 93,
        "A-" => 90,
        "B+" => 87,
        "B" => 83,
        "B-" => 80,
        "C+" => 77,
        "C" => 73,
        "C-" => 70,
        "D+" => 67,
        "D" => 63,
        "D-" => 60,
        "F" => 50,
        _ => 0,
    }
}

pub fn rocket() -> Rocket<Build> {
    rocket::build()
        .mount(
            "/",
            routes![about, add, api, check, favicon, forward_me, index, report, save],
        )
        .mount("/img", FileServer::from("img"))
        .mount("/css", FileServer::from("css"))
        .attach(DirectoryDbConn::fairing())
        .attach(Template::custom(|engines| {
            engines.tera.register_filter("country", filter_country);
        }))
        .manage(InstancesCache {
            timeout: AtomicU64::new(0),
            instances: RwLock::new(vec![]),
            negative_lookups: RwLock::new(HashMap::new()),
        })
}

pub async fn run_db_migrations(rocket: Rocket<Build>) -> Rocket<Build> {
    pub const MIGRATIONS: EmbeddedMigrations = embed_migrations!();
    let db = DirectoryDbConn::get_one(&rocket)
        .await
        .expect("database connection");
    db.run(|conn| {
        conn.run_pending_migrations(MIGRATIONS)
            .expect("diesel migrations");
    })
    .await;
    rocket
}

pub fn set_cached(cache: &RwLock<HashMap<String, u64>>, key: &str) {
    if let Ok(mut write_cache) = cache.write() {
        write_cache.insert(key.into(), get_epoch());
    }
}

pub fn strip_url(url: String) -> String {
    let mut check_url = url;
    // remove query from URL
    if let Some(query_start) = check_url.find('?') {
        check_url = check_url[..query_start].into();
    }
    // remove hash from URL
    if let Some(query_start) = check_url.find('#') {
        check_url = check_url[..query_start].into();
    }
    // remove trailing index.php
    if let Some(stripped_url) = check_url.strip_suffix("index.php") {
        check_url = stripped_url.into();
    }
    // remove trailing slash, but only for web root, not for paths:
    // - https://example.com/ -> https://example.com
    // - https://example.com// -> https://example.com
    // - but https://example.com/path/ remains unchanged
    let (schema, uri) = check_url.split_at(7);
    let cleaned_uri = SLASHES_EXP
        .get_or_init(|| Regex::new(r"/{2,}").unwrap())
        .replace_all(uri, "/");
    check_url = format!("{schema}{cleaned_uri}");
    if check_url.matches('/').count() == 3 {
        check_url = check_url.trim_end_matches('/').into();
    }
    check_url
}

pub async fn update_instance_cache(db: DirectoryDbConn, cache: &State<InstancesCache>) {
    let now = get_epoch();
    if now >= cache.timeout.load(Relaxed) {
        match db.run(|conn| get_instances().load::<Instance>(conn)).await {
            // flush cache
            Ok(instances_live) => {
                cache.timeout.store(now + CRON_INTERVAL, Relaxed);
                if let Ok(mut instances_cache) = cache.instances.write() {
                    *instances_cache = instances_live;
                }
            }
            // database might be write-locked, try it again in a minute
            Err(_) => cache.timeout.store(now + 60, Relaxed),
        }
    }
}

pub fn filter_country(value: &Value, args: &HashMap<String, Value>) -> Result<Value> {
    use isocountry::CountryCode;
    let country_code = try_get_value!("country", "value", String, value);
    let mut country_code_points = ['A', 'Q'];
    let mut country_chars = country_code.chars();
    for country_code_point in country_code_points.iter_mut() {
        if let Some(char_code_point) = country_chars.next() {
            if let Some(character) =
                std::char::from_u32(REGIONAL_INDICATOR_OFFSET + char_code_point as u32)
            {
                *country_code_point = character;
            }
        }
    }
    let country_name = match CountryCode::for_alpha2(&country_code) {
        Ok(country) => country.name(),
        Err(_) => "Unknown country",
    };
    let country_emoji = country_code_points.iter().cloned().collect::<String>();
    macro_rules! TABLE_CELL_FORMAT {
        () => {
            "<td title=\"{0}\" aria-label=\"{0}\">{1}</td>"
        };
    }
    let output = match args.get("label") {
        Some(label) => match try_get_value!("country", "label", bool, label) {
            true => format!("{country_name}. {country_emoji}"),
            false => format!(TABLE_CELL_FORMAT!(), country_name, country_emoji),
        },
        None => format!(TABLE_CELL_FORMAT!(), country_name, country_emoji),
    };
    Ok(to_value(output).unwrap_or(Value::Null))
}
