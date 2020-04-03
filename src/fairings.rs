use rocket::fairing::{Fairing, Info, Kind};
use rocket::Rocket;

pub struct Lock {
    pub keyhole: String
}

pub struct LockFairing;

impl Fairing for LockFairing {
    fn info(&self) -> Info {
        Info {
            name: "Stores the cron key configuration value in a managed state",
            kind: Kind::Attach
        }
    }

    fn on_attach(&self, rocket: Rocket) -> Result<Rocket, Rocket> {
        let lock = rocket.config()
            .get_str("cron_key")
            .unwrap()
            .to_string();
        Ok(
            rocket.manage(
                Lock {
                    keyhole: lock
                }
            )
        )
    }
}
