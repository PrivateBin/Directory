use super::models::*;
use super::schema::instances::dsl::*;
use super::{get_epoch, get_instances, sql_query, Build, Rocket};
use diesel::prelude::*;
use diesel::SqliteConnection;
use futures::future::select_all;
use rocket_sync_db_pools::Config;
use std::fmt::Write;
use std::time::{Duration, Instant};
use tokio::time::sleep;

pub const CRON_INTERVAL: u64 = 900; // 15 minutes
pub const CHECKS_TO_STORE: u64 = 100; // amount of checks to keep
pub const MAX_FAILURES: u64 = 90; // remove instances that failed this many times

struct InstanceCheckResult {
    message: String,
    scan_update: Option<ScanNew>,
    scan_update_success: String,
    instance: Instance,
    instance_update: Option<InstanceNew>,
    instance_update_success: String,
}

pub async fn check_full(rocket: Rocket<Build>) {
    use super::schema::scans::dsl::{instance_id, percent, rating, scanner, scans};

    let directory_config =
        Config::from("directory", &rocket).expect("configuration of directory database");
    let conn = SqliteConnection::establish(&directory_config.url)
        .expect("connection to directory database");
    let cached_instances = get_instances().load::<Instance>(&conn);
    match cached_instances {
        Ok(instance_list) => {
            let mut instance_update_queries = vec![];
            let mut scan_update_queries = vec![];
            let mut children = vec![];
            for instance in instance_list.into_iter() {
                children.push(check_instance(instance));
            }
            let mut pinned_children: Vec<_> = children.into_iter().map(Box::pin).collect();
            while !pinned_children.is_empty() {
                let (result, _index, remaining_children) = select_all(pinned_children).await;
                pinned_children = remaining_children;
                let message = result.message;
                print!("{message}");

                // robots.txt must have changed or site no longer an instance, delete it immediately
                if message.ends_with("doesn't want to get added to the directory.\n")
                    || message.ends_with("doesn't seem to be a PrivateBin instance.\n")
                {
                    match diesel::delete(instances.filter(id.eq(result.instance.id))).execute(&conn)
                    {
                        Ok(_) => println!("    removed the instance, due to: {message}"),
                        Err(e) => {
                            println!("    error removing the instance: {e:?}");
                        }
                    }
                    continue;
                }

                if let Some(updated_scan) = result.scan_update {
                    scan_update_queries.push((
                        diesel::update(
                            scans
                                .filter(instance_id.eq(result.instance.id))
                                .filter(scanner.eq(updated_scan.scanner)),
                        )
                        .set((
                            rating.eq(updated_scan.rating.clone()),
                            percent.eq(updated_scan.percent),
                        )),
                        result.scan_update_success,
                        result.instance.url.clone(),
                    ));
                }
                if let Some(updated_instance) = result.instance_update {
                    instance_update_queries.push((
                        diesel::update(instances.filter(id.eq(result.instance.id))).set((
                            version.eq(updated_instance.version),
                            https.eq(updated_instance.https),
                            https_redirect.eq(updated_instance.https_redirect),
                            attachments.eq(updated_instance.attachments),
                            country_id.eq(updated_instance.country_id),
                        )),
                        result.instance_update_success,
                        result.instance.url,
                    ));
                }
            }

            let timer = Instant::now();
            for (query, query_success, instance_url) in instance_update_queries {
                match query.execute(&conn) {
                    Ok(_) => {
                        println!("{query_success}");
                    }
                    Err(e) => {
                        println!("Instance {instance_url} failed to be updated with error: {e:?}");
                    }
                }
            }
            println!(
                "all instance update queries concluded ({:?})",
                timer.elapsed()
            );

            let timer = Instant::now();
            for (query, query_success, instance_url) in scan_update_queries {
                match query.execute(&conn) {
                    Ok(_) => {
                        println!("{query_success}");
                    }
                    Err(e) => {
                        println!("Instance {instance_url} failed to be updated with error: {e:?}");
                    }
                }
            }
            println!("all scan update queries concluded ({:?})", timer.elapsed());

            // delete checks and instances that failed too many times
            let timer = Instant::now();
            match sql_query(&format!(
                "DELETE FROM instances \
                WHERE id in ( \
                    SELECT instance_id \
                    FROM checks \
                    WHERE up = 0 \
                    GROUP BY instance_id \
                    HAVING COUNT(up) >= {MAX_FAILURES} \
                );"
            ))
            .execute(&conn)
            {
                Ok(_) => println!(
                    "removed instances that failed too many times ({:?})",
                    timer.elapsed()
                ),
                Err(e) => {
                    println!("error removing instances failing too many times: {e:?}");
                }
            }
        }
        Err(e) => {
            println!("failed retrieving instances from database with error: {e:?}");
        }
    }
}

async fn check_instance(instance: Instance) -> InstanceCheckResult {
    let timer = Instant::now();
    let mut message = String::new();
    let mut instance_options = [
        ("version", instance.version.clone(), String::new()),
        (
            "https",
            format!("{:?}", instance.https.clone()),
            String::new(),
        ),
        (
            "https_redirect",
            format!("{:?}", instance.https_redirect.clone()),
            String::new(),
        ),
        (
            "attachments",
            format!("{:?}", instance.attachments.clone()),
            String::new(),
        ),
        ("country_id", instance.country_id.clone(), String::new()),
    ];
    let mut scan: ScanNew;
    let mut instance_update = None;
    let mut instance_update_success = String::new();
    let mut scan_update = None;
    let mut scan_update_success = String::new();
    let instance_url = instance.url.clone();
    match PrivateBin::new(instance.url.clone()).await {
        Ok(privatebin) => {
            instance_options[0].2 = privatebin.instance.version.clone();
            instance_options[1].2 = format!("{:?}", privatebin.instance.https.clone());
            instance_options[2].2 = format!("{:?}", privatebin.instance.https_redirect.clone());
            instance_options[3].2 = format!("{:?}", privatebin.instance.attachments.clone());
            instance_options[4].2 = privatebin.instance.country_id.clone();
            let elapsed = timer.elapsed();
            let timer = Instant::now();
            if instance_options.iter().any(|x| x.1 != x.2) {
                instance_update = Some(privatebin.instance);
                let _ = writeln!(
                    &mut instance_update_success,
                    "Instance {instance_url} checked and updated ({elapsed:?}):"
                );
                for (label, old, new) in instance_options.iter() {
                    if old != new {
                        let _ = writeln!(
                            &mut instance_update_success,
                            "    {label} was {old}, updated to {new}"
                        );
                    }
                }
            } else {
                let _ = writeln!(
                    &mut message,
                    "Instance {instance_url} checked, no update required ({elapsed:?})"
                );
            }

            // retrieve latest scan
            scan = privatebin.scans[0].clone();
            // if missing, wait for the scan to conclude and poll again
            let rating = scan.rating.clone();
            if rating == "-" {
                sleep(Duration::from_secs(5)).await;
                scan = PrivateBin::check_rating_mozilla_observatory(&instance_url).await;
            }
            let elapsed = timer.elapsed();
            if rating != "-" && rating != instance.rating_mozilla_observatory {
                scan_update = Some(scan.clone());
                let _ = writeln!(
                    &mut scan_update_success,
                    "Instance {instance_url} rating updated to: {rating} ({elapsed:?})"
                );
            } else {
                let _ = writeln!(
                    &mut scan_update_success,
                    "Instance {instance_url} rating remains unchanged at: {rating} ({elapsed:?})"
                );
            }
        }
        Err(e) => {
            let _ = writeln!(
                &mut message,
                "Instance {instance_url} failed to be checked with error: {e:?}"
            );
        }
    }

    InstanceCheckResult {
        message,
        scan_update,
        scan_update_success,
        instance,
        instance_update,
        instance_update_success,
    }
}

async fn check_instance_up(instance: Instance) -> (String, CheckNew, Duration) {
    // measure instance being up or down
    let timer = Instant::now();
    let check_result = CheckNew::new(instance.check_up().await, instance.id);
    (instance.url, check_result, timer.elapsed())
}

pub async fn check_up(rocket: Rocket<Build>) {
    use super::schema::checks::dsl::{checks, updated};

    let directory_config =
        Config::from("directory", &rocket).expect("configuration of directory database");
    let conn = SqliteConnection::establish(&directory_config.url)
        .expect("connection to directory database");
    let cached_instances = get_instances().load::<Instance>(&conn);
    match cached_instances {
        Ok(instance_list) => {
            let mut instance_checks = vec![];
            let mut children = vec![];
            for instance in instance_list.into_iter() {
                children.push(check_instance_up(instance));
            }
            let mut pinned_children: Vec<_> = children.into_iter().map(Box::pin).collect();
            while !pinned_children.is_empty() {
                let ((instance_url, instance_check, elapsed), _index, remaining_children) =
                    select_all(pinned_children).await;
                instance_checks.push(instance_check);
                println!("Instance {instance_url} checked ({elapsed:?})");
                pinned_children = remaining_children;
            }

            // store checks
            let timer = Instant::now();
            match diesel::insert_into(checks)
                .values(&instance_checks)
                .execute(&conn)
            {
                Ok(_) => {
                    println!("stored uptime checks ({:?})", timer.elapsed());
                    let timer = Instant::now();

                    // delete checks older then:
                    let cutoff = get_epoch() - ((CHECKS_TO_STORE - 1) * CRON_INTERVAL);
                    match diesel::delete(checks)
                        .filter(updated.lt(diesel::dsl::sql(&format!(
                            "datetime({cutoff}, 'unixepoch')"
                        ))))
                        .execute(&conn)
                    {
                        Ok(_) => {
                            println!(
                                "cleaned up checks stored before {cutoff} ({:?})",
                                timer.elapsed()
                            );
                        }
                        Err(e) => {
                            println!(
                                "failed to cleanup checks stored before {cutoff}, with error: {e:?}"
                            );
                        }
                    }
                }
                Err(e) => {
                    println!("failed to store uptime checks with error: {e:?}");
                }
            }
        }
        Err(e) => {
            println!("failed retrieving instances from database with error: {e:?}");
        }
    }
}
